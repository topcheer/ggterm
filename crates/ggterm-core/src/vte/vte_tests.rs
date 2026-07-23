//! Tests for the VTE parser state machine.
//!
//! These tests use a mock Perform implementation that records all
//! callbacks, allowing verification of parser behavior without
//! depending on the full Terminal implementation.

use crate::vte::{Parser, Perform};

/// Events recorded by the mock Perform implementation.
#[derive(Debug, Clone, PartialEq)]
enum Event {
    Print(u8),
    Execute(u8),
    Csi(Vec<u16>, u8),
    Esc(u8),
    Osc(Vec<u8>),
}

/// Mock Perform that records all callbacks into a Vec.
struct MockPerform {
    events: Vec<Event>,
}

impl MockPerform {
    fn new() -> Self {
        Self { events: Vec::new() }
    }
}

impl Perform for MockPerform {
    fn print(&mut self, byte: u8) {
        self.events.push(Event::Print(byte));
    }

    fn execute(&mut self, byte: u8) {
        self.events.push(Event::Execute(byte));
    }

    fn csi(&mut self, _intermediates: &[u8], params: &[u16], final_byte: u8) {
        self.events.push(Event::Csi(params.to_vec(), final_byte));
    }

    fn esc(&mut self, _intermediates: &[u8], final_byte: u8) {
        self.events.push(Event::Esc(final_byte));
    }

    fn osc(&mut self, data: &[u8]) {
        self.events.push(Event::Osc(data.to_vec()));
    }
}

// ── Basic print ───────────────────────────────────────────────────

#[test]
fn test_print_simple_ascii() {
    let mut parser = Parser::new();
    let mut p = MockPerform::new();
    parser.feed(b"Hello", &mut p);
    assert_eq!(
        p.events,
        vec![
            Event::Print(b'H'),
            Event::Print(b'e'),
            Event::Print(b'l'),
            Event::Print(b'l'),
            Event::Print(b'o'),
        ]
    );
}

// ── Control characters ───────────────────────────────────────────

#[test]
fn test_execute_bell() {
    let mut parser = Parser::new();
    let mut p = MockPerform::new();
    parser.feed(b"\x07", &mut p);
    assert_eq!(p.events, vec![Event::Execute(0x07)]);
}

#[test]
fn test_execute_crlf() {
    let mut parser = Parser::new();
    let mut p = MockPerform::new();
    parser.feed(b"\r\n", &mut p);
    assert_eq!(p.events, vec![Event::Execute(b'\r'), Event::Execute(b'\n')]);
}

// ── CSI sequences ────────────────────────────────────────────────

#[test]
fn test_csi_no_params() {
    let mut parser = Parser::new();
    let mut p = MockPerform::new();
    // ESC [ H → cursor home (no params)
    parser.feed(b"\x1b[H", &mut p);
    assert_eq!(p.events, vec![Event::Csi(vec![], b'H')]);
}

#[test]
fn test_csi_single_param() {
    let mut parser = Parser::new();
    let mut p = MockPerform::new();
    // ESC [ 2 J → erase entire display
    parser.feed(b"\x1b[2J", &mut p);
    assert_eq!(p.events, vec![Event::Csi(vec![2], b'J')]);
}

#[test]
fn test_csi_multiple_params() {
    let mut parser = Parser::new();
    let mut p = MockPerform::new();
    // ESC [ 10 ; 20 H → cursor to row 10, col 20
    parser.feed(b"\x1b[10;20H", &mut p);
    assert_eq!(p.events, vec![Event::Csi(vec![10, 20], b'H')]);
}

#[test]
fn test_csi_many_params() {
    let mut parser = Parser::new();
    let mut p = MockPerform::new();
    // ESC [ 1 ; 2 ; 3 ; 4 m → SGR with 4 params
    parser.feed(b"\x1b[1;2;3;4m", &mut p);
    assert_eq!(p.events, vec![Event::Csi(vec![1, 2, 3, 4], b'm')]);
}

#[test]
fn test_csi_private_mode() {
    let mut parser = Parser::new();
    let mut p = MockPerform::new();
    // ESC [ ? 25 h → show cursor (private mode)
    parser.feed(b"\x1b[?25h", &mut p);
    assert_eq!(p.events.len(), 1);
    // Private mode uses '?' intermediate, params=[25], final='h'
    match &p.events[0] {
        Event::Csi(params, final_byte) => {
            assert_eq!(params, &vec![25]);
            assert_eq!(*final_byte, b'h');
        }
        other => panic!("expected Csi, got {other:?}"),
    }
}

#[test]
fn test_csi_empty_param_defaults() {
    let mut parser = Parser::new();
    let mut p = MockPerform::new();
    // ESC [ ; H → cursor home with empty params (parser emits single 0)
    parser.feed(b"\x1b[;H", &mut p);
    assert_eq!(p.events, vec![Event::Csi(vec![0], b'H')]);
}

// ── ESC sequences ────────────────────────────────────────────────

#[test]
fn test_esc_single_char() {
    let mut parser = Parser::new();
    let mut p = MockPerform::new();
    // ESC D → IND (index)
    parser.feed(b"\x1bD", &mut p);
    assert_eq!(p.events, vec![Event::Esc(b'D')]);
}

#[test]
fn test_esc_7() {
    let mut parser = Parser::new();
    let mut p = MockPerform::new();
    // ESC 7 → save cursor
    parser.feed(b"\x1b7", &mut p);
    assert_eq!(p.events, vec![Event::Esc(b'7')]);
}

// ── OSC sequences ────────────────────────────────────────────────

#[test]
fn test_osc_bell_terminated() {
    let mut parser = Parser::new();
    let mut p = MockPerform::new();
    // ESC ] 0 ; test \x07 → set title to "test"
    parser.feed(b"\x1b]0;test\x07", &mut p);
    assert_eq!(p.events, vec![Event::Osc(b"0;test".to_vec())]);
}

#[test]
fn test_osc_string_terminated() {
    let mut parser = Parser::new();
    let mut p = MockPerform::new();
    // ESC ] 0 ; hello ESC \ → set title (ST terminated)
    parser.feed(b"\x1b]0;hello\x1b\\", &mut p);
    assert_eq!(p.events, vec![Event::Osc(b"0;hello".to_vec())]);
}

#[test]
fn test_osc_empty_payload() {
    let mut parser = Parser::new();
    let mut p = MockPerform::new();
    parser.feed(b"\x1b]\x07", &mut p);
    assert_eq!(p.events, vec![Event::Osc(vec![])]);
}

// ── Mixed sequences ──────────────────────────────────────────────

#[test]
fn test_mixed_print_and_csi() {
    let mut parser = Parser::new();
    let mut p = MockPerform::new();
    // "AB" then ESC[2J then "CD"
    parser.feed(b"AB\x1b[2JCD", &mut p);
    assert_eq!(
        p.events,
        vec![
            Event::Print(b'A'),
            Event::Print(b'B'),
            Event::Csi(vec![2], b'J'),
            Event::Print(b'C'),
            Event::Print(b'D'),
        ]
    );
}

#[test]
fn test_csi_then_print() {
    let mut parser = Parser::new();
    let mut p = MockPerform::new();
    parser.feed(b"\x1b[1mX", &mut p);
    assert_eq!(
        p.events,
        vec![Event::Csi(vec![1], b'm'), Event::Print(b'X')]
    );
}

// ── Split packets (partial writes) ───────────────────────────────

#[test]
fn test_csi_split_across_feeds() {
    let mut parser = Parser::new();
    let mut p = MockPerform::new();
    // ESC [ arrives first, then 2 J arrives second
    parser.feed(b"\x1b[", &mut p);
    assert!(p.events.is_empty()); // no callback yet
    parser.feed(b"2J", &mut p);
    assert_eq!(p.events, vec![Event::Csi(vec![2], b'J')]);
}

#[test]
fn test_osc_split_across_feeds() {
    let mut parser = Parser::new();
    let mut p = MockPerform::new();
    parser.feed(b"\x1b]0;par", &mut p);
    assert!(p.events.is_empty());
    parser.feed(b"t1\x07", &mut p);
    assert_eq!(p.events, vec![Event::Osc(b"0;part1".to_vec())]);
}

// ── Edge cases ───────────────────────────────────────────────────

#[test]
fn test_empty_input() {
    let mut parser = Parser::new();
    let mut p = MockPerform::new();
    parser.feed(b"", &mut p);
    assert!(p.events.is_empty());
}

#[test]
fn test_consecutive_csi() {
    let mut parser = Parser::new();
    let mut p = MockPerform::new();
    parser.feed(b"\x1b[31m\x1b[1m", &mut p);
    assert_eq!(
        p.events,
        vec![Event::Csi(vec![31], b'm'), Event::Csi(vec![1], b'm')]
    );
}

#[test]
fn test_csi_long_param() {
    let mut parser = Parser::new();
    let mut p = MockPerform::new();
    // Large param value
    parser.feed(b"\x1b[9999A", &mut p);
    assert_eq!(p.events, vec![Event::Csi(vec![9999], b'A')]);
}

#[test]
fn test_osc_with_long_payload() {
    let mut parser = Parser::new();
    let mut p = MockPerform::new();
    let payload = b"0;This is a very long title that tests the OSC buffer capacity";
    let mut input = vec![0x1b, b']'];
    input.extend_from_slice(payload);
    input.push(0x07);
    parser.feed(&input, &mut p);
    assert_eq!(p.events, vec![Event::Osc(payload.to_vec())]);
}

// ── ESC followed by unexpected byte ──────────────────────────────

#[test]
fn test_esc_then_printable() {
    let mut parser = Parser::new();
    let mut p = MockPerform::new();
    // ESC followed by a non-standard byte should not crash
    parser.feed(b"\x1bZ", &mut p);
    // Should produce an Esc event with 'Z' as final
    assert!(p.events.iter().any(|e| matches!(e, Event::Esc(b'Z'))));
}

// ── UTF-8 multibyte ──────────────────────────────────────────────

#[test]
fn test_utf8_2_byte() {
    let mut parser = Parser::new();
    let mut p = MockPerform::new();
    // 'é' = 0xC3 0xA9 in UTF-8
    parser.feed(&[0xC3, 0xA9], &mut p);
    // Parser should feed both bytes as print
    assert_eq!(p.events.len(), 2); // two print bytes
    assert!(p.events.iter().all(|e| matches!(e, Event::Print(_))));
}

#[test]
fn test_utf8_3_byte() {
    let mut parser = Parser::new();
    let mut p = MockPerform::new();
    // '中' = 0xE4 0xB8 0xAD
    parser.feed(&[0xE4, 0xB8, 0xAD], &mut p);
    assert_eq!(p.events.len(), 3);
    assert!(p.events.iter().all(|e| matches!(e, Event::Print(_))));
}

#[test]
fn test_utf8_4_byte_emoji() {
    let mut parser = Parser::new();
    let mut p = MockPerform::new();
    // '🎉' = 0xF0 0x9F 0x8E 0x89
    parser.feed(&[0xF0, 0x9F, 0x8E, 0x89], &mut p);
    assert_eq!(p.events.len(), 4);
    assert!(p.events.iter().all(|e| matches!(e, Event::Print(_))));
}
