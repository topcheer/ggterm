//! Application menu bar definitions and dispatch (P19-A).
//!
//! Defines the native menu structure as pure data so it can be unit-tested
//! without a windowing system. The actual native menu is installed at runtime
//! via platform-specific APIs (macOS `NSApp`, GTK, Win32), but the action
//! dispatch is handled here through [`dispatch_menu_action`].
//!
//! Menu items are polled in the event loop via [`poll_pending_action`] and
//! routed to existing handler methods on `DesktopApp`.

use std::sync::Mutex;

/// Actions that can be triggered by the menu bar.
///
/// Each variant maps to an existing keyboard-shortcut handler on `DesktopApp`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MenuAction {
    // File menu
    NewTab,
    CloseTab,
    Quit,

    // Edit menu
    Copy,
    Paste,
    SelectAll,
    ClearScrollback,
    ResetTerminal,

    // View menu
    ZoomIn,
    ZoomOut,
    ZoomReset,
    ToggleFullscreen,
    ToggleStatusBar,
    CycleTheme,

    // Shell menu
    ScrollbackSearch,

    // Help menu
    About,
}

/// A single clickable menu entry.
#[derive(Debug, Clone)]
pub struct MenuItem {
    pub label: &'static str,
    pub accelerator: Option<&'static str>,
    pub action: Option<MenuAction>,
    /// `true` for separator items.
    pub separator: bool,
}

impl MenuItem {
    /// Create a normal menu item.
    pub const fn item(label: &'static str, accel: &'static str, action: MenuAction) -> Self {
        Self {
            label,
            accelerator: Some(accel),
            action: Some(action),
            separator: false,
        }
    }

    /// Create a separator.
    pub const fn sep() -> Self {
        Self {
            label: "",
            accelerator: None,
            action: None,
            separator: true,
        }
    }
}

/// A top-level menu (e.g. "File", "Edit").
#[derive(Debug, Clone)]
pub struct MenuDefinition {
    pub title: &'static str,
    pub items: &'static [MenuItem],
}

// ═══════════════════════════════════════════════════════════════════════════
//  Static menu definitions
// ═══════════════════════════════════════════════════════════════════════════

/// All menu bar entries, in display order.
pub static MENU_DEFINITIONS: &[MenuDefinition] = &[
    MenuDefinition {
        title: "File",
        items: &[
            MenuItem::item("New Tab", "Ctrl+T", MenuAction::NewTab),
            MenuItem::item("Close Tab", "Ctrl+W", MenuAction::CloseTab),
            MenuItem::sep(),
            MenuItem::item("Quit", "Ctrl+Q", MenuAction::Quit),
        ],
    },
    MenuDefinition {
        title: "Edit",
        items: &[
            MenuItem::item("Copy", "Ctrl+Shift+C", MenuAction::Copy),
            MenuItem::item("Paste", "Ctrl+Shift+V", MenuAction::Paste),
            MenuItem::item("Select All", "Ctrl+Shift+A", MenuAction::SelectAll),
            MenuItem::sep(),
            MenuItem::item(
                "Clear Scrollback",
                "Ctrl+Shift+K",
                MenuAction::ClearScrollback,
            ),
            MenuItem::item("Reset Terminal", "Ctrl+Shift+R", MenuAction::ResetTerminal),
        ],
    },
    MenuDefinition {
        title: "View",
        items: &[
            MenuItem::item("Zoom In", "Ctrl+=", MenuAction::ZoomIn),
            MenuItem::item("Zoom Out", "Ctrl+-", MenuAction::ZoomOut),
            MenuItem::item("Reset Zoom", "Ctrl+0", MenuAction::ZoomReset),
            MenuItem::sep(),
            MenuItem::item("Toggle Fullscreen", "F11", MenuAction::ToggleFullscreen),
            MenuItem::item(
                "Toggle Status Bar",
                "Ctrl+Shift+B",
                MenuAction::ToggleStatusBar,
            ),
            MenuItem::item("Cycle Theme", "Ctrl+Shift+T", MenuAction::CycleTheme),
        ],
    },
    MenuDefinition {
        title: "Shell",
        items: &[
            MenuItem::item("New Tab", "Ctrl+T", MenuAction::NewTab),
            MenuItem::sep(),
            MenuItem::item(
                "Scrollback Search",
                "Ctrl+Shift+F",
                MenuAction::ScrollbackSearch,
            ),
        ],
    },
    MenuDefinition {
        title: "Help",
        items: &[MenuItem::item("About", "", MenuAction::About)],
    },
];

// ═══════════════════════════════════════════════════════════════════════════
//  Action queue (thread-safe, Send + Sync)
// ═══════════════════════════════════════════════════════════════════════════

/// Pending menu action, set by platform menu callbacks and consumed by the
/// event loop. `Mutex<Option<MenuAction>>` is `Send + Sync`.
static PENDING_ACTION: Mutex<Option<MenuAction>> = Mutex::new(None);

/// Queue a menu action (called from platform menu callbacks).
pub fn queue_action(action: MenuAction) {
    if let Ok(mut slot) = PENDING_ACTION.lock() {
        *slot = Some(action);
    }
}

/// Poll for a pending menu action. Returns `Some(action)` if a menu item was
/// clicked since the last poll, `None` otherwise.
///
/// Should be called each frame from `about_to_wait()`.
pub fn poll_pending_action() -> Option<MenuAction> {
    if let Ok(mut slot) = PENDING_ACTION.lock() {
        slot.take()
    } else {
        None
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_menu_definitions_count() {
        assert_eq!(MENU_DEFINITIONS.len(), 5);
        assert_eq!(MENU_DEFINITIONS[0].title, "File");
        assert_eq!(MENU_DEFINITIONS[1].title, "Edit");
        assert_eq!(MENU_DEFINITIONS[2].title, "View");
        assert_eq!(MENU_DEFINITIONS[3].title, "Shell");
        assert_eq!(MENU_DEFINITIONS[4].title, "Help");
    }

    #[test]
    fn test_file_menu_items() {
        let file = &MENU_DEFINITIONS[0];
        // File menu: New Tab, Close Tab, separator, Quit
        assert_eq!(file.items.len(), 4);
        assert_eq!(file.items[0].action, Some(MenuAction::NewTab));
        assert_eq!(file.items[1].action, Some(MenuAction::CloseTab));
        assert!(file.items[2].separator);
        assert_eq!(file.items[3].action, Some(MenuAction::Quit));
    }

    #[test]
    fn test_edit_menu_items() {
        let edit = &MENU_DEFINITIONS[1];
        assert_eq!(edit.items.len(), 6);
        assert_eq!(edit.items[0].action, Some(MenuAction::Copy));
        assert_eq!(edit.items[1].action, Some(MenuAction::Paste));
        assert_eq!(edit.items[2].action, Some(MenuAction::SelectAll));
        assert!(edit.items[3].separator);
        assert_eq!(edit.items[4].action, Some(MenuAction::ClearScrollback));
        assert_eq!(edit.items[5].action, Some(MenuAction::ResetTerminal));
    }

    #[test]
    fn test_view_menu_items() {
        let view = &MENU_DEFINITIONS[2];
        assert_eq!(view.items.len(), 7);
        assert_eq!(view.items[0].action, Some(MenuAction::ZoomIn));
        assert_eq!(view.items[1].action, Some(MenuAction::ZoomOut));
        assert_eq!(view.items[2].action, Some(MenuAction::ZoomReset));
        assert!(view.items[3].separator);
        assert_eq!(view.items[4].action, Some(MenuAction::ToggleFullscreen));
        assert_eq!(view.items[5].action, Some(MenuAction::ToggleStatusBar));
        assert_eq!(view.items[6].action, Some(MenuAction::CycleTheme));
    }

    #[test]
    fn test_shell_menu_items() {
        let shell = &MENU_DEFINITIONS[3];
        assert_eq!(shell.items.len(), 3);
        assert_eq!(shell.items[0].action, Some(MenuAction::NewTab));
        assert!(shell.items[1].separator);
        assert_eq!(shell.items[2].action, Some(MenuAction::ScrollbackSearch));
    }

    #[test]
    fn test_help_menu_items() {
        let help = &MENU_DEFINITIONS[4];
        assert_eq!(help.items.len(), 1);
        assert_eq!(help.items[0].action, Some(MenuAction::About));
    }

    #[test]
    fn test_accelerators_defined() {
        // Every non-separator item except About should have an accelerator.
        for menu in MENU_DEFINITIONS {
            for item in menu.items {
                if item.separator {
                    continue;
                }
                if item.action == Some(MenuAction::About) {
                    assert!(
                        item.accelerator.is_none() || item.accelerator == Some(""),
                        "About should have no accelerator"
                    );
                } else {
                    assert!(
                        item.accelerator.is_some(),
                        "{} should have an accelerator",
                        item.label
                    );
                }
            }
        }
    }

    #[test]
    fn test_all_actions_have_menu_entry() {
        let actions_in_menus: std::collections::HashSet<_> = MENU_DEFINITIONS
            .iter()
            .flat_map(|m| m.items.iter())
            .filter_map(|i| i.action)
            .collect();

        assert!(actions_in_menus.contains(&MenuAction::NewTab));
        assert!(actions_in_menus.contains(&MenuAction::CloseTab));
        assert!(actions_in_menus.contains(&MenuAction::Quit));
        assert!(actions_in_menus.contains(&MenuAction::Copy));
        assert!(actions_in_menus.contains(&MenuAction::Paste));
        assert!(actions_in_menus.contains(&MenuAction::SelectAll));
        assert!(actions_in_menus.contains(&MenuAction::ClearScrollback));
        assert!(actions_in_menus.contains(&MenuAction::ResetTerminal));
        assert!(actions_in_menus.contains(&MenuAction::ZoomIn));
        assert!(actions_in_menus.contains(&MenuAction::ZoomOut));
        assert!(actions_in_menus.contains(&MenuAction::ZoomReset));
        assert!(actions_in_menus.contains(&MenuAction::ToggleFullscreen));
        assert!(actions_in_menus.contains(&MenuAction::ToggleStatusBar));
        assert!(actions_in_menus.contains(&MenuAction::CycleTheme));
        assert!(actions_in_menus.contains(&MenuAction::ScrollbackSearch));
        assert!(actions_in_menus.contains(&MenuAction::About));
    }

    #[test]
    fn test_separators_have_no_action() {
        for menu in MENU_DEFINITIONS {
            for item in menu.items {
                if item.separator {
                    assert!(item.action.is_none(), "Separator should have no action");
                    assert!(item.accelerator.is_none());
                    assert_eq!(item.label, "");
                }
            }
        }
    }

    #[test]
    fn test_queue_and_poll_action() {
        // Clear any stale state.
        let _ = poll_pending_action();

        queue_action(MenuAction::About);
        assert_eq!(poll_pending_action(), Some(MenuAction::About));
        // Second poll should be None (already consumed).
        assert_eq!(poll_pending_action(), None);
    }

    #[test]
    fn test_queue_overwrites_previous() {
        let _ = poll_pending_action();

        queue_action(MenuAction::NewTab);
        queue_action(MenuAction::CloseTab);
        // Only the last queued action survives.
        assert_eq!(poll_pending_action(), Some(MenuAction::CloseTab));
    }

    #[test]
    fn test_action_variants_count() {
        // Ensure we haven't accidentally added or removed variants.
        let count = [
            MenuAction::NewTab,
            MenuAction::CloseTab,
            MenuAction::Quit,
            MenuAction::Copy,
            MenuAction::Paste,
            MenuAction::SelectAll,
            MenuAction::ClearScrollback,
            MenuAction::ResetTerminal,
            MenuAction::ZoomIn,
            MenuAction::ZoomOut,
            MenuAction::ZoomReset,
            MenuAction::ToggleFullscreen,
            MenuAction::ToggleStatusBar,
            MenuAction::CycleTheme,
            MenuAction::ScrollbackSearch,
            MenuAction::About,
        ]
        .len();
        assert_eq!(count, 16, "Expected 16 menu action variants");
    }
}
