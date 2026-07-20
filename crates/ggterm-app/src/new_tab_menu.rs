//! Dropdown menu for the "+" button in the tab bar.
//!
//! Provides quick access to:
//! - New Tab
//! - Split Horizontal (left | right)
//! - Split Vertical (top / bottom)

/// Actions available from the "+" dropdown menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewTabMenuAction {
    /// Create a new tab.
    NewTab,
    /// Split the current pane horizontally (left | right).
    SplitHorizontal,
    /// Split the current pane vertically (top / bottom).
    SplitVertical,
}

impl NewTabMenuAction {
    /// Display label for the menu item.
    pub fn label(&self) -> &'static str {
        match self {
            Self::NewTab => "New Tab",
            Self::SplitHorizontal => "Split Horizontal",
            Self::SplitVertical => "Split Vertical",
        }
    }

    /// All actions in display order.
    pub fn all() -> &'static [NewTabMenuAction] {
        &[Self::NewTab, Self::SplitHorizontal, Self::SplitVertical]
    }
}

/// Dropdown menu state for the "+" button.
#[derive(Debug, Clone, Default)]
pub struct NewTabMenuState {
    /// Whether the dropdown is currently visible.
    pub visible: bool,
    /// Pixel position (x, y) where the dropdown appears (top-left corner).
    pub pos: (f32, f32),
    /// Index of the highlighted item (for hover).
    pub hovered: Option<usize>,
    /// Actual rendered width (set by renderer, used by hit_test).
    pub effective_width: f32,
}

impl NewTabMenuState {
    /// Show the dropdown at the given pixel position.
    pub fn show(&mut self, x: f32, y: f32) {
        self.visible = true;
        self.pos = (x, y);
        self.hovered = None;
    }

    /// Hide the dropdown.
    pub fn hide(&mut self) {
        self.visible = false;
        self.hovered = None;
    }

    /// Toggle visibility.
    pub fn toggle(&mut self, x: f32, y: f32) {
        if self.visible {
            self.hide();
        } else {
            self.show(x, y);
        }
    }

    /// Menu item height in physical pixels.
    pub const ITEM_HEIGHT: f32 = 32.0;
    /// Menu padding in physical pixels.
    pub const PADDING: f32 = 10.0;
    /// Menu width in physical pixels.
    pub const WIDTH: f32 = 280.0;
    /// Corner radius.
    pub const RADIUS: f32 = 8.0;

    /// Total menu height given the number of items.
    pub fn menu_height(&self) -> f32 {
        let n = NewTabMenuAction::all().len() as f32;
        n * Self::ITEM_HEIGHT + Self::PADDING * 2.0
    }

    /// Get the bounding rect of menu item `index`.
    pub fn item_rect(&self, index: usize) -> (f32, f32, f32, f32) {
        let (x, y) = self.pos;
        let item_y = y + Self::PADDING + index as f32 * Self::ITEM_HEIGHT;
        (
            x + Self::PADDING,
            item_y,
            Self::WIDTH - Self::PADDING * 2.0,
            Self::ITEM_HEIGHT,
        )
    }

    /// Hit-test a pixel position. Returns the action index if the position
    /// is within the menu bounds.
    pub fn hit_test(&self, px: f32, py: f32) -> Option<usize> {
        if !self.visible {
            return None;
        }
        let (x, y) = self.pos;
        let w = if self.effective_width > 0.0 {
            self.effective_width
        } else {
            Self::WIDTH
        };
        if px < x || px > x + w || py < y || py > y + self.menu_height() {
            return None;
        }
        let rel_y = py - y - Self::PADDING;
        if rel_y < 0.0 {
            return None;
        }
        let index = (rel_y / Self::ITEM_HEIGHT) as usize;
        let max = NewTabMenuAction::all().len();
        if index < max { Some(index) } else { None }
    }

    /// Get the action at the given index.
    pub fn action_at(index: usize) -> Option<NewTabMenuAction> {
        NewTabMenuAction::all().get(index).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_all_actions_have_labels() {
        for a in NewTabMenuAction::all() {
            assert!(!a.label().is_empty());
        }
    }

    #[test]
    fn t_all_actions_count() {
        assert_eq!(NewTabMenuAction::all().len(), 3);
    }

    #[test]
    fn t_show_hide() {
        let mut m = NewTabMenuState::default();
        assert!(!m.visible);
        m.show(100.0, 200.0);
        assert!(m.visible);
        assert_eq!(m.pos, (100.0, 200.0));
        m.hide();
        assert!(!m.visible);
    }

    #[test]
    fn t_toggle() {
        let mut m = NewTabMenuState::default();
        assert!(!m.visible);
        m.toggle(10.0, 20.0);
        assert!(m.visible);
        m.toggle(10.0, 20.0);
        assert!(!m.visible);
    }

    #[test]
    fn t_hit_test_inside() {
        let mut m = NewTabMenuState::default();
        m.show(100.0, 200.0);
        // First item.
        assert_eq!(m.hit_test(200.0, 215.0), Some(0));
        // Second item.
        assert_eq!(m.hit_test(200.0, 250.0), Some(1));
        // Third item.
        assert_eq!(m.hit_test(200.0, 285.0), Some(2));
    }

    #[test]
    fn t_hit_test_outside() {
        let mut m = NewTabMenuState::default();
        m.show(100.0, 200.0);
        assert_eq!(m.hit_test(50.0, 50.0), None);
        assert_eq!(m.hit_test(500.0, 210.0), None);
        // When not visible, always None.
        m.hide();
        assert_eq!(m.hit_test(150.0, 210.0), None);
    }

    #[test]
    fn t_menu_height() {
        let m = NewTabMenuState::default();
        // 3 items * 32 + 2 * 10 padding = 96 + 20 = 116
        assert_eq!(m.menu_height(), 116.0);
    }

    #[test]
    fn t_item_rect() {
        let mut m = NewTabMenuState::default();
        m.show(100.0, 200.0);
        let (x, y, w, h) = m.item_rect(0);
        assert!((x - 110.0).abs() < 0.01);
        assert!((y - 210.0).abs() < 0.01);
        assert!((w - 260.0).abs() < 0.01);
        assert!((h - 32.0).abs() < 0.01);
    }

    #[test]
    fn t_action_at() {
        assert_eq!(
            NewTabMenuState::action_at(0),
            Some(NewTabMenuAction::NewTab)
        );
        assert_eq!(
            NewTabMenuState::action_at(1),
            Some(NewTabMenuAction::SplitHorizontal)
        );
        assert_eq!(
            NewTabMenuState::action_at(2),
            Some(NewTabMenuAction::SplitVertical)
        );
        assert_eq!(NewTabMenuState::action_at(99), None);
    }

    #[test]
    fn t_show_resets_hovered() {
        let mut m = NewTabMenuState {
            hovered: Some(2),
            ..Default::default()
        };
        m.show(0.0, 0.0);
        assert_eq!(m.hovered, None);
    }
}
