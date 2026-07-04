//! Window split-pane layout — tmux-style terminal splits.
//!
//! A [`SplitTree`] manages a binary tree of [`SplitNode`]s within a single tab.
//! Each leaf node is a `Pane` holding a [`PaneId`] that maps to a `TabSession`.
//! Interior nodes are `Horizontal` (left | right) or `Vertical` (top | bottom)
//! splits with a configurable ratio.
//!
//! # Example
//! ```
//! use ggterm_app::splits::{SplitTree, Rect};
//!
//! let mut tree = SplitTree::new(0);          // single pane, id=0
//! tree.split_horizontal(0.5);                 // split into [0 | 1]
//! tree.split_vertical(0.5);                   // split pane 1 into [1 / 2]
//!
//! let areas = tree.areas(Rect::new(0, 0, 100, 50));
//! assert_eq!(areas.len(), 3);                // three panes
//! ```

#[cfg(feature = "desktop")]
use crate::desktop_config::PANE_GAP;

/// Fallback constant when desktop feature is disabled.
#[cfg(not(feature = "desktop"))]
const PANE_GAP: f32 = 6.0;

/// Identifier for a terminal pane within a split tree.
pub type PaneId = usize;

/// A rectangular region in pixel (or cell) coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    /// X offset from the left edge.
    pub x: u32,
    /// Y offset from the top edge.
    pub y: u32,
    /// Width of the region.
    pub width: u32,
    /// Height of the region.
    pub height: u32,
}

impl Rect {
    /// Create a new rectangle.
    pub const fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Split this rect horizontally at the given ratio (0.0-1.0).
    ///
    /// Returns `(left, right)` where `left` gets `ratio * width`.
    /// A gutter of [`PANE_GAP`](crate::desktop_config::PANE_GAP) pixels is reserved
    /// between the two halves (was 1px before P26-G).
    pub fn split_h(self, ratio: f32) -> (Rect, Rect) {
        let gutter = if self.width > PANE_GAP as u32 {
            PANE_GAP as u32
        } else {
            0
        };
        let left_w = ((self.width as f32 * ratio) as u32).min(self.width.saturating_sub(gutter));
        let right_w = self.width.saturating_sub(left_w + gutter);
        let left = Rect::new(self.x, self.y, left_w, self.height);
        let right = Rect::new(self.x + left_w + gutter, self.y, right_w, self.height);
        (left, right)
    }

    /// Check whether a point is inside this rect.
    pub fn contains_point(&self, px: u32, py: u32) -> bool {
        px >= self.x && px < self.x + self.width && py >= self.y && py < self.y + self.height
    }

    /// Split this rect vertically at the given ratio (0.0-1.0).
    ///
    /// Returns `(top, bottom)` where `top` gets `ratio * height`.
    /// A gutter of [`PANE_GAP`](crate::desktop_config::PANE_GAP) pixels is reserved
    /// between the two halves (was 1px before P26-G).
    pub fn split_v(self, ratio: f32) -> (Rect, Rect) {
        let gutter = if self.height > PANE_GAP as u32 {
            PANE_GAP as u32
        } else {
            0
        };
        let top_h = ((self.height as f32 * ratio) as u32).min(self.height.saturating_sub(gutter));
        let bottom_h = self.height.saturating_sub(top_h + gutter);
        let top = Rect::new(self.x, self.y, self.width, top_h);
        let bottom = Rect::new(self.x, self.y + top_h + gutter, self.width, bottom_h);
        (top, bottom)
    }
}

/// A node in the split-pane binary tree.
#[derive(Debug, Clone, PartialEq)]
pub enum SplitNode {
    /// Leaf node — a single terminal pane.
    Pane(PaneId),
    /// Horizontal split — left | right.
    Horizontal {
        left: Box<SplitNode>,
        right: Box<SplitNode>,
        ratio: f32,
    },
    /// Vertical split — top / bottom.
    Vertical {
        top: Box<SplitNode>,
        bottom: Box<SplitNode>,
        ratio: f32,
    },
}

impl SplitNode {
    /// Create a single pane leaf node.
    pub fn pane(id: PaneId) -> Self {
        SplitNode::Pane(id)
    }

    /// Collect all pane IDs in left-to-right, top-to-bottom order.
    pub fn pane_ids(&self) -> Vec<PaneId> {
        match self {
            SplitNode::Pane(id) => vec![*id],
            SplitNode::Horizontal { left, right, .. } => {
                let mut ids = left.pane_ids();
                ids.extend(right.pane_ids());
                ids
            }
            SplitNode::Vertical { top, bottom, .. } => {
                let mut ids = top.pane_ids();
                ids.extend(bottom.pane_ids());
                ids
            }
        }
    }

    /// Count the number of leaf panes in this subtree.
    pub fn pane_count(&self) -> usize {
        match self {
            SplitNode::Pane(_) => 1,
            SplitNode::Horizontal { left, right, .. } => left.pane_count() + right.pane_count(),
            SplitNode::Vertical { top, bottom, .. } => top.pane_count() + bottom.pane_count(),
        }
    }

    /// Reset all split ratios in this subtree to 0.5 (even split).
    pub fn balance(&mut self) {
        match self {
            SplitNode::Pane(_) => {}
            SplitNode::Horizontal { left, right, ratio } => {
                *ratio = 0.5;
                left.balance();
                right.balance();
            }
            SplitNode::Vertical { top, bottom, ratio } => {
                *ratio = 0.5;
                top.balance();
                bottom.balance();
            }
        }
    }

    /// Check whether this subtree contains the given pane ID.
    pub fn contains(&self, id: PaneId) -> bool {
        match self {
            SplitNode::Pane(pid) => *pid == id,
            SplitNode::Horizontal { left, right, .. } => left.contains(id) || right.contains(id),
            SplitNode::Vertical { top, bottom, .. } => top.contains(id) || bottom.contains(id),
        }
    }

    /// Find a pane ID near the given position for click-to-focus.
    ///
    /// Traverses the tree using `bounds` to determine which pane the
    /// point `(px, py)` falls within.
    pub fn pane_at_point(&self, px: u32, py: u32, bounds: Rect) -> Option<PaneId> {
        match self {
            SplitNode::Pane(id) => {
                if px >= bounds.x
                    && px < bounds.x + bounds.width
                    && py >= bounds.y
                    && py < bounds.y + bounds.height
                {
                    Some(*id)
                } else {
                    None
                }
            }
            SplitNode::Horizontal { left, right, .. } => {
                let (lb, rb) = bounds.split_h(0.5);
                left.pane_at_point(px, py, lb)
                    .or_else(|| right.pane_at_point(px, py, rb))
            }
            SplitNode::Vertical { top, bottom, .. } => {
                let (tb, bb) = bounds.split_v(0.5);
                top.pane_at_point(px, py, tb)
                    .or_else(|| bottom.pane_at_point(px, py, bb))
            }
        }
    }

    /// Compute rectangular areas for every pane in this subtree.
    ///
    /// Returns a vec of `(PaneId, Rect)` pairs.
    pub fn areas(&self, bounds: Rect) -> Vec<(PaneId, Rect)> {
        match self {
            SplitNode::Pane(id) => vec![(*id, bounds)],
            SplitNode::Horizontal { left, right, ratio } => {
                let (lb, rb) = bounds.split_h(*ratio);
                let mut result = left.areas(lb);
                result.extend(right.areas(rb));
                result
            }
            SplitNode::Vertical { top, bottom, ratio } => {
                let (tb, bb) = bounds.split_v(*ratio);
                let mut result = top.areas(tb);
                result.extend(bottom.areas(bb));
                result
            }
        }
    }

    /// Insert a new pane by splitting an existing pane horizontally.
    ///
    /// The existing pane (`target_id`) becomes the left side, and
    /// `new_id` becomes the right side. Returns `true` if inserted.
    pub fn insert_horizontal(&mut self, target_id: PaneId, new_id: PaneId, ratio: f32) -> bool {
        match self {
            SplitNode::Pane(id) if *id == target_id => {
                let id = *id;
                let old = std::mem::replace(self, SplitNode::Pane(id));
                *self = SplitNode::Horizontal {
                    left: Box::new(old),
                    right: Box::new(SplitNode::Pane(new_id)),
                    ratio,
                };
                true
            }
            SplitNode::Horizontal { left, right, .. } => {
                left.insert_horizontal(target_id, new_id, ratio)
                    || right.insert_horizontal(target_id, new_id, ratio)
            }
            SplitNode::Vertical { top, bottom, .. } => {
                top.insert_horizontal(target_id, new_id, ratio)
                    || bottom.insert_horizontal(target_id, new_id, ratio)
            }
            _ => false,
        }
    }

    /// Insert a new pane by splitting an existing pane vertically.
    ///
    /// The existing pane (`target_id`) becomes the top, and `new_id`
    /// becomes the bottom. Returns `true` if inserted.
    pub fn insert_vertical(&mut self, target_id: PaneId, new_id: PaneId, ratio: f32) -> bool {
        match self {
            SplitNode::Pane(id) if *id == target_id => {
                let id = *id;
                let old = std::mem::replace(self, SplitNode::Pane(id));
                *self = SplitNode::Vertical {
                    top: Box::new(old),
                    bottom: Box::new(SplitNode::Pane(new_id)),
                    ratio,
                };
                true
            }
            SplitNode::Horizontal { left, right, .. } => {
                left.insert_vertical(target_id, new_id, ratio)
                    || right.insert_vertical(target_id, new_id, ratio)
            }
            SplitNode::Vertical { top, bottom, .. } => {
                top.insert_vertical(target_id, new_id, ratio)
                    || bottom.insert_vertical(target_id, new_id, ratio)
            }
            _ => false,
        }
    }

    /// Remove a pane and collapse the tree.
    ///
    /// When a pane is removed, its parent split node is replaced by
    /// the surviving sibling. Returns `Some(removed_id)` if found.
    pub fn remove(&mut self, target_id: PaneId) -> Option<PaneId> {
        match self {
            SplitNode::Pane(id) if *id == target_id => {
                // Can't remove the root leaf from within itself.
                // The caller (SplitTree) handles this case.
                None
            }
            SplitNode::Horizontal { left, right, .. } => {
                // Try left child.
                if let SplitNode::Pane(id) = left.as_ref()
                    && *id == target_id
                {
                    let _removed = left.pane_ids()[0];
                    *self = right.as_ref().clone();
                    return Some(_removed);
                }
                // Try right child.
                if let SplitNode::Pane(id) = right.as_ref()
                    && *id == target_id
                {
                    let _removed = right.pane_ids()[0];
                    *self = left.as_ref().clone();
                    return Some(_removed);
                }
                // Recurse.
                left.remove(target_id).or_else(|| right.remove(target_id))
            }
            SplitNode::Vertical { top, bottom, .. } => {
                if let SplitNode::Pane(id) = top.as_ref()
                    && *id == target_id
                {
                    let _removed = top.pane_ids()[0];
                    *self = bottom.as_ref().clone();
                    return Some(_removed);
                }
                if let SplitNode::Pane(id) = bottom.as_ref()
                    && *id == target_id
                {
                    let _removed = bottom.pane_ids()[0];
                    *self = top.as_ref().clone();
                    return Some(_removed);
                }
                top.remove(target_id).or_else(|| bottom.remove(target_id))
            }
            _ => None,
        }
    }

    /// P21-A: Check if a point is near a separator line.
    ///
    /// Returns `Some(true)` for horizontal separator (left|right),
    /// `Some(false)` for vertical separator (top/bottom),
    /// `None` if not near any separator.
    pub fn separator_at_point(&self, px: u32, py: u32, bounds: Rect) -> Option<bool> {
        const HIT: u32 = 4;

        match self {
            SplitNode::Pane(_) => None,
            SplitNode::Horizontal { left, right, ratio } => {
                let (lb, rb) = bounds.split_h(*ratio);
                let sep_x = lb.x + lb.width;

                if px >= sep_x.saturating_sub(HIT) && px <= sep_x + HIT {
                    return Some(true);
                }
                if lb.contains_point(px, py) {
                    return left.separator_at_point(px, py, lb);
                }
                if rb.contains_point(px, py) {
                    return right.separator_at_point(px, py, rb);
                }
                None
            }
            SplitNode::Vertical { top, bottom, ratio } => {
                let (tb, bb) = bounds.split_v(*ratio);
                let sep_y = tb.y + tb.height;

                if py >= sep_y.saturating_sub(HIT) && py <= sep_y + HIT {
                    return Some(false);
                }
                if tb.contains_point(px, py) {
                    return top.separator_at_point(px, py, tb);
                }
                if bb.contains_point(px, py) {
                    return bottom.separator_at_point(px, py, bb);
                }
                None
            }
        }
    }

    /// P21-A: Set the ratio of the separator nearest to the given point.
    ///
    /// Computes the new ratio from the absolute pixel position.
    /// Returns `true` if a separator was found and adjusted.
    pub fn set_ratio_at_point(&mut self, px: u32, py: u32, bounds: Rect) -> bool {
        const HIT: u32 = 4;

        match self {
            SplitNode::Pane(_) => false,
            SplitNode::Horizontal { left, right, ratio } => {
                let (lb, rb) = bounds.split_h(*ratio);
                let sep_x = lb.x + lb.width;

                if px >= sep_x.saturating_sub(HIT) && px <= sep_x + HIT {
                    if bounds.width > 0 {
                        *ratio = ((px - bounds.x) as f32 / bounds.width as f32).clamp(0.1, 0.9);
                    }
                    return true;
                }
                if lb.contains_point(px, py) {
                    return left.set_ratio_at_point(px, py, lb);
                }
                if rb.contains_point(px, py) {
                    return right.set_ratio_at_point(px, py, rb);
                }
                false
            }
            SplitNode::Vertical { top, bottom, ratio } => {
                let (tb, bb) = bounds.split_v(*ratio);
                let sep_y = tb.y + tb.height;

                if py >= sep_y.saturating_sub(HIT) && py <= sep_y + HIT {
                    if bounds.height > 0 {
                        *ratio = ((py - bounds.y) as f32 / bounds.height as f32).clamp(0.1, 0.9);
                    }
                    return true;
                }
                if tb.contains_point(px, py) {
                    return top.set_ratio_at_point(px, py, tb);
                }
                if bb.contains_point(px, py) {
                    return bottom.set_ratio_at_point(px, py, bb);
                }
                false
            }
        }
    }

    /// Adjust the split ratio of the nearest ancestor split containing `target_id`.
    ///
    /// `delta` is added to the ratio, clamped to [0.1, 0.9].
    /// Returns `true` if a ratio was adjusted.
    pub fn adjust_ratio(&mut self, target_id: PaneId, delta: f32) -> bool {
        match self {
            SplitNode::Pane(_) => false,
            SplitNode::Horizontal { left, right, ratio } => {
                if left.contains(target_id) || right.contains(target_id) {
                    // If the target is an immediate child, adjust this split's ratio.
                    let left_is_target =
                        matches!(left.as_ref(), SplitNode::Pane(id) if *id == target_id);
                    let right_is_target =
                        matches!(right.as_ref(), SplitNode::Pane(id) if *id == target_id);
                    if left_is_target || right_is_target {
                        *ratio = (*ratio + delta).clamp(0.1, 0.9);
                        return true;
                    }
                    // Otherwise recurse into the subtree that contains the target.
                    if left.contains(target_id) {
                        return left.adjust_ratio(target_id, delta);
                    }
                    return right.adjust_ratio(target_id, delta);
                }
                false
            }
            SplitNode::Vertical { top, bottom, ratio } => {
                if top.contains(target_id) || bottom.contains(target_id) {
                    let top_is_target =
                        matches!(top.as_ref(), SplitNode::Pane(id) if *id == target_id);
                    let bottom_is_target =
                        matches!(bottom.as_ref(), SplitNode::Pane(id) if *id == target_id);
                    if top_is_target || bottom_is_target {
                        *ratio = (*ratio + delta).clamp(0.1, 0.9);
                        return true;
                    }
                    if top.contains(target_id) {
                        return top.adjust_ratio(target_id, delta);
                    }
                    return bottom.adjust_ratio(target_id, delta);
                }
                false
            }
        }
    }
}

/// Manages a split-pane tree with focus tracking for a single tab.
#[derive(Debug, Clone)]
pub struct SplitTree {
    /// Root of the binary tree.
    root: SplitNode,
    /// Currently focused pane ID.
    active: PaneId,
    /// Next available pane ID for allocation.
    next_id: PaneId,
}

impl SplitTree {
    /// Create a new split tree with a single pane (id = 0).
    pub fn new(initial_id: PaneId) -> Self {
        Self {
            root: SplitNode::Pane(initial_id),
            active: initial_id,
            next_id: initial_id + 1,
        }
    }

    /// Reconstruct a split tree from saved state (session restore).
    ///
    /// Takes a root node, the active pane ID, and computes `next_id`
    /// from the maximum pane ID in the tree.
    pub fn from_parts(root: SplitNode, active: PaneId) -> Self {
        let max_id = root.pane_ids().into_iter().max().unwrap_or(0);
        Self {
            root,
            active,
            next_id: max_id + 1,
        }
    }

    /// Get the root node.
    pub fn root(&self) -> &SplitNode {
        &self.root
    }

    /// Get the active (focused) pane ID.
    pub fn active(&self) -> PaneId {
        self.active
    }

    /// Set the active pane.
    pub fn set_active(&mut self, id: PaneId) {
        if self.root.contains(id) {
            self.active = id;
        }
    }

    /// Allocate the next pane ID.
    pub fn alloc_id(&mut self) -> PaneId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Number of panes in the tree.
    pub fn pane_count(&self) -> usize {
        self.root.pane_count()
    }

    /// Reset all split ratios to 0.5 (even spacing).
    pub fn balance(&mut self) {
        self.root.balance();
    }

    /// Collect all pane IDs in visual order (left→right, top→bottom).
    pub fn pane_ids(&self) -> Vec<PaneId> {
        self.root.pane_ids()
    }

    /// Split the active pane horizontally (left | right).
    ///
    /// The new pane becomes the active pane. Returns the new pane ID.
    pub fn split_horizontal(&mut self, ratio: f32) -> PaneId {
        let new_id = self.alloc_id();
        let target = self.active;
        if self.root.insert_horizontal(target, new_id, ratio) {
            self.active = new_id;
        }
        new_id
    }

    /// Split the active pane vertically (top / bottom).
    ///
    /// The new pane becomes the active pane. Returns the new pane ID.
    pub fn split_vertical(&mut self, ratio: f32) -> PaneId {
        let new_id = self.alloc_id();
        let target = self.active;
        if self.root.insert_vertical(target, new_id, ratio) {
            self.active = new_id;
        }
        new_id
    }

    /// Split a specific pane horizontally. Returns the new pane ID.
    pub fn split_horizontal_pane(&mut self, target: PaneId, ratio: f32) -> PaneId {
        let new_id = self.alloc_id();
        if self.root.insert_horizontal(target, new_id, ratio) {
            self.active = new_id;
        }
        new_id
    }

    /// Split a specific pane vertically. Returns the new pane ID.
    pub fn split_vertical_pane(&mut self, target: PaneId, ratio: f32) -> PaneId {
        let new_id = self.alloc_id();
        if self.root.insert_vertical(target, new_id, ratio) {
            self.active = new_id;
        }
        new_id
    }

    /// Remove a pane and collapse the tree.
    ///
    /// If the removed pane was active, focus moves to the next pane.
    /// Returns `true` if the pane was removed.
    pub fn remove(&mut self, id: PaneId) -> bool {
        // Can't remove the only pane.
        if self.pane_count() <= 1 {
            return false;
        }

        // Get pane list before removal for focus calculation.
        let ids = self.pane_ids();
        let removed = self.root.remove(id);

        if removed.is_some() {
            // Move focus if the active pane was removed.
            if self.active == id {
                // Find the pane that was before the removed one.
                if let Some(idx) = ids.iter().position(|&p| p == id) {
                    // Try the pane before, otherwise the one after.
                    let new_active = if idx > 0 {
                        ids[idx - 1]
                    } else {
                        ids.get(1).copied().unwrap_or(0)
                    };
                    self.active = new_active;
                }
            }
            true
        } else {
            false
        }
    }

    /// Cycle focus to the next pane (wraps around).
    pub fn focus_next(&mut self) {
        let ids = self.pane_ids();
        if ids.len() <= 1 {
            return;
        }
        if let Some(idx) = ids.iter().position(|&id| id == self.active) {
            let next_idx = (idx + 1) % ids.len();
            self.active = ids[next_idx];
        }
    }

    /// Cycle focus to the previous pane (wraps around).
    pub fn focus_prev(&mut self) {
        let ids = self.pane_ids();
        if ids.len() <= 1 {
            return;
        }
        if let Some(idx) = ids.iter().position(|&id| id == self.active) {
            let prev_idx = if idx == 0 { ids.len() - 1 } else { idx - 1 };
            self.active = ids[prev_idx];
        }
    }

    /// Compute areas for all panes given the total bounds.
    pub fn areas(&self, bounds: Rect) -> Vec<(PaneId, Rect)> {
        self.root.areas(bounds)
    }

    /// Get the rectangular area for a specific pane.
    pub fn area_for(&self, id: PaneId, bounds: Rect) -> Option<Rect> {
        self.areas(bounds)
            .into_iter()
            .find(|(pid, _)| *pid == id)
            .map(|(_, r)| r)
    }

    /// Find which pane contains the given point.
    pub fn pane_at_point(&self, px: u32, py: u32, bounds: Rect) -> Option<PaneId> {
        self.root.pane_at_point(px, py, bounds)
    }

    /// Adjust the split ratio for the active pane.
    ///
    /// `delta` is added to the ratio of the nearest enclosing split,
    /// clamped to [0.1, 0.9]. Positive delta grows the left/top side.
    pub fn adjust_active_ratio(&mut self, delta: f32) -> bool {
        self.root.adjust_ratio(self.active, delta)
    }

    /// Adjust the split ratio for a specific pane.
    pub fn adjust_ratio(&mut self, id: PaneId, delta: f32) -> bool {
        self.root.adjust_ratio(id, delta)
    }

    /// Check whether the tree has only a single pane (no splits).
    pub fn is_single(&self) -> bool {
        self.pane_count() == 1
    }

    /// P21-A: Check if a point is near a separator line for cursor styling.
    ///
    /// Returns `Some(true)` for horizontal separator (left|right),
    /// `Some(false)` for vertical separator (top/bottom),
    /// `None` if not near any separator.
    pub fn separator_at_point(&self, px: u32, py: u32, bounds: Rect) -> Option<bool> {
        self.root.separator_at_point(px, py, bounds)
    }

    /// P21-A: Set the ratio of the separator nearest to the given point.
    ///
    /// Returns `true` if a separator was found and adjusted.
    pub fn set_ratio_at_point(&mut self, px: u32, py: u32, bounds: Rect) -> bool {
        self.root.set_ratio_at_point(px, py, bounds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Rect tests ──────────────────────────────────────────────

    #[test]
    fn t_rect_split_h_even() {
        let r = Rect::new(0, 0, 100, 50);
        let (left, right) = r.split_h(0.5);
        assert_eq!(left.x, 0);
        assert!(left.width >= 44 && left.width <= 50);
        assert_eq!(left.height, 50);
        // Right starts after left + PANE_GAP(6px) gutter
        assert_eq!(right.x, left.x + left.width + PANE_GAP as u32);
        assert!(right.width >= 44 && right.width <= 50);
        // Total width should be preserved
        assert_eq!(left.width + right.width + PANE_GAP as u32, 100);
    }

    #[test]
    fn t_rect_split_h_ratio() {
        let r = Rect::new(0, 0, 100, 50);
        let (left, right) = r.split_h(0.3);
        assert_eq!(left.width, 30);
        // Right gets remaining width minus PANE_GAP gutter
        assert!(right.width >= 64); // 100 - 30 - 6 = 64
    }

    #[test]
    fn t_rect_split_v() {
        let r = Rect::new(10, 20, 80, 60);
        let (top, bottom) = r.split_v(0.5);
        assert_eq!(top.x, 10);
        assert_eq!(top.y, 20);
        assert!(top.height >= 24 && top.height <= 30);
        assert!(bottom.height >= 24 && bottom.height <= 30);
        // Bottom starts after top + PANE_GAP(6px) gutter
        assert_eq!(bottom.y, top.y + top.height + PANE_GAP as u32);
        // Total height should be preserved
        assert_eq!(top.height + bottom.height + PANE_GAP as u32, 60);
    }

    #[test]
    fn t_rect_split_h_small() {
        let r = Rect::new(0, 0, 2, 10);
        let (left, right) = r.split_h(0.5);
        // Width=2, gutter=0 (width not > PANE_GAP) → left=1, right=1
        assert_eq!(left.width, 1);
        assert_eq!(right.width, 1);
    }

    #[test]
    fn t_rect_split_h_zero_ratio() {
        let r = Rect::new(0, 0, 100, 50);
        let (left, _right) = r.split_h(0.0);
        assert_eq!(left.width, 0);
    }

    #[test]
    fn t_rect_split_h_full_ratio() {
        let r = Rect::new(0, 0, 100, 50);
        let (left, _right) = r.split_h(1.0);
        // Full width minus PANE_GAP gutter
        assert!(left.width >= 94); // 100 - 6 = 94
    }

    // ── SplitNode basic tests ───────────────────────────────────

    #[test]
    fn t_node_pane_ids_single() {
        let n = SplitNode::pane(0);
        assert_eq!(n.pane_ids(), vec![0]);
    }

    #[test]
    fn t_node_pane_count_single() {
        let n = SplitNode::pane(0);
        assert_eq!(n.pane_count(), 1);
    }

    #[test]
    fn t_node_contains() {
        let n = SplitNode::pane(0);
        assert!(n.contains(0));
        assert!(!n.contains(1));
    }

    // ── SplitTree single pane ───────────────────────────────────

    #[test]
    fn t_tree_new_single_pane() {
        let tree = SplitTree::new(0);
        assert_eq!(tree.pane_count(), 1);
        assert_eq!(tree.active(), 0);
        assert!(tree.is_single());
        assert_eq!(tree.pane_ids(), vec![0]);
    }

    #[test]
    fn t_tree_areas_single() {
        let tree = SplitTree::new(0);
        let areas = tree.areas(Rect::new(0, 0, 80, 24));
        assert_eq!(areas.len(), 1);
        assert_eq!(areas[0], (0, Rect::new(0, 0, 80, 24)));
    }

    // ── SplitTree horizontal split ──────────────────────────────

    #[test]
    fn t_tree_split_horizontal() {
        let mut tree = SplitTree::new(0);
        tree.split_horizontal(0.5);
        assert_eq!(tree.pane_count(), 2);
        assert!(!tree.is_single());
        assert_eq!(tree.pane_ids(), vec![0, 1]);
        assert_eq!(tree.active(), 1); // new pane is active
    }

    #[test]
    fn t_tree_split_horizontal_areas() {
        let mut tree = SplitTree::new(0);
        tree.split_horizontal(0.5);
        let areas = tree.areas(Rect::new(0, 0, 100, 50));
        assert_eq!(areas.len(), 2);

        // Pane 0 is on the left, pane 1 is on the right.
        let (id0, r0) = areas[0];
        let (id1, r1) = areas[1];
        assert_eq!(id0, 0);
        assert_eq!(id1, 1);
        assert!(r0.width < 100); // not full width
        assert!(r1.width < 100);
        // Heights should match the original.
        assert_eq!(r0.height, 50);
        assert_eq!(r1.height, 50);
    }

    #[test]
    fn t_tree_split_horizontal_ratio_30_70() {
        let mut tree = SplitTree::new(0);
        tree.split_horizontal(0.3);
        let areas = tree.areas(Rect::new(0, 0, 100, 50));
        let (_, r0) = areas[0];
        let (_, r1) = areas[1];
        assert_eq!(r0.width, 30);
        // Right gets remaining width minus PANE_GAP gutter
        assert!(r1.width >= 64); // 100 - 30 - 6 = 64
    }

    // ── SplitTree vertical split ────────────────────────────────

    #[test]
    fn t_tree_split_vertical() {
        let mut tree = SplitTree::new(0);
        tree.split_vertical(0.5);
        assert_eq!(tree.pane_count(), 2);
        assert_eq!(tree.pane_ids(), vec![0, 1]);
    }

    #[test]
    fn t_tree_split_vertical_areas() {
        let mut tree = SplitTree::new(0);
        tree.split_vertical(0.5);
        let areas = tree.areas(Rect::new(0, 0, 100, 50));
        assert_eq!(areas.len(), 2);
        let (_, r0) = areas[0];
        let (_, r1) = areas[1];
        assert_eq!(r0.width, 100); // full width
        assert!(r0.height < 50); // half height
        assert!(r1.height < 50);
    }

    // ── Nested splits ───────────────────────────────────────────

    #[test]
    fn t_tree_nested_h_then_v() {
        // Start with pane 0.
        // Split horizontally → [0 | 1]
        // Split pane 1 vertically → [0 | [1 / 2]]
        let mut tree = SplitTree::new(0);
        tree.split_horizontal(0.5); // pane 0 | pane 1
        assert_eq!(tree.active(), 1);
        tree.split_vertical(0.5); // split pane 1 into [1 / 2]
        assert_eq!(tree.pane_count(), 3);
        assert_eq!(tree.pane_ids(), vec![0, 1, 2]);
    }

    #[test]
    fn t_tree_nested_areas() {
        let mut tree = SplitTree::new(0);
        tree.split_horizontal(0.5); // [0 | 1]
        tree.split_vertical(0.5); // split pane 1 → [0 | [2/1]]

        let areas = tree.areas(Rect::new(0, 0, 100, 50));
        assert_eq!(areas.len(), 3);

        // Pane 0: left half, full height
        let r0 = areas.iter().find(|(id, _)| id == &0).unwrap().1;
        assert!(r0.width < 100);
        assert_eq!(r0.height, 50);

        // Pane 2: top-right quarter
        let r2 = areas.iter().find(|(id, _)| id == &2).unwrap().1;
        assert!(r2.height < 50);
        assert!(r2.x > r0.x); // to the right of pane 0
    }

    #[test]
    fn t_tree_three_way_split() {
        // [0] → split H → [0 | 1] → split H on 1 → [0 | [1 | 2]]
        let mut tree = SplitTree::new(0);
        tree.split_horizontal(0.5);
        tree.set_active(1);
        tree.split_horizontal(0.5);
        assert_eq!(tree.pane_count(), 3);
        assert_eq!(tree.pane_ids(), vec![0, 1, 2]);
    }

    // ── Remove tests ────────────────────────────────────────────

    #[test]
    fn t_tree_remove_pane() {
        let mut tree = SplitTree::new(0);
        tree.split_horizontal(0.5);
        // [0 | 1], remove pane 1
        assert!(tree.remove(1));
        assert_eq!(tree.pane_count(), 1);
        assert_eq!(tree.pane_ids(), vec![0]);
        assert!(tree.is_single());
    }

    #[test]
    fn t_tree_remove_collapses_parent() {
        // [0 | [2 / 1]] → remove pane 2 → [0 | 1]
        let mut tree = SplitTree::new(0);
        tree.split_horizontal(0.5); // [0 | 1]
        tree.split_vertical(0.5); // [0 | [2 / 1]]

        assert!(tree.remove(2));
        assert_eq!(tree.pane_count(), 2);
        assert_eq!(tree.pane_ids(), vec![0, 1]);
    }

    #[test]
    fn t_tree_remove_last_collapses_to_single() {
        let mut tree = SplitTree::new(0);
        tree.split_horizontal(0.5);
        // Remove pane 0 → should collapse to pane 1
        assert!(tree.remove(0));
        assert_eq!(tree.pane_count(), 1);
        assert_eq!(tree.pane_ids(), vec![1]);
        assert_eq!(tree.active(), 1);
    }

    #[test]
    fn t_tree_remove_single_pane_fails() {
        let mut tree = SplitTree::new(0);
        assert!(!tree.remove(0)); // can't remove the only pane
        assert_eq!(tree.pane_count(), 1);
    }

    #[test]
    fn t_tree_remove_nonexistent_fails() {
        let mut tree = SplitTree::new(0);
        tree.split_horizontal(0.5);
        assert!(!tree.remove(99)); // pane 99 doesn't exist
    }

    #[test]
    fn t_tree_remove_active_moves_focus() {
        let mut tree = SplitTree::new(0);
        tree.split_horizontal(0.5); // active = 1
        tree.set_active(0); // focus pane 0
        assert_eq!(tree.active(), 0);
        tree.remove(0); // remove active pane → focus should move
        assert_eq!(tree.active(), 1);
    }

    // ── Focus cycling tests ─────────────────────────────────────

    #[test]
    fn t_focus_next_wraps() {
        let mut tree = SplitTree::new(0);
        tree.split_horizontal(0.5); // [0, 1], active=1
        tree.split_horizontal(0.5); // [0, 1, 2], active=2

        tree.set_active(0);
        tree.focus_next();
        assert_eq!(tree.active(), 1);
        tree.focus_next();
        assert_eq!(tree.active(), 2);
        tree.focus_next();
        assert_eq!(tree.active(), 0); // wraps
    }

    #[test]
    fn t_focus_prev_wraps() {
        let mut tree = SplitTree::new(0);
        tree.split_horizontal(0.5);
        tree.split_horizontal(0.5); // [0, 1, 2]

        tree.set_active(0);
        tree.focus_prev();
        assert_eq!(tree.active(), 2); // wraps to last
        tree.focus_prev();
        assert_eq!(tree.active(), 1);
    }

    #[test]
    fn t_focus_next_single_pane_noop() {
        let mut tree = SplitTree::new(0);
        tree.focus_next();
        assert_eq!(tree.active(), 0);
        tree.focus_prev();
        assert_eq!(tree.active(), 0);
    }

    // ── Ratio adjustment tests ──────────────────────────────────

    #[test]
    fn t_adjust_ratio_grow() {
        let mut tree = SplitTree::new(0);
        tree.split_horizontal(0.5); // [0 | 1]
        tree.set_active(0);
        assert!(tree.adjust_active_ratio(0.1)); // grow left
        let areas = tree.areas(Rect::new(0, 0, 100, 50));
        let r0 = areas.iter().find(|(id, _)| id == &0).unwrap().1;
        let r1 = areas.iter().find(|(id, _)| id == &1).unwrap().1;
        assert!(r0.width > r1.width); // left is now bigger
    }

    #[test]
    fn t_adjust_ratio_shrink() {
        let mut tree = SplitTree::new(0);
        tree.split_horizontal(0.5);
        tree.set_active(0);
        assert!(tree.adjust_active_ratio(-0.1)); // shrink left
        let areas = tree.areas(Rect::new(0, 0, 100, 50));
        let r0 = areas.iter().find(|(id, _)| id == &0).unwrap().1;
        assert!(r0.width < 50);
    }

    #[test]
    fn t_adjust_ratio_clamp() {
        let mut tree = SplitTree::new(0);
        tree.split_horizontal(0.5);
        tree.set_active(0);
        // Push way beyond the clamp.
        tree.adjust_active_ratio(10.0);
        let areas = tree.areas(Rect::new(0, 0, 100, 50));
        let r0 = areas.iter().find(|(id, _)| id == &0).unwrap().1;
        // Ratio clamped to 0.9 → width ~90
        assert!(r0.width >= 89);
    }

    #[test]
    fn t_adjust_ratio_single_pane_noop() {
        let mut tree = SplitTree::new(0);
        assert!(!tree.adjust_active_ratio(0.1));
    }

    // ── pane_at_point tests ─────────────────────────────────────

    #[test]
    fn t_pane_at_point_single() {
        let tree = SplitTree::new(0);
        assert_eq!(
            tree.pane_at_point(10, 10, Rect::new(0, 0, 100, 50)),
            Some(0)
        );
        assert_eq!(tree.pane_at_point(200, 200, Rect::new(0, 0, 100, 50)), None);
    }

    #[test]
    fn t_pane_at_point_split_h() {
        let mut tree = SplitTree::new(0);
        tree.split_horizontal(0.5);
        // Click left side → pane 0
        assert_eq!(
            tree.pane_at_point(10, 10, Rect::new(0, 0, 100, 50)),
            Some(0)
        );
        // Click right side → pane 1
        assert_eq!(
            tree.pane_at_point(80, 10, Rect::new(0, 0, 100, 50)),
            Some(1)
        );
    }

    #[test]
    fn t_pane_at_point_split_v() {
        let mut tree = SplitTree::new(0);
        tree.split_vertical(0.5);
        // Click top → pane 0
        assert_eq!(tree.pane_at_point(10, 5, Rect::new(0, 0, 100, 50)), Some(0));
        // Click bottom → pane 1
        assert_eq!(
            tree.pane_at_point(10, 40, Rect::new(0, 0, 100, 50)),
            Some(1)
        );
    }

    // ── area_for tests ──────────────────────────────────────────

    #[test]
    fn t_area_for_existing() {
        let mut tree = SplitTree::new(0);
        tree.split_horizontal(0.5);
        let r = tree.area_for(1, Rect::new(0, 0, 100, 50));
        assert!(r.is_some());
        let r = r.unwrap();
        assert!(r.x > 0); // right side
    }

    #[test]
    fn t_area_for_nonexistent() {
        let tree = SplitTree::new(0);
        assert!(tree.area_for(99, Rect::new(0, 0, 100, 50)).is_none());
    }

    // ── Complex tree tests ──────────────────────────────────────

    #[test]
    fn t_complex_tree_4_panes() {
        // Build: [0 | [1 / [2 | 3]]]
        let mut tree = SplitTree::new(0);
        tree.split_horizontal(0.5); // [0 | 1]
        tree.split_vertical(0.5); // [0 | [2 / 1]]
        tree.set_active(1);
        tree.split_horizontal(0.5); // [0 | [2 / [1 | 3]]]

        assert_eq!(tree.pane_count(), 4);
        assert!(tree.pane_ids().contains(&3));

        // All 4 panes get distinct areas.
        let areas = tree.areas(Rect::new(0, 0, 200, 100));
        assert_eq!(areas.len(), 4);
        let unique: std::collections::HashSet<_> = areas.iter().map(|(id, _)| id).collect();
        assert_eq!(unique.len(), 4);
    }

    #[test]
    fn t_complex_tree_remove_all_except_one() {
        let mut tree = SplitTree::new(0);
        tree.split_horizontal(0.5);
        tree.split_vertical(0.5);
        tree.split_horizontal(0.5);

        // Remove panes one by one.
        while tree.pane_count() > 1 {
            let ids = tree.pane_ids();
            let last = *ids.last().unwrap();
            assert!(tree.remove(last));
        }
        assert_eq!(tree.pane_count(), 1);
        assert!(tree.is_single());
    }

    #[test]
    fn t_tree_alloc_id_increments() {
        let mut tree = SplitTree::new(5);
        assert_eq!(tree.alloc_id(), 6);
        assert_eq!(tree.alloc_id(), 7);
        assert_eq!(tree.alloc_id(), 8);
    }

    #[test]
    fn t_tree_set_active_validates() {
        let mut tree = SplitTree::new(0);
        tree.split_horizontal(0.5); // panes [0, 1]
        tree.set_active(1);
        assert_eq!(tree.active(), 1);
        tree.set_active(99); // invalid — should not change
        assert_eq!(tree.active(), 1);
    }

    #[test]
    fn t_tree_split_specific_pane() {
        let mut tree = SplitTree::new(0);
        tree.split_horizontal(0.5); // [0 | 1]
        // Split pane 0 specifically (not active pane).
        // Existing pane 0 becomes top, new pane 2 becomes bottom.
        tree.split_vertical_pane(0, 0.5); // [[0 / 2] | 1]
        assert_eq!(tree.pane_count(), 3);
        assert_eq!(tree.pane_ids(), vec![0, 2, 1]);
    }

    #[test]
    fn t_node_clone_and_eq() {
        let n1 = SplitNode::pane(0);
        let n2 = n1.clone();
        assert_eq!(n1, n2);
    }

    #[test]
    fn t_deep_nested_tree_pane_count() {
        // Chain 5 horizontal splits.
        let mut tree = SplitTree::new(0);
        for _ in 0..5 {
            tree.split_horizontal(0.5);
        }
        assert_eq!(tree.pane_count(), 6);
        assert_eq!(tree.pane_ids().len(), 6);
    }

    // ── P19-H: Integration edge cases ─────────────────────────

    #[test]
    fn t_split_remove_split_cycle() {
        // Split → remove → split again: new IDs must not reuse old ones.
        let mut tree = SplitTree::new(0);
        let id1 = tree.split_horizontal(0.5);
        assert_eq!(id1, 1);
        assert!(tree.remove(1));
        // Tree is back to single pane, but next_id is still 2.
        let id2 = tree.split_horizontal(0.5);
        assert_eq!(id2, 2, "new pane should not reuse removed ID");
        assert_eq!(tree.pane_count(), 2);
    }

    #[test]
    fn t_remove_from_deep_nested_middle() {
        // Build [[0 / 2] | [1 | 3]]
        let mut tree = SplitTree::new(0);
        tree.split_horizontal(0.5); // [0 | 1]
        tree.split_vertical_pane(0, 0.5); // [[0 / 2] | 1]
        tree.split_horizontal_pane(1, 0.5); // [[0 / 2] | [1 | 3]]
        assert_eq!(tree.pane_count(), 4);

        // Remove pane 2 (deep nested in left subtree).
        assert!(tree.remove(2));
        assert_eq!(tree.pane_count(), 3);
        // Pane 0 should survive (collapsed from vertical split).
        assert!(tree.pane_ids().contains(&0));
        // Pane 1 and 3 should survive.
        assert!(tree.pane_ids().contains(&1));
        assert!(tree.pane_ids().contains(&3));
    }

    #[test]
    fn t_focus_cycle_visits_all_panes() {
        // In a 4-pane tree, focus_next should visit all 4 before wrapping.
        let mut tree = SplitTree::new(0);
        tree.split_horizontal(0.5); // [0 | 1]
        tree.split_horizontal_pane(0, 0.5); // [[0 | 2] | 1]
        tree.split_horizontal_pane(1, 0.5); // [[0 | 2] | [1 | 3]]

        tree.set_active(0);
        let mut visited = vec![tree.active()];
        for _ in 0..3 {
            tree.focus_next();
            visited.push(tree.active());
        }
        // All 4 panes should be visited.
        let unique: std::collections::HashSet<_> = visited.iter().collect();
        assert_eq!(unique.len(), 4, "focus_next must visit all panes");
        // One more should wrap back to start.
        tree.focus_next();
        assert_eq!(tree.active(), 0);
    }

    #[test]
    fn t_areas_all_nonzero_in_complex_tree() {
        let mut tree = SplitTree::new(0);
        tree.split_horizontal(0.5);
        tree.split_vertical(0.5);

        let areas = tree.areas(Rect::new(0, 0, 200, 100));
        for (id, r) in &areas {
            assert!(r.width > 0, "pane {id} has zero width");
            assert!(r.height > 0, "pane {id} has zero height");
        }
    }

    #[test]
    fn t_root_accessor() {
        let tree = SplitTree::new(0);
        assert!(matches!(tree.root(), SplitNode::Pane(0)));
    }

    #[test]
    fn t_adjust_ratio_for_specific_pane() {
        let mut tree = SplitTree::new(0);
        tree.split_horizontal(0.5); // [0 | 1]
        // Adjust ratio for pane 1 (not active).
        assert!(tree.adjust_ratio(0, 0.2));
        let areas = tree.areas(Rect::new(0, 0, 100, 50));
        let r0 = areas.iter().find(|(id, _)| id == &0).unwrap().1;
        // Left side grew by +0.2 (from 0.5 to 0.7).
        assert!(r0.width > 60);
    }

    // ── P26-G: PANE_GAP gutter tests ─────────────────────────────

    #[test]
    fn t_pane_gap_h_split_preserves_total() {
        let r = Rect::new(0, 0, 200, 100);
        let (left, right) = r.split_h(0.4);
        // left + gap + right == total width
        assert_eq!(left.width + PANE_GAP as u32 + right.width, 200);
        assert_eq!(right.x, left.x + left.width + PANE_GAP as u32);
    }

    #[test]
    fn t_pane_gap_v_split_preserves_total() {
        let r = Rect::new(0, 0, 200, 100);
        let (top, bottom) = r.split_v(0.6);
        // top + gap + bottom == total height
        assert_eq!(top.height + PANE_GAP as u32 + bottom.height, 100);
        assert_eq!(bottom.y, top.y + top.height + PANE_GAP as u32);
    }

    #[test]
    fn t_pane_gap_no_gutter_when_too_small() {
        // Width < PANE_GAP → gutter = 0
        let r = Rect::new(0, 0, 3, 10);
        let (left, right) = r.split_h(0.5);
        // 3px total, no gutter → left=1, right=2 (floor(1.5)=1, rest=2)
        assert_eq!(left.width + right.width, 3);
    }

    #[test]
    fn t_balance_resets_ratios() {
        let mut tree = SplitTree::new(0);
        tree.split_horizontal(0.7); // Uneven split.
        tree.split_vertical(0.3); // Another uneven split.
        tree.balance();
        // All splits should now be 0.5.
        let areas = tree.areas(Rect::new(0, 0, 200, 100));
        assert_eq!(areas.len(), 3);
    }

    #[test]
    fn t_balance_single_pane_noop() {
        let mut tree = SplitTree::new(0);
        tree.balance(); // Should not panic.
        let areas = tree.areas(Rect::new(0, 0, 100, 50));
        assert_eq!(areas.len(), 1);
    }
}
