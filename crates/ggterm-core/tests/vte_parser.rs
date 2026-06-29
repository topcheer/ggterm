//! Tests for the VTE parser (enhanced: UTF-8, DEC private mode, CsiIntermediate, ESC sequences).

use ggterm_core::vte::{Parser, Perform};

/// Test Perform that records all callbacks.
#[derive(Default)]
struct Recorder {
    prints: Vec<u8>,
    executes: Vec<u8>,
    csis: Vec<(Vec<u8>, Vec<u16>, u8)>, // (intermediates, params, final)
    escs: Vec<(Vec<u8>, u8)>,           // (intermediates, final)
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
        self.csis
            .push((intermediates.to_vec(), params.to_vec(), final_byte));
    }

    fn esc(&mut self, intermediates: &[u8], final_byte: u8) {
        self.escs.push((intermediates.to_vec(), final_byte));
    }

    fn osc(&mut self, data: &[u8]) {
        self.oscs.push(data.to_vec());
    }
}

// ===========================================================================
// Original tests (updated for new esc() recording)
// ===========================================================================

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
    let (i0, p0, f0) = &rec.csis[0];
    assert_eq!(*f0, b'J');
    assert_eq!(*p0, vec![2]);
    assert!(i0.is_empty());
    // Second: H (cursor home)
    let (_, _p1, f1) = &rec.csis[1];
    assert_eq!(*f1, b'H');
    // Third: ?25h (show cursor) — private mode, '?' is intermediate
    let (i2, p2, f2) = &rec.csis[2];
    assert_eq!(*f2, b'h');
    assert_eq!(*p2, vec![25]);
    assert_eq!(*i2, vec![b'?'], "'?' should be in intermediates");
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
fn test_empty_params_with_semicolons() {
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    // ESC [ ; H — should parse as cursor home with empty params
    parser.feed(b"\x1b[;H", &mut rec);

    assert_eq!(rec.csis.len(), 1);
}

// ===========================================================================
// DEC Private Mode tests ('?' prefix)
// ===========================================================================

#[test]
fn test_dec_private_show_cursor() {
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    parser.feed(b"\x1b[?25h", &mut rec);

    assert_eq!(rec.csis.len(), 1);
    let (inter, params, final_byte) = &rec.csis[0];
    assert_eq!(*final_byte, b'h');
    assert_eq!(*params, vec![25]);
    assert_eq!(*inter, vec![b'?'], "'?' should be captured as intermediate");
}

#[test]
fn test_dec_private_hide_cursor() {
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    parser.feed(b"\x1b[?25l", &mut rec);

    assert_eq!(rec.csis.len(), 1);
    let (inter, params, final_byte) = &rec.csis[0];
    assert_eq!(*final_byte, b'l');
    assert_eq!(*params, vec![25]);
    assert_eq!(*inter, vec![b'?']);
}

#[test]
fn test_dec_private_alt_screen() {
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    parser.feed(b"\x1b[?1049h", &mut rec);

    assert_eq!(rec.csis.len(), 1);
    let (inter, params, final_byte) = &rec.csis[0];
    assert_eq!(*final_byte, b'h');
    assert_eq!(*params, vec![1049]);
    assert_eq!(*inter, vec![b'?']);
}

#[test]
fn test_dec_private_bracketed_paste() {
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    parser.feed(b"\x1b[?2004h", &mut rec);

    assert_eq!(rec.csis.len(), 1);
    let (inter, params, final_byte) = &rec.csis[0];
    assert_eq!(*final_byte, b'h');
    assert_eq!(*params, vec![2004]);
    assert_eq!(*inter, vec![b'?']);
}

#[test]
fn test_dec_private_cursor_blink() {
    // ESC[?12h — enable cursor blink
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    parser.feed(b"\x1b[?12h\x1b[?12l", &mut rec);

    assert_eq!(rec.csis.len(), 2);
    // Enable
    let (i0, p0, f0) = &rec.csis[0];
    assert_eq!(*f0, b'h');
    assert_eq!(*p0, vec![12]);
    assert_eq!(*i0, vec![b'?']);
    // Disable
    let (i1, p1, f1) = &rec.csis[1];
    assert_eq!(*f1, b'l');
    assert_eq!(*p1, vec![12]);
    assert_eq!(*i1, vec![b'?']);
}

#[test]
fn test_dec_private_multiple_modes() {
    // Multiple DEC private mode sequences in sequence
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    parser.feed(b"\x1b[?25h\x1b[?1049h\x1b[?2004h", &mut rec);

    assert_eq!(rec.csis.len(), 3);
    for (_, _, final_byte) in &rec.csis {
        assert_eq!(*final_byte, b'h');
    }
    let (_, p0, _) = &rec.csis[0];
    let (_, p1, _) = &rec.csis[1];
    let (_, p2, _) = &rec.csis[2];
    assert_eq!(*p0, vec![25]);
    assert_eq!(*p1, vec![1049]);
    assert_eq!(*p2, vec![2004]);
}

// ===========================================================================
// Other private mode prefixes (<, >, =)
// ===========================================================================

#[test]
fn test_csi_gt_prefix() {
    // ESC[>0c — request device attributes (some terminals use '>' prefix)
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    parser.feed(b"\x1b[>0c", &mut rec);

    assert_eq!(rec.csis.len(), 1);
    let (inter, params, final_byte) = &rec.csis[0];
    assert_eq!(*final_byte, b'c');
    assert_eq!(*params, vec![0]);
    assert_eq!(*inter, vec![b'>'], "'>' should be in intermediates");
}

#[test]
fn test_csi_lt_prefix() {
    // ESC[<0M — SGR mouse tracking (some terminals use '<' prefix)
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    parser.feed(b"\x1b[<M", &mut rec);

    assert_eq!(rec.csis.len(), 1);
    let (inter, _params, final_byte) = &rec.csis[0];
    assert_eq!(*final_byte, b'M');
    assert_eq!(*inter, vec![b'<'], "'<' should be in intermediates");
}

#[test]
fn test_csi_eq_prefix() {
    // ESC[=15h — set mode (legacy ANSI)
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    parser.feed(b"\x1b[=15h", &mut rec);

    assert_eq!(rec.csis.len(), 1);
    let (inter, params, final_byte) = &rec.csis[0];
    assert_eq!(*final_byte, b'h');
    assert_eq!(*params, vec![15]);
    assert_eq!(*inter, vec![b'='], "'=' should be in intermediates");
}

// ===========================================================================
// UTF-8 multi-byte character tests
// ===========================================================================

#[test]
fn test_utf8_two_byte() {
    // é = U+00E9 = C3 A9
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    parser.feed(&[0xc3, 0xa9], &mut rec);

    // Parser passes each byte individually to print()
    assert_eq!(rec.prints, vec![0xc3, 0xa9]);
}

#[test]
fn test_utf8_three_byte() {
    // 中 = U+4E2D = E4 B8 AD
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    parser.feed(&[0xe4, 0xb8, 0xad], &mut rec);

    assert_eq!(rec.prints, vec![0xe4, 0xb8, 0xad]);
}

#[test]
fn test_utf8_four_byte_emoji() {
    // 😀 = U+1F600 = F0 9F 98 80
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    parser.feed(&[0xf0, 0x9f, 0x98, 0x80], &mut rec);

    assert_eq!(rec.prints, vec![0xf0, 0x9f, 0x98, 0x80]);
}

#[test]
fn test_utf8_mixed_with_ascii() {
    // "A中B"
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    parser.feed(b"A\xe4\xb8\xadB", &mut rec);

    assert_eq!(rec.prints, vec![b'A', 0xe4, 0xb8, 0xad, b'B']);
}

#[test]
fn test_utf8_multiple_chars() {
    // "你好" = E4 BD A0 E5 A5 BD
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    parser.feed(&[0xe4, 0xbd, 0xa0, 0xe5, 0xa5, 0xbd], &mut rec);

    assert_eq!(rec.prints, vec![0xe4, 0xbd, 0xa0, 0xe5, 0xa5, 0xbd]);
}

#[test]
fn test_utf8_invalid_leading_byte() {
    // 0xFE is not a valid UTF-8 leading byte — treated as single byte
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    parser.feed(&[0xfe], &mut rec);

    assert_eq!(rec.prints, vec![0xfe]);
}

#[test]
fn test_utf8_truncated_sequence() {
    // Start a 2-byte sequence (0xC3) then send a non-continuation byte ('A')
    // Parser should print the accumulated byte(s) as-is, then process 'A' normally
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    parser.feed(&[0xc3, b'A'], &mut rec);

    // 0xC3 printed as fallback, then 'A' printed normally
    assert_eq!(rec.prints, vec![0xc3, b'A']);
}

#[test]
fn test_utf8_truncated_three_byte() {
    // Start a 3-byte sequence, send only 1 continuation, then non-continuation
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    // E4 B8 'X' — E4 B8 accumulated, then X breaks it
    parser.feed(&[0xe4, 0xb8, b'X'], &mut rec);

    // Two bytes printed as fallback, then X
    assert_eq!(rec.prints, vec![0xe4, 0xb8, b'X']);
}

#[test]
fn test_utf8_feed_in_chunks() {
    // Feed a 3-byte UTF-8 char one byte at a time
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    parser.feed(&[0xe4], &mut rec);
    parser.feed(&[0xb8], &mut rec);
    parser.feed(&[0xad], &mut rec);

    assert_eq!(rec.prints, vec![0xe4, 0xb8, 0xad]);
}

// ===========================================================================
// ESC sequence tests
// ===========================================================================

#[test]
fn test_esc_ris_reset() {
    // ESC c = RIS (Reset to Initial State)
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    parser.feed(b"\x1bc", &mut rec);

    assert_eq!(rec.escs.len(), 1);
    let (inter, final_byte) = &rec.escs[0];
    assert_eq!(*final_byte, b'c');
    assert!(inter.is_empty());
    assert!(rec.csis.is_empty());
}

#[test]
fn test_esc_decsc_save_cursor() {
    // ESC 7 = DECSC (Save Cursor)
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    parser.feed(b"\x1b7", &mut rec);

    assert_eq!(rec.escs.len(), 1);
    let (inter, final_byte) = &rec.escs[0];
    assert_eq!(*final_byte, b'7');
    assert!(inter.is_empty());
}

#[test]
fn test_esc_decrc_restore_cursor() {
    // ESC 8 = DECRC (Restore Cursor)
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    parser.feed(b"\x1b8", &mut rec);

    assert_eq!(rec.escs.len(), 1);
    let (inter, final_byte) = &rec.escs[0];
    assert_eq!(*final_byte, b'8');
    assert!(inter.is_empty());
}

#[test]
fn test_esc_deckpam() {
    // ESC = = DECKPAM (Application Keypad Mode)
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    parser.feed(b"\x1b=", &mut rec);

    assert_eq!(rec.escs.len(), 1);
    let (inter, final_byte) = &rec.escs[0];
    assert_eq!(*final_byte, b'=');
    assert!(inter.is_empty());
}

#[test]
fn test_esc_deckpnm() {
    // ESC > = DECKPNM (Normal Keypad Mode)
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    parser.feed(b"\x1b>", &mut rec);

    assert_eq!(rec.escs.len(), 1);
    let (inter, final_byte) = &rec.escs[0];
    assert_eq!(*final_byte, b'>');
    assert!(inter.is_empty());
}

#[test]
fn test_esc_ri_reverse_index() {
    // ESC M = RI (Reverse Index — move cursor up, scroll down if at top)
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    parser.feed(b"\x1bM", &mut rec);

    assert_eq!(rec.escs.len(), 1);
    let (inter, final_byte) = &rec.escs[0];
    assert_eq!(*final_byte, b'M');
    assert!(inter.is_empty());
}

#[test]
fn test_esc_index() {
    // ESC D = IND (Index — move cursor down, scroll up if at bottom)
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    parser.feed(b"\x1bD", &mut rec);

    assert_eq!(rec.escs.len(), 1);
    let (_, final_byte) = &rec.escs[0];
    assert_eq!(*final_byte, b'D');
}

#[test]
fn test_esc_nel() {
    // ESC E = NEL (Next Line)
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    parser.feed(b"\x1bE", &mut rec);

    assert_eq!(rec.escs.len(), 1);
    let (_, final_byte) = &rec.escs[0];
    assert_eq!(*final_byte, b'E');
}

#[test]
fn test_esc_hts() {
    // ESC H = HTS (Horizontal Tab Set)
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    parser.feed(b"\x1bH", &mut rec);

    assert_eq!(rec.escs.len(), 1);
    let (_, final_byte) = &rec.escs[0];
    assert_eq!(*final_byte, b'H');
}

#[test]
fn test_esc_multiple_sequences() {
    // Save, move, restore: ESC 7 ESC D ESC 8
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    parser.feed(b"\x1b7\x1bD\x1b8", &mut rec);

    assert_eq!(rec.escs.len(), 3);
    assert_eq!(rec.escs[0].1, b'7');
    assert_eq!(rec.escs[1].1, b'D');
    assert_eq!(rec.escs[2].1, b'8');
}

#[test]
fn test_esc_does_not_interfere_with_print() {
    // Text before and after ESC sequence
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    parser.feed(b"AB\x1b7CD", &mut rec);

    assert_eq!(&rec.prints, b"ABCD");
    assert_eq!(rec.escs.len(), 1);
    assert_eq!(rec.escs[0].1, b'7');
}

// ===========================================================================
// CsiIntermediate tests (intermediate bytes between params and final)
// ===========================================================================

#[test]
fn test_csi_soft_reset() {
    // ESC[!p = DECSTR (Soft Terminal Reset)
    // '!' (0x21) is an intermediate byte, 'p' is the final byte
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    parser.feed(b"\x1b[!p", &mut rec);

    assert_eq!(rec.csis.len(), 1);
    let (inter, params, final_byte) = &rec.csis[0];
    assert_eq!(*final_byte, b'p');
    assert_eq!(*inter, vec![b'!'], "'!' should be in intermediates");
    assert!(params.is_empty(), "no params for ESC[!p");
}

#[test]
fn test_csi_intermediate_space() {
    // ESC[0 q — set cursor style (space is intermediate)
    // ' ' (0x20) is intermediate, 'q' is final
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    parser.feed(b"\x1b[0 q", &mut rec);

    assert_eq!(rec.csis.len(), 1);
    let (inter, params, final_byte) = &rec.csis[0];
    assert_eq!(*final_byte, b'q');
    assert_eq!(*inter, vec![b' '], "space should be in intermediates");
    assert_eq!(*params, vec![0]);
}

#[test]
fn test_csi_intermediate_with_params() {
    // ESC[1;2!p — soft reset with params (unusual but valid parsing)
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    parser.feed(b"\x1b[1;2!p", &mut rec);

    assert_eq!(rec.csis.len(), 1);
    let (inter, params, final_byte) = &rec.csis[0];
    assert_eq!(*final_byte, b'p');
    assert_eq!(*inter, vec![b'!']);
    assert_eq!(*params, vec![1, 2]);
}

#[test]
fn test_csi_multiple_intermediates() {
    // ESC[ $ } — two intermediate bytes ($=0x24, }=0x7D final)
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    parser.feed(b"\x1b[$}", &mut rec);

    assert_eq!(rec.csis.len(), 1);
    let (inter, _params, final_byte) = &rec.csis[0];
    assert_eq!(*final_byte, b'}');
    // Only first 2 intermediates stored (parser caps at 2)
    assert!(!inter.is_empty());
    assert_eq!(inter[0], b'$');
}

#[test]
fn test_csi_intermediate_quote() {
    // ESC["q — set selection clipboard (some terminals)
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    parser.feed(b"\x1b[\"q", &mut rec);

    assert_eq!(rec.csis.len(), 1);
    let (inter, _params, final_byte) = &rec.csis[0];
    assert_eq!(*final_byte, b'q');
    assert_eq!(*inter, vec![b'"']);
}

// ===========================================================================
// Edge case tests
// ===========================================================================

#[test]
fn test_empty_sgr_reset() {
    // ESC[m with no params = SGR reset (same as ESC[0m)
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    parser.feed(b"\x1b[m", &mut rec);

    assert_eq!(rec.csis.len(), 1);
    let (_, params, final_byte) = &rec.csis[0];
    assert_eq!(*final_byte, b'm');
    assert!(params.is_empty());
}

#[test]
fn test_large_param_saturating() {
    // ESC[999999m — u16 max is 65535, should saturate
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    parser.feed(b"\x1b[999999m", &mut rec);

    assert_eq!(rec.csis.len(), 1);
    let (_, params, final_byte) = &rec.csis[0];
    assert_eq!(*final_byte, b'm');
    assert_eq!(params.len(), 1);
    // Saturating arithmetic caps at u16::MAX = 65535
    assert_eq!(params[0], 65535, "large param should saturate to u16::MAX");
}

#[test]
fn test_large_param_multi_saturating() {
    // Two large params
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    parser.feed(b"\x1b[999999;999999m", &mut rec);

    assert_eq!(rec.csis.len(), 1);
    let (_, params, _) = &rec.csis[0];
    assert_eq!(params.len(), 2);
    assert_eq!(params[0], 65535);
    assert_eq!(params[1], 65535);
}

#[test]
fn test_multi_semicolon_empty_params() {
    // ESC[1;;4m — empty middle param (defaults to 0)
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    parser.feed(b"\x1b[1;;4m", &mut rec);

    assert_eq!(rec.csis.len(), 1);
    let (_, params, final_byte) = &rec.csis[0];
    assert_eq!(*final_byte, b'm');
    // Params: 1, (empty=0), 4
    assert_eq!(*params, vec![1, 0, 4]);
}

#[test]
fn test_many_semicolons() {
    // ESC[1;2;3;4;5m — five params
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    parser.feed(b"\x1b[1;2;3;4;5m", &mut rec);

    assert_eq!(rec.csis.len(), 1);
    let (_, params, _) = &rec.csis[0];
    assert_eq!(*params, vec![1, 2, 3, 4, 5]);
}

#[test]
fn test_trailing_semicolon() {
    // ESC[31;m — trailing semicolon does NOT add an implicit trailing 0.
    // The parser only counts params that were explicitly set, so the trailing
    // empty param after ';' is ignored in dispatch.
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    parser.feed(b"\x1b[31;m", &mut rec);

    assert_eq!(rec.csis.len(), 1);
    let (_, params, _) = &rec.csis[0];
    assert_eq!(*params, vec![31]);
}

#[test]
fn test_leading_semicolon() {
    // ESC[;31m — leading semicolon, implicit 0 then 31
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    parser.feed(b"\x1b[;31m", &mut rec);

    assert_eq!(rec.csis.len(), 1);
    let (_, params, _) = &rec.csis[0];
    assert_eq!(*params, vec![0, 31]);
}

#[test]
fn test_param_cap_at_16() {
    // Parser has a fixed-size params array of 16 — more params should not crash
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    // 20 params: 1;2;3;...;20
    let input: Vec<u8> = format!(
        "\x1b[{}m",
        (1..=20)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(";")
    )
    .into_bytes();
    parser.feed(&input, &mut rec);

    assert_eq!(rec.csis.len(), 1);
    let (_, params, _) = &rec.csis[0];
    // Parser caps at 16 params
    assert!(params.len() <= 16, "params should not exceed 16 entries");
}

#[test]
fn test_control_char_during_csi() {
    // Control character (like BEL) during CSI param accumulation.
    // The parser's `_` fallthrough arm transitions to Ground without executing
    // the control byte — this aborts the CSI sequence.
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    parser.feed(b"\x1b[1\x07m", &mut rec);

    // BEL is NOT executed (parser aborts CSI on unexpected byte)
    assert!(rec.executes.is_empty());
    // CSI sequence is aborted, so 'm' is printed as plain text
    assert!(rec.csis.is_empty());
}

#[test]
fn test_backslash_after_esc_not_st() {
    // ESC followed by non-CSI/OSC/non-intermediate should go to ground
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    parser.feed(b"\x1b\\", &mut rec);

    // ESC \ is ST string terminator, but outside OSC context it's just an ESC sequence
    // Parser treats \ (0x5C, which is in 0x40-0x7E) as a final byte
    assert_eq!(rec.escs.len(), 1);
    let (_, final_byte) = &rec.escs[0];
    assert_eq!(*final_byte, b'\\');
}

// ===========================================================================
// Integration: realistic terminal output
// ===========================================================================

#[test]
fn test_realistic_terminal_sequence() {
    // Simulate a typical terminal session:
    // 1. Clear screen + home
    // 2. Hide cursor
    // 3. Print text
    // 4. Set colors (bold + red fg)
    // 5. Print colored text
    // 6. Reset SGR
    // 7. Show cursor
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    parser.feed(
        b"\x1b[2J\x1b[H\x1b[?25lHello\x1b[1;31mWorld\x1b[0m\x1b[?25h",
        &mut rec,
    );

    // CSIs: 2J, H, ?25l, 1;31m, 0m, ?25h = 6 CSI sequences
    assert_eq!(rec.csis.len(), 6);

    // Verify DEC private mode sequences
    let (i2, _, f2) = &rec.csis[2]; // ?25l
    assert_eq!(*f2, b'l');
    assert_eq!(*i2, vec![b'?']);

    let (i5, _, f5) = &rec.csis[5]; // ?25h
    assert_eq!(*f5, b'h');
    assert_eq!(*i5, vec![b'?']);

    // Text printed: "HelloWorld"
    assert_eq!(&rec.prints, b"HelloWorld");
}

#[test]
fn test_vim_escape_sequences() {
    // Vim uses many escape sequences — test a few common ones
    let mut parser = Parser::new();
    let mut rec = Recorder::default();

    // Enter alt screen, clear, set title, print
    parser.feed(b"\x1b[?1049h\x1b[2J\x1b[H\x1b]2;Vim\x07Ready", &mut rec);

    assert_eq!(rec.csis.len(), 3); // ?1049h, 2J, H
    assert_eq!(rec.oscs.len(), 1); // title
    assert_eq!(rec.oscs[0], b"2;Vim");
    assert_eq!(&rec.prints, b"Ready");
}
