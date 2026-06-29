//! macOS native menu bar integration (P19-F).
//!
//! Maps [`crate::menu_bar::MENU_DEFINITIONS`] to native `NSMenu`/`NSMenuItem`.
//! Menu item clicks dispatch [`MenuAction`] values through the thread-safe
//! [`crate::menu_bar::queue_action`] queue, polled from the event loop.
//!
//! # Platform
//! This module compiles **only** on `target_os = "macos"`.
//!
//! # Status
//! The data layer (tag mapping, accelerator parsing) is fully tested.
//! The native NSMenu installation is a logging stub until the objc2 0.6
//! API (`define_class!` ivars, `NSMenuItem::alloc` trait bounds) is finalized.

#![cfg(target_os = "macos")]
#![allow(dead_code)] // Data layer tested; NSMenu bridge pending objc2 0.6

use crate::menu_bar::{MENU_DEFINITIONS, MenuAction};

fn action_to_tag(action: MenuAction) -> isize {
    match action {
        MenuAction::NewTab => 1,
        MenuAction::CloseTab => 2,
        MenuAction::Quit => 3,
        MenuAction::Copy => 4,
        MenuAction::Paste => 5,
        MenuAction::SelectAll => 6,
        MenuAction::ClearScrollback => 7,
        MenuAction::ResetTerminal => 8,
        MenuAction::ZoomIn => 9,
        MenuAction::ZoomOut => 10,
        MenuAction::ZoomReset => 11,
        MenuAction::ToggleFullscreen => 12,
        MenuAction::ToggleStatusBar => 13,
        MenuAction::CycleTheme => 14,
        MenuAction::ScrollbackSearch => 15,
        MenuAction::About => 16,
    }
}

/// Convert an integer tag back to a `MenuAction`.
fn tag_to_action(tag: isize) -> Option<MenuAction> {
    match tag {
        1 => Some(MenuAction::NewTab),
        2 => Some(MenuAction::CloseTab),
        3 => Some(MenuAction::Quit),
        4 => Some(MenuAction::Copy),
        5 => Some(MenuAction::Paste),
        6 => Some(MenuAction::SelectAll),
        7 => Some(MenuAction::ClearScrollback),
        8 => Some(MenuAction::ResetTerminal),
        9 => Some(MenuAction::ZoomIn),
        10 => Some(MenuAction::ZoomOut),
        11 => Some(MenuAction::ZoomReset),
        12 => Some(MenuAction::ToggleFullscreen),
        13 => Some(MenuAction::ToggleStatusBar),
        14 => Some(MenuAction::CycleTheme),
        15 => Some(MenuAction::ScrollbackSearch),
        16 => Some(MenuAction::About),
        _ => None,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  Accelerator parsing
// ═══════════════════════════════════════════════════════════════════════════

/// Modifier flag bits matching macOS `NSEventModifierFlags` raw values.
const MOD_CONTROL: u64 = 0x40000;
const MOD_SHIFT: u64 = 0x20000;
const MOD_OPTION: u64 = 0x80000;
const MOD_COMMAND: u64 = 0x100000;

/// Parse an accelerator string like `"Ctrl+Shift+V"` into `(key, modifier_mask)`.
fn parse_accelerator(accel: &str) -> (String, u64) {
    let mut mask: u64 = 0;
    let parts: Vec<&str> = accel.split('+').collect();

    if parts.is_empty() {
        return (String::new(), 0);
    }

    let key_part = parts[parts.len() - 1].trim();

    for &part in &parts[..parts.len() - 1] {
        match part.trim().to_ascii_lowercase().as_str() {
            "ctrl" | "control" => mask |= MOD_CONTROL,
            "shift" => mask |= MOD_SHIFT,
            "alt" | "opt" | "option" => mask |= MOD_OPTION,
            "super" | "cmd" | "meta" | "win" => mask |= MOD_COMMAND,
            _ => {}
        }
    }

    let key_eq = if key_part.len() == 1 {
        key_part.to_ascii_lowercase()
    } else if key_part.starts_with('F') || key_part.starts_with('f') {
        key_part.to_lowercase()
    } else {
        String::new()
    };

    (key_eq, mask)
}

// ═══════════════════════════════════════════════════════════════════════════
//  Menu installation (logging stub — NSMenu bridge pending objc2 0.6)
// ═══════════════════════════════════════════════════════════════════════════

/// Install the native macOS menu bar.
///
/// Currently a logging stub — the actual `NSMenu` construction via objc2
/// requires resolving `define_class!` ivar syntax and `NSMenuItem::alloc()`
/// trait bounds for objc2 0.6. All keyboard shortcuts continue to work via
/// `window.rs` event handlers.
pub fn install_native_menu() {
    // Log the menu structure for debugging.
    for def in MENU_DEFINITIONS {
        log::debug!("Menu: {}", def.title);
        for item in def.items {
            if item.separator {
                log::debug!("  ---");
            } else if let Some(action) = item.action {
                log::debug!(
                    "  {} [{}] -> {:?} (tag {})",
                    item.label,
                    item.accelerator.unwrap_or(""),
                    action,
                    action_to_tag(action)
                );
            }
        }
    }
    log::info!("native menu: definitions ready (NSMenu bridge pending)");
}

// ═══════════════════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_install_does_not_panic() {
        install_native_menu();
    }

    #[test]
    fn test_action_tag_roundtrip() {
        let actions = [
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
        ];

        for action in actions {
            let tag = action_to_tag(action);
            assert_ne!(tag, 0, "tag should not be 0");
            assert_eq!(tag_to_action(tag), Some(action));
        }
    }

    #[test]
    fn test_tag_zero_returns_none() {
        assert_eq!(tag_to_action(0), None);
    }

    #[test]
    fn test_tag_unknown_returns_none() {
        assert_eq!(tag_to_action(99), None);
        assert_eq!(tag_to_action(-1), None);
    }

    #[test]
    fn test_tags_are_unique() {
        use std::collections::HashSet;
        let tags: HashSet<isize> = [
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
        .iter()
        .map(|a| action_to_tag(*a))
        .collect();

        assert_eq!(tags.len(), 16, "all 16 tags must be unique");
    }

    #[test]
    fn test_parse_accelerator_ctrl_t() {
        let (key, mask) = parse_accelerator("Ctrl+T");
        assert_eq!(key, "t");
        assert!(mask & MOD_CONTROL != 0);
        assert!(mask & MOD_SHIFT == 0);
    }

    #[test]
    fn test_parse_accelerator_ctrl_shift_v() {
        let (key, mask) = parse_accelerator("Ctrl+Shift+V");
        assert_eq!(key, "v");
        assert!(mask & MOD_CONTROL != 0);
        assert!(mask & MOD_SHIFT != 0);
    }

    #[test]
    fn test_parse_accelerator_f11() {
        let (key, mask) = parse_accelerator("F11");
        assert_eq!(key, "f11");
        assert_eq!(mask, 0);
    }

    #[test]
    fn test_parse_accelerator_cmd_k() {
        let (key, mask) = parse_accelerator("Cmd+K");
        assert_eq!(key, "k");
        assert!(mask & MOD_COMMAND != 0);
    }

    #[test]
    fn test_parse_accelerator_empty() {
        let (key, mask) = parse_accelerator("");
        assert_eq!(key, "");
        assert_eq!(mask, 0);
    }

    #[test]
    fn test_parse_accelerator_ctrl_equal() {
        let (key, mask) = parse_accelerator("Ctrl+=");
        assert_eq!(key, "=");
        assert!(mask & MOD_CONTROL != 0);
    }

    #[test]
    fn test_parse_accelerator_alt_1() {
        let (key, mask) = parse_accelerator("Alt+1");
        assert_eq!(key, "1");
        assert!(mask & MOD_OPTION != 0);
    }

    #[test]
    fn test_menu_definitions_not_empty() {
        assert!(!MENU_DEFINITIONS.is_empty());
        for def in MENU_DEFINITIONS {
            assert!(!def.title.is_empty());
            assert!(!def.items.is_empty());
        }
    }
}
