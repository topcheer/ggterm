//! Damage tracking for efficient partial rendering.
//!
//! The [`DamageTracker`] accumulates dirty regions and merges them into a
//! single bounding rectangle for compact representation.

/// A rectangular dirty region (bounding box).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DirtyRect {
    /// Left column (inclusive).
    pub x: usize,
    /// Top row (inclusive).
    pub y: usize,
    /// Width in columns.
    pub width: usize,
    /// Height in rows.
    pub height: usize,
}

impl DirtyRect {
    /// Create a new dirty rectangle.
    pub fn new(x: usize, y: usize, width: usize, height: usize) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Right edge (exclusive).
    pub fn right(&self) -> usize {
        self.x + self.width
    }

    /// Bottom edge (exclusive).
    pub fn bottom(&self) -> usize {
        self.y + self.height
    }

    /// Compute the union (bounding box) of two rectangles.
    pub fn union(&self, other: &Self) -> Self {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let right = self.right().max(other.right());
        let bottom = self.bottom().max(other.bottom());
        Self {
            x,
            y,
            width: right - x,
            height: bottom - y,
        }
    }
}

/// Tracks dirty (modified) regions of the terminal grid.
///
/// Uses a single merged bounding rectangle.
#[derive(Debug, Clone)]
pub struct DamageTracker {
    dirty: Option<DirtyRect>,
    width: usize,
}

impl DamageTracker {
    /// Create a tracker for a grid of the given width.
    pub fn new(width: usize) -> Self {
        Self { dirty: None, width }
    }

    /// Returns true if any region is dirty.
    pub fn is_dirty(&self) -> bool {
        self.dirty.is_some()
    }

    /// Current dirty rect without clearing.
    pub fn dirty(&self) -> Option<DirtyRect> {
        self.dirty
    }

    /// Mark a single cell dirty.
    pub fn mark_cell(&mut self, x: usize, y: usize) {
        self.mark_rect(x, y, 1, 1);
    }

    /// Mark an entire row dirty (full grid width).
    pub fn mark_row(&mut self, y: usize) {
        let w = if self.width > 0 { self.width } else { 1 };
        self.mark_rect(0, y, w, 1);
    }

    /// Mark height rows starting at start_y dirty.
    pub fn mark_rows(&mut self, start_y: usize, height: usize) {
        let w = if self.width > 0 { self.width } else { 1 };
        self.mark_rect(0, start_y, w, height);
    }

    /// Mark a rectangular region dirty, merging with existing.
    pub fn mark_rect(&mut self, x: usize, y: usize, width: usize, height: usize) {
        if width == 0 || height == 0 {
            return;
        }
        let nr = DirtyRect::new(x, y, width, height);
        self.dirty = Some(match self.dirty {
            Some(existing) => existing.union(&nr),
            None => nr,
        });
    }

    /// Mark the entire grid dirty.
    pub fn mark_all(&mut self, height: usize) {
        let w = if self.width > 0 { self.width } else { 1 };
        self.dirty = Some(DirtyRect::new(0, 0, w, height));
    }

    /// Take ownership of the dirty region, clearing the tracker.
    pub fn take_dirty(&mut self) -> Option<DirtyRect> {
        self.dirty.take()
    }

    /// Clear all dirty marks.
    pub fn clear(&mut self) {
        self.dirty = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirty_rect_union() {
        let a = DirtyRect::new(0, 0, 5, 5);
        let b = DirtyRect::new(3, 3, 5, 5);
        let u = a.union(&b);
        assert_eq!(u.x, 0);
        assert_eq!(u.y, 0);
        assert_eq!(u.width, 8);
        assert_eq!(u.height, 8);
    }

    #[test]
    fn tracker_empty() {
        let d = DamageTracker::new(80);
        assert!(!d.is_dirty());
    }

    #[test]
    fn tracker_mark_cell() {
        let mut d = DamageTracker::new(80);
        d.mark_cell(10, 5);
        assert_eq!(d.dirty(), Some(DirtyRect::new(10, 5, 1, 1)));
    }

    #[test]
    fn tracker_mark_row() {
        let mut d = DamageTracker::new(80);
        d.mark_row(3);
        assert_eq!(d.dirty(), Some(DirtyRect::new(0, 3, 80, 1)));
    }

    #[test]
    fn tracker_mark_rows() {
        let mut d = DamageTracker::new(80);
        d.mark_rows(10, 3);
        assert_eq!(d.dirty(), Some(DirtyRect::new(0, 10, 80, 3)));
    }

    #[test]
    fn tracker_merge() {
        let mut d = DamageTracker::new(80);
        d.mark_cell(10, 5);
        d.mark_cell(20, 10);
        let dirty = d.dirty().unwrap();
        assert_eq!(dirty.x, 10);
        assert_eq!(dirty.y, 5);
        assert_eq!(dirty.right(), 21);
        assert_eq!(dirty.bottom(), 11);
    }

    #[test]
    fn tracker_take_clears() {
        let mut d = DamageTracker::new(80);
        d.mark_cell(0, 0);
        let taken = d.take_dirty();
        assert_eq!(taken, Some(DirtyRect::new(0, 0, 1, 1)));
        assert!(!d.is_dirty());
    }

    #[test]
    fn tracker_clear() {
        let mut d = DamageTracker::new(80);
        d.mark_cell(0, 0);
        d.clear();
        assert!(!d.is_dirty());
    }

    #[test]
    fn tracker_mark_all() {
        let mut d = DamageTracker::new(80);
        d.mark_all(24);
        assert_eq!(d.dirty(), Some(DirtyRect::new(0, 0, 80, 24)));
    }

    #[test]
    fn tracker_zero_size_ignored() {
        let mut d = DamageTracker::new(80);
        d.mark_rect(0, 0, 0, 10);
        d.mark_rect(0, 0, 10, 0);
        assert!(!d.is_dirty());
    }
}
