//! P27-C: Right-click context menu for the terminal.
//!
//! A simple popup menu rendered with SDF UiRects. Appears at the mouse
//! position on right-click and closes on selection or outside-click.

/// A context menu action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextMenuAction {
    /// Copy selected text to clipboard.
    Copy,
    /// Paste from clipboard.
    Paste,
    /// Select all text in the visible viewport.
    SelectAll,
    /// Search in scrollback.
    Search,
    /// Search selected text on the web (opens default browser).
    SearchWeb,
    /// Open URL at cursor position in default browser.
    OpenUrl,
    /// Copy the output of the last completed command to clipboard.
    CopyOutput,
    /// Split current pane horizontally (left | right).
    SplitHorizontal,
    /// Split current pane vertically (top / bottom).
    SplitVertical,
    /// Clear scrollback + screen.
    Clear,
    /// Export scrollback to a text file.
    ExportScrollback,
    /// Reset terminal — reinitialize the shell session.
    Reset,
}

impl ContextMenuAction {
    /// Display label for the menu item.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Copy => "Copy",
            Self::Paste => "Paste",
            Self::SelectAll => "Select All",
            Self::Search => "Search",
            Self::SearchWeb => "Search Web",
            Self::OpenUrl => "Open URL",
            Self::CopyOutput => "Copy Output",
            Self::SplitHorizontal => "Split Horizontal",
            Self::SplitVertical => "Split Vertical",
            Self::Clear => "Clear",
            Self::ExportScrollback => "Export Scrollback",
            Self::Reset => "Reset",
        }
    }

    /// All actions in display order.
    pub fn all() -> &'static [ContextMenuAction] {
        &[
            Self::Copy,
            Self::Paste,
            Self::SelectAll,
            Self::Search,
            Self::SearchWeb,
            Self::OpenUrl,
            Self::CopyOutput,
            Self::SplitHorizontal,
            Self::SplitVertical,
            Self::Clear,
            Self::ExportScrollback,
            Self::Reset,
        ]
    }

    /// Default keyboard shortcut text shown right-aligned in the menu.
    /// Returns `None` for actions without a default shortcut.
    pub fn shortcut(&self) -> Option<&'static str> {
        if cfg!(target_os = "macos") {
            match self {
                Self::Copy => Some("\u{2318}C"),
                Self::Paste => Some("\u{2318}V"),
                Self::SelectAll => Some("\u{2318}A"),
                Self::Search => Some("\u{2318}F"),
                Self::Clear => Some("\u{2318}K"),
                Self::SplitHorizontal => Some("\u{2318}D"),
                Self::SplitVertical => Some("\u{21E7}\u{2318}D"),
                _ => None,
            }
        } else {
            match self {
                Self::Copy => Some("Ctrl+Shift+C"),
                Self::Paste => Some("Ctrl+Shift+V"),
                Self::SelectAll => Some("Ctrl+Shift+A"),
                Self::Search => Some("Ctrl+Shift+F"),
                Self::Clear => Some("Ctrl+Shift+L"),
                Self::SplitHorizontal => Some("Ctrl+Shift+D"),
                Self::SplitVertical => Some("Ctrl+Shift+-"),
                _ => None,
            }
        }
    }

    /// Separator before action groups: [clipboard], [search], [splits], [actions].
    pub fn separator_before(action: &Self) -> bool {
        matches!(action, Self::Search | Self::SplitHorizontal | Self::Clear)
    }

    /// Whether this action is currently applicable given the app state.
    ///
    /// Disabled items are rendered dimmed and cannot be activated.
    pub fn is_enabled(&self, has_selection: bool, has_url: bool, clipboard_has_text: bool) -> bool {
        match self {
            Self::Copy => has_selection,
            Self::Paste => clipboard_has_text,
            Self::SearchWeb => has_selection,
            Self::OpenUrl => has_url,
            _ => true, // SelectAll, Search, splits, Clear, Export, Reset always available
        }
    }
}

/// Context menu state.
#[derive(Debug, Clone, Default)]
pub struct ContextMenuState {
    /// Whether the menu is currently visible.
    pub visible: bool,
    /// Pixel position (x, y) where the menu appears (top-left corner).
    pub pos: (f32, f32),
    /// Index of the highlighted item (for hover).
    pub hovered: Option<usize>,
    /// Actual rendered width (set by renderer, used by hit_test).
    pub effective_width: f32,
}

impl ContextMenuState {
    /// Show the menu at the given pixel position.
    pub fn show(&mut self, x: f32, y: f32) {
        self.visible = true;
        self.pos = (x, y);
        self.hovered = None;
    }

    /// Hide the menu.
    pub fn hide(&mut self) {
        self.visible = false;
        self.hovered = None;
    }

    /// Menu item height in physical pixels.
    pub const ITEM_HEIGHT: f32 = 32.0;
    /// Menu padding in physical pixels.
    pub const PADDING: f32 = 10.0;
    /// Menu width in physical pixels.
    /// Wide enough to fit labels + shortcut hints.
    pub const WIDTH: f32 = 260.0;
    /// Corner radius.
    pub const RADIUS: f32 = 8.0;

    /// Total menu height given the number of items.
    pub fn menu_height(&self) -> f32 {
        let n = ContextMenuAction::all().len() as f32;
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
        let max = ContextMenuAction::all().len();
        if index < max { Some(index) } else { None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_all_actions_have_labels() {
        for a in ContextMenuAction::all() {
            assert!(!a.label().is_empty());
        }
    }

    #[test]
    fn t_show_hide() {
        let mut m = ContextMenuState::default();
        assert!(!m.visible);
        m.show(100.0, 200.0);
        assert!(m.visible);
        assert_eq!(m.pos, (100.0, 200.0));
        m.hide();
        assert!(!m.visible);
    }

    #[test]
    fn t_hit_test_inside() {
        let mut m = ContextMenuState::default();
        m.show(100.0, 200.0);
        // First item.
        assert_eq!(m.hit_test(180.0, 220.0), Some(0));
        // Second item.
        assert_eq!(m.hit_test(180.0, 255.0), Some(1));
        // Last item (8 actions, index 7).
        let last_y = 200.0 + ContextMenuState::PADDING + 7.0 * ContextMenuState::ITEM_HEIGHT;
        assert_eq!(m.hit_test(180.0, last_y), Some(7));
    }

    #[test]
    fn t_hit_test_outside() {
        let mut m = ContextMenuState::default();
        m.show(100.0, 200.0);
        assert_eq!(m.hit_test(50.0, 50.0), None);
        assert_eq!(m.hit_test(400.0, 210.0), None);
        // When not visible, always None.
        m.hide();
        assert_eq!(m.hit_test(150.0, 210.0), None);
    }

    #[test]
    fn t_menu_height() {
        let m = ContextMenuState::default();
        let h = m.menu_height();
        // 12 items * 32 + 2 * 10 padding = 384 + 20 = 404
        assert_eq!(h, 404.0);
    }

    #[test]
    fn t_item_rect() {
        let mut m = ContextMenuState::default();
        m.show(100.0, 200.0);
        let (x, y, w, h) = m.item_rect(0);
        assert!((x - 110.0).abs() < 0.01); // 100 + padding(10)
        assert!((y - 210.0).abs() < 0.01); // 200 + padding(10)
        assert!((w - 240.0).abs() < 0.01); // 260 - 2*10
        assert!((h - 32.0).abs() < 0.01); // ITEM_HEIGHT
    }

    #[test]
    fn t_hovered_initially_none() {
        let m = ContextMenuState::default();
        assert_eq!(m.hovered, None);
    }

    #[test]
    fn t_show_resets_hovered() {
        let mut m = ContextMenuState {
            hovered: Some(3),
            ..Default::default()
        };
        m.show(0.0, 0.0);
        assert_eq!(m.hovered, None);
    }

    #[test]
    fn t_shortcut_not_empty() {
        for a in ContextMenuAction::all() {
            if let Some(s) = a.shortcut() {
                assert!(!s.is_empty());
            }
        }
    }

    #[test]
    fn t_copy_paste_have_shortcuts() {
        assert!(ContextMenuAction::Copy.shortcut().is_some());
        assert!(ContextMenuAction::Paste.shortcut().is_some());
    }

    #[test]
    fn t_separator_before_search() {
        assert!(ContextMenuAction::separator_before(
            &ContextMenuAction::Search
        ));
    }

    #[test]
    fn t_separator_before_splits() {
        assert!(ContextMenuAction::separator_before(
            &ContextMenuAction::SplitHorizontal
        ));
    }

    #[test]
    fn t_separator_before_clear() {
        assert!(ContextMenuAction::separator_before(
            &ContextMenuAction::Clear
        ));
    }

    #[test]
    fn t_no_separator_before_copy() {
        assert!(!ContextMenuAction::separator_before(
            &ContextMenuAction::Copy
        ));
    }

    /// Regression: CopyOutput is in all() and has correct label.
    #[test]
    fn t_copy_output_in_all() {
        let all = ContextMenuAction::all();
        assert!(
            all.contains(&ContextMenuAction::CopyOutput),
            "CopyOutput must be in all()"
        );
    }

    #[test]
    fn t_copy_output_label() {
        assert_eq!(ContextMenuAction::CopyOutput.label(), "Copy Output");
    }

    /// Regression: is_enabled for CopyOutput is always true (output may or
    /// may not exist, but the action handles that with a toast).
    #[test]
    fn t_copy_output_always_enabled() {
        assert!(ContextMenuAction::CopyOutput.is_enabled(false, false, false));
        assert!(ContextMenuAction::CopyOutput.is_enabled(true, true, true));
    }
}
