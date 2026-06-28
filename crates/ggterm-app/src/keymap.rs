//! Winit keyboard mapping: `PhysicalKey` / `Key` → `InputKey`.
//!
//! This module is feature-gated behind `desktop`. It translates winit's
//! keyboard events into our toolkit-agnostic [`InputKey`] type, which is
//! then encoded by [`InputEncoder`](crate::input::InputEncoder) into ANSI
//! bytes for the PTY.
//!
//! ## Mapping strategy
//!
//! - **Special keys** (arrows, F-keys, Home/End/PageUp/PageDown, Insert/Delete)
//!   use the physical `KeyCode` directly.
//! - **Named character keys** (Tab, Enter, Backspace, Escape) map to their
//!   conventional control characters.
//! - **Printable characters** use the logical key text from winit, which
//!   respects keyboard layout.
//! - **Modifier keys** (Shift/Ctrl/Alt) are tracked separately via
//!   `WindowEvent::ModifiersChanged`.

use crate::input::{InputKey, KeyModifiers, SpecialKey};

/// Map a winit physical key + logical key to our `InputKey`.
///
/// Returns `None` for keys we don't know how to encode (e.g. modifier keys
/// themselves, media keys, etc.).
pub fn map_winit_key(
    physical: &winit::keyboard::PhysicalKey,
    logical_text: Option<&str>,
    mods: &KeyModifiers,
) -> Option<InputKey> {
    use winit::keyboard::KeyCode;

    let code = match physical {
        winit::keyboard::PhysicalKey::Code(code) => *code,
        _ => return None,
    };

    // 1. Arrow keys
    let special = match code {
        KeyCode::ArrowUp => SpecialKey::Up,
        KeyCode::ArrowDown => SpecialKey::Down,
        KeyCode::ArrowLeft => SpecialKey::Left,
        KeyCode::ArrowRight => SpecialKey::Right,
        KeyCode::Home => SpecialKey::Home,
        KeyCode::End => SpecialKey::End,
        KeyCode::PageUp => SpecialKey::PageUp,
        KeyCode::PageDown => SpecialKey::PageDown,
        KeyCode::Insert => SpecialKey::Insert,
        KeyCode::Delete => SpecialKey::Delete,
        KeyCode::F1 => SpecialKey::F1,
        KeyCode::F2 => SpecialKey::F2,
        KeyCode::F3 => SpecialKey::F3,
        KeyCode::F4 => SpecialKey::F4,
        KeyCode::F5 => SpecialKey::F5,
        KeyCode::F6 => SpecialKey::F6,
        KeyCode::F7 => SpecialKey::F7,
        KeyCode::F8 => SpecialKey::F8,
        KeyCode::F9 => SpecialKey::F9,
        KeyCode::F10 => SpecialKey::F10,
        KeyCode::F11 => SpecialKey::F11,
        KeyCode::F12 => SpecialKey::F12,
        _ => {
            // Fall through to character mapping below
            return map_char_key(code, logical_text, mods);
        }
    };
    Some(InputKey::special_mod(special, *mods))
}

/// Map character-producing keys (Tab, Enter, Backspace, Escape, printable chars).
fn map_char_key(
    code: winit::keyboard::KeyCode,
    logical_text: Option<&str>,
    mods: &KeyModifiers,
) -> Option<InputKey> {
    use winit::keyboard::KeyCode;

    // Named keys that produce specific control characters
    let ch = match code {
        KeyCode::Tab => '\t',
        KeyCode::Enter => '\r',
        KeyCode::Backspace => '\x7f', // DEL — what most terminals expect
        KeyCode::Escape => '\x1b',
        KeyCode::Space => ' ',
        _ => {
            // Printable characters: use the logical key text from winit.
            // This respects keyboard layout (e.g. 'a' on QWERTY vs 'а' on ЙЦУКЕН).
            return logical_text
                .and_then(|s| s.chars().next())
                .map(|ch| InputKey::char_mod(ch, *mods));
        }
    };
    Some(InputKey::char_mod(ch, *mods))
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::{KeyCode, PhysicalKey};

    fn pk(code: KeyCode) -> PhysicalKey {
        PhysicalKey::Code(code)
    }

    fn no_mods() -> KeyModifiers {
        KeyModifiers::default()
    }

    fn shift() -> KeyModifiers {
        KeyModifiers { shift: true, ctrl: false, alt: false }
    }

    fn ctrl() -> KeyModifiers {
        KeyModifiers { shift: false, ctrl: true, alt: false }
    }

    fn alt() -> KeyModifiers {
        KeyModifiers { shift: false, ctrl: false, alt: true }
    }

    // --- Arrow keys ---

    #[test]
    fn arrow_up() {
        let key = map_winit_key(&pk(KeyCode::ArrowUp), None, &no_mods());
        assert_eq!(key, Some(InputKey::special(SpecialKey::Up)));
    }

    #[test]
    fn arrow_down() {
        let key = map_winit_key(&pk(KeyCode::ArrowDown), None, &no_mods());
        assert_eq!(key, Some(InputKey::special(SpecialKey::Down)));
    }

    #[test]
    fn arrow_left() {
        let key = map_winit_key(&pk(KeyCode::ArrowLeft), None, &no_mods());
        assert_eq!(key, Some(InputKey::special(SpecialKey::Left)));
    }

    #[test]
    fn arrow_right() {
        let key = map_winit_key(&pk(KeyCode::ArrowRight), None, &no_mods());
        assert_eq!(key, Some(InputKey::special(SpecialKey::Right)));
    }

    #[test]
    fn arrow_with_ctrl() {
        let key = map_winit_key(&pk(KeyCode::ArrowUp), None, &ctrl());
        assert_eq!(key, Some(InputKey::special_mod(SpecialKey::Up, ctrl())));
    }

    // --- Navigation keys ---

    #[test]
    fn home_key() {
        let key = map_winit_key(&pk(KeyCode::Home), None, &no_mods());
        assert_eq!(key, Some(InputKey::special(SpecialKey::Home)));
    }

    #[test]
    fn end_key() {
        let key = map_winit_key(&pk(KeyCode::End), None, &no_mods());
        assert_eq!(key, Some(InputKey::special(SpecialKey::End)));
    }

    #[test]
    fn page_up() {
        let key = map_winit_key(&pk(KeyCode::PageUp), None, &no_mods());
        assert_eq!(key, Some(InputKey::special(SpecialKey::PageUp)));
    }

    #[test]
    fn page_down() {
        let key = map_winit_key(&pk(KeyCode::PageDown), None, &no_mods());
        assert_eq!(key, Some(InputKey::special(SpecialKey::PageDown)));
    }

    #[test]
    fn insert_key() {
        let key = map_winit_key(&pk(KeyCode::Insert), None, &no_mods());
        assert_eq!(key, Some(InputKey::special(SpecialKey::Insert)));
    }

    #[test]
    fn delete_key() {
        let key = map_winit_key(&pk(KeyCode::Delete), None, &no_mods());
        assert_eq!(key, Some(InputKey::special(SpecialKey::Delete)));
    }

    // --- Function keys ---

    #[test]
    fn f1_through_f4() {
        assert_eq!(
            map_winit_key(&pk(KeyCode::F1), None, &no_mods()),
            Some(InputKey::special(SpecialKey::F1))
        );
        assert_eq!(
            map_winit_key(&pk(KeyCode::F2), None, &no_mods()),
            Some(InputKey::special(SpecialKey::F2))
        );
        assert_eq!(
            map_winit_key(&pk(KeyCode::F3), None, &no_mods()),
            Some(InputKey::special(SpecialKey::F3))
        );
        assert_eq!(
            map_winit_key(&pk(KeyCode::F4), None, &no_mods()),
            Some(InputKey::special(SpecialKey::F4))
        );
    }

    #[test]
    fn f5_through_f8() {
        assert_eq!(
            map_winit_key(&pk(KeyCode::F5), None, &no_mods()),
            Some(InputKey::special(SpecialKey::F5))
        );
        assert_eq!(
            map_winit_key(&pk(KeyCode::F6), None, &no_mods()),
            Some(InputKey::special(SpecialKey::F6))
        );
        assert_eq!(
            map_winit_key(&pk(KeyCode::F7), None, &no_mods()),
            Some(InputKey::special(SpecialKey::F7))
        );
        assert_eq!(
            map_winit_key(&pk(KeyCode::F8), None, &no_mods()),
            Some(InputKey::special(SpecialKey::F8))
        );
    }

    #[test]
    fn f9_through_f12() {
        assert_eq!(
            map_winit_key(&pk(KeyCode::F9), None, &no_mods()),
            Some(InputKey::special(SpecialKey::F9))
        );
        assert_eq!(
            map_winit_key(&pk(KeyCode::F10), None, &no_mods()),
            Some(InputKey::special(SpecialKey::F10))
        );
        assert_eq!(
            map_winit_key(&pk(KeyCode::F11), None, &no_mods()),
            Some(InputKey::special(SpecialKey::F11))
        );
        assert_eq!(
            map_winit_key(&pk(KeyCode::F12), None, &no_mods()),
            Some(InputKey::special(SpecialKey::F12))
        );
    }

    // --- Named character keys ---

    #[test]
    fn tab_key() {
        let key = map_winit_key(&pk(KeyCode::Tab), None, &no_mods());
        assert_eq!(key, Some(InputKey::char('\t')));
    }

    #[test]
    fn enter_key() {
        let key = map_winit_key(&pk(KeyCode::Enter), None, &no_mods());
        assert_eq!(key, Some(InputKey::char('\r')));
    }

    #[test]
    fn backspace_key() {
        let key = map_winit_key(&pk(KeyCode::Backspace), None, &no_mods());
        assert_eq!(key, Some(InputKey::char('\x7f')));
    }

    #[test]
    fn escape_key() {
        let key = map_winit_key(&pk(KeyCode::Escape), None, &no_mods());
        assert_eq!(key, Some(InputKey::char('\x1b')));
    }

    // --- Printable characters ---

    #[test]
    fn printable_a() {
        let key = map_winit_key(&pk(KeyCode::KeyA), Some("a"), &no_mods());
        assert_eq!(key, Some(InputKey::char('a')));
    }

    #[test]
    fn printable_uppercase() {
        let key = map_winit_key(&pk(KeyCode::KeyA), Some("A"), &shift());
        assert_eq!(key, Some(InputKey::char_mod('A', shift())));
    }

    #[test]
    fn printable_number() {
        let key = map_winit_key(&pk(KeyCode::Digit1), Some("1"), &no_mods());
        assert_eq!(key, Some(InputKey::char('1')));
    }

    #[test]
    fn printable_with_ctrl() {
        let key = map_winit_key(&pk(KeyCode::KeyA), Some("a"), &ctrl());
        assert_eq!(key, Some(InputKey::char_mod('a', ctrl())));
    }

    #[test]
    fn printable_with_alt() {
        let key = map_winit_key(&pk(KeyCode::KeyA), Some("a"), &alt());
        assert_eq!(key, Some(InputKey::char_mod('a', alt())));
    }

    #[test]
    fn space_key() {
        let key = map_winit_key(&pk(KeyCode::Space), Some(" "), &no_mods());
        assert_eq!(key, Some(InputKey::char(' ')));
    }

    // --- Edge cases ---

    #[test]
    fn unidentified_physical_returns_none() {
        let unidentified = winit::keyboard::PhysicalKey::Unidentified(
            winit::keyboard::NativeKeyCode::Unidentified,
        );
        let key = map_winit_key(&unidentified, None, &no_mods());
        assert_eq!(key, None);
    }

    #[test]
    fn printable_no_logical_text() {
        // Some keycodes don't have logical text (e.g. modifier-only keys)
        let key = map_winit_key(&pk(KeyCode::ShiftLeft), None, &no_mods());
        assert_eq!(key, None);
    }

    #[test]
    fn printable_unicode() {
        // CJK character from logical key text
        let key = map_winit_key(&pk(KeyCode::KeyA), Some("中"), &no_mods());
        assert_eq!(key, Some(InputKey::char('中')));
    }

    // --- InputEncoder integration ---

    #[test]
    fn keymap_then_encode_arrow_up() {
        let encoder = crate::input::InputEncoder::new();
        let key = map_winit_key(&pk(KeyCode::ArrowUp), None, &no_mods()).unwrap();
        let bytes = encoder.encode(&key);
        assert_eq!(bytes, b"\x1b[A");
    }

    #[test]
    fn keymap_then_encode_enter() {
        let encoder = crate::input::InputEncoder::new();
        let key = map_winit_key(&pk(KeyCode::Enter), None, &no_mods()).unwrap();
        let bytes = encoder.encode(&key);
        assert_eq!(bytes, b"\r");
    }

    #[test]
    fn keymap_then_encode_ctrl_c() {
        let encoder = crate::input::InputEncoder::new();
        let key = map_winit_key(&pk(KeyCode::KeyC), Some("c"), &ctrl()).unwrap();
        let bytes = encoder.encode(&key);
        assert_eq!(bytes, b"\x03");
    }

    #[test]
    fn keymap_then_encode_f1() {
        let encoder = crate::input::InputEncoder::new();
        let key = map_winit_key(&pk(KeyCode::F1), None, &no_mods()).unwrap();
        let bytes = encoder.encode(&key);
        assert_eq!(bytes, b"\x1bOP");
    }
}
