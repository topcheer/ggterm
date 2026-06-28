//! Tests for the VTE parser.

use ggterm_core::vte::{Parser, Perform};

/// Test Perform that records all callbacks.
#[derive(Default)]
struct Recorder {
    prints: Vec<u8>,
    executes: Vec<u8>,
    csis: Vec<(Vec<u8>, Vec<u16>, u8)>,
    oscs: Vec<Vec<u8>>,
}

impl Perform for Recorder {
    fn print(&mut self, byte: u8) {
        self.prints.push(byte);
    }

    fn execute(&mut self, byte: u8) {
        self.executes.push(byte);
    }

    fn csi(&mut self, intermediates: &[u8], params: &[u16], final_byte: u8) {
        self.csis.push((intermediates.to_vec(), params.to_vec(), final_byte));
    }

    fn osc(&mut self, data: &[u8]) {
        self.oscs.push(data.to_vec());
    }
}

#[test]
fn test_print_plain_text() {
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    parser.feed(b"Hello", &mut rec);

    assert_eq!(&rec.prints, b"Hello");
    assert!(rec.csis.is_empty());
}

#[test]
fn test_csi_sgr_reset() {
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    parser.feed(b"\x1b[0m", &mut rec);

    assert_eq!(rec.csis.len(), 1);
    let (_, params, final_byte) = &rec.csis[0];
    assert_eq!(*final_byte, b'm');
    // Default param is 0 when not specified
    assert!(params.is_empty() || params == &[0]);
}

#[test]
fn test_csi_sgr_bold_fg() {
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    parser.feed(b"\x1b[1;31m", &mut rec);

    assert_eq!(rec.csis.len(), 1);
    let (_, params, final_byte) = &rec.csis[0];
    assert_eq!(*final_byte, b'm');
    assert_eq!(*params, vec![1, 31]);
}

#[test]
fn test_csi_cursor_up() {
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    parser.feed(b"\x1b[5A", &mut rec);

    assert_eq!(rec.csis.len(), 1);
    let (_, params, final_byte) = &rec.csis[0];
    assert_eq!(*final_byte, b'A');
    assert_eq!(*params, vec![5]);
}

#[test]
fn test_csi_default_param() {
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    // ESC [ H = cursor home, no params
    parser.feed(b"\x1b[H", &mut rec);

    assert_eq!(rec.csis.len(), 1);
    let (_, params, final_byte) = &rec.csis[0];
    assert_eq!(*final_byte, b'H');
    assert!(params.is_empty());
}

#[test]
fn test_osc_title_bel_terminated() {
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    // ESC ] 0 ; title BEL
    parser.feed(b"\x1b]0;My Title\x07", &mut rec);

    assert_eq!(rec.oscs.len(), 1);
    assert_eq!(rec.oscs[0], b"0;My Title");
}

#[test]
fn test_osc_title_st_terminated() {
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    // ESC ] 2 ; title ESC \
    parser.feed(b"\x1b]2;Title ST\x1b\\", &mut rec);

    assert_eq!(rec.oscs.len(), 1);
    assert_eq!(rec.oscs[0], b"2;Title ST");
}

#[test]
fn test_control_characters() {
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    // BEL, BS, HT, LF, CR
    parser.feed(b"\x07\x08\x09\x0a\x0d", &mut rec);

    assert_eq!(rec.executes, vec![0x07, 0x08, 0x09, 0x0a, 0x0d]);
}

#[test]
fn test_mixed_text_and_escape() {
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    // Text, then SGR red, then more text
    parser.feed(b"hi\x1b[31mbye", &mut rec);

    assert_eq!(&rec.prints, b"hibye");
    assert_eq!(rec.csis.len(), 1);
}

#[test]
fn test_multiple_csi_sequences() {
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    parser.feed(b"\x1b[2J\x1b[H\x1b[?25h", &mut rec);

    assert_eq!(rec.csis.len(), 3);
    // First: 2J (clear screen)
    let (_, p0, f0) = &rec.csis[0];
    assert_eq!(*f0, b'J');
    assert_eq!(*p0, vec![2]);
    // Second: H (cursor home)
    let (_, p1, f1) = &rec.csis[1];
    assert_eq!(*f1, b'H');
    // Third: ?25h (show cursor) — private mode with '?'
    // Note: '?' is an intermediate in this simplified parser
}

#[test]
fn test_feed_in_chunks() {
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    // Feed a CSI sequence in two chunks
    parser.feed(b"\x1b[3", &mut rec);
    parser.feed(b"1m", &mut rec);

    assert_eq!(rec.csis.len(), 1);
    let (_, params, final_byte) = &rec.csis[0];
    assert_eq!(*final_byte, b'm');
    assert_eq!(*params, vec![31]);
}

#[test]
fn test_esc_sequence() {
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    // ESC c = RIS (reset to initial state)
    parser.feed(b"\x1bc", &mut rec);

    // ESC callback should fire (we don't record it in this test struct,
    // but we verify the parser doesn't crash)
    assert!(rec.csis.is_empty());
}

#[test]
fn test_empty_params_with_semicolons() {
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    // ESC [ ; H — should parse as cursor home with empty params
    parser.feed(b"\x1b[;H", &mut rec);

    assert_eq!(rec.csis.len(), 1);
}
