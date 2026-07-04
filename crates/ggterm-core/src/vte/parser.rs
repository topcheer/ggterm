//! VTE parser based on the Paul Williams ANSI parser state machine.
//!
//! Reference: https://vt100.net/emu/dec_ansi_parser
//!
//! The parser is a byte-driven state machine. Each input byte causes a
//! state transition and/or invokes a callback on the [`Perform`] trait.

use super::perform::Perform;

/// VTE parser state machine.
///
/// Feed raw bytes via [`Parser::feed`], and callbacks will be invoked on
/// the provided [`Perform`] implementation.
///
/// # States
///
/// - `Ground`: Normal input (printable chars, control chars, ESC)
/// - `Escape`: After ESC byte
/// - `CsiEntry`: After `ESC [`
/// - `CsiParam`: Accumulating numeric parameters
/// - `CsiIntermediate`: After params, collecting intermediate bytes
/// - `OscString`: Inside `ESC ]` ... `BEL`/`ST`
/// - `Utf8Sequence`: Accumulating a multi-byte UTF-8 character
pub struct Parser {
    state: State,
    /// Accumulated intermediate bytes (0x20-0x2F).
    intermediates: [u8; 2],
    intermediate_count: usize,
    /// Accumulated parameters for CSI/DCS sequences.
    params: [u16; 16],
    param_count: usize,
    /// Sub-parameters for the current parameter (colon-separated).
    /// e.g. `4:1` stores sub=1 for the current param.
    /// We pack the sub-value into the high byte of params: params[i] = main | (sub << 8).
    /// A sub of 0 means no sub-parameter was given (default).
    param_sub: [u16; 16],
    /// True if the current parameter has been explicitly set.
    param_set: bool,
    /// True if currently collecting a colon sub-parameter value.
    in_sub_param: bool,
    /// Accumulator for OSC string data.
    string_buffer: Vec<u8>,
    /// DCS final byte (set when entering DcsString).
    dcs_final: u8,
    /// UTF-8 decoding state.
    utf8_buf: [u8; 4],
    utf8_len: usize,
    utf8_expected: usize,
}

impl Parser {
    pub fn new() -> Self {
        Self {
            state: State::Ground,
            intermediates: [0; 2],
            intermediate_count: 0,
            params: [0; 16],
            param_count: 0,
            param_sub: [0; 16],
            param_set: false,
            in_sub_param: false,
            string_buffer: Vec::with_capacity(256),
            dcs_final: 0,
            utf8_buf: [0; 4],
            utf8_len: 0,
            utf8_expected: 0,
        }
    }

    /// Feed raw bytes to the parser, invoking callbacks on `perform`.
    pub fn feed<P: Perform>(&mut self, data: &[u8], perform: &mut P) {
        for &byte in data {
            self.advance(byte, perform);
        }
    }

    /// Process a single byte through the state machine.
    fn advance<P: Perform>(&mut self, byte: u8, perform: &mut P) {
        match self.state {
            State::Ground => self.ground(byte, perform),
            State::Escape => self.escape(byte, perform),
            State::EscapeIntermediate => self.escape_intermediate(byte, perform),
            State::CsiEntry => self.csi_entry(byte, perform),
            State::CsiParam => self.csi_param(byte, perform),
            State::CsiIntermediate => self.csi_intermediate(byte, perform),
            State::OscString => self.osc_string(byte, perform),
            State::OscEsc => {
                if byte == b'\\' {
                    perform.osc(&self.string_buffer);
                } else {
                    // Unexpected byte after ESC in OSC context; abort OSC
                }
                self.state = State::Ground;
            }
            State::Utf8Sequence => self.utf8_continue(byte, perform),
            State::DcsEntry => self.dcs_entry(byte, perform),
            State::DcsParam => self.dcs_param(byte, perform),
            State::DcsIntermediate => self.dcs_intermediate(byte, perform),
            State::DcsString => self.dcs_string(byte, perform),
            State::DcsEsc => {
                // After ESC in DCS context: ST = ESC \
                if byte != b'\\' {
                    // Not ST — could be another escape, but we just
                    // go back to consuming DCS data or ground.
                    if byte == 0x1b {
                        self.state = State::DcsEsc;
                    } else {
                        self.state = State::DcsString;
                    }
                } else {
                    // ST received — end of DCS sequence
                    self.dispatch_dcs(perform);
                    self.state = State::Ground;
                }
            }
        }
    }

    // -- State handlers ------------------------------------------------------

    fn ground<P: Perform>(&mut self, byte: u8, perform: &mut P) {
        if byte == 0x1b {
            // ESC — enter escape state
            self.state = State::Escape;
            self.reset_seq();
        } else if (0x20..=0x7e).contains(&byte) {
            // Printable ASCII
            perform.print(byte);
        } else if byte >= 0x80 {
            // Possible UTF-8 multi-byte sequence
            self.utf8_start(byte, perform);
        } else {
            // C0 control character (0x00-0x1F), except ESC handled above
            perform.execute(byte);
        }
    }

    fn escape<P: Perform>(&mut self, byte: u8, perform: &mut P) {
        match byte {
            b'[' => {
                self.state = State::CsiEntry;
                self.reset_seq();
            }
            b']' => {
                self.state = State::OscString;
                self.string_buffer.clear();
            }
            b'P' => {
                // DCS — Device Control String (ESC P ... ST)
                // Collect params/intermediates then string data until ST.
                self.reset_seq();
                self.state = State::DcsEntry;
                self.dcs_final = 0;
            }
            b'X' | b'^' | b'_' => {
                // SOS (ESC X), PM (ESC ^), APC (ESC _) — string sequences
                // that must be consumed until ST, same as DCS.
                self.state = State::DcsString;
            }
            // 0x20-0x2F: intermediate bytes
            0x20..=0x2f => {
                self.state = State::EscapeIntermediate;
                self.intermediates[0] = byte;
                self.intermediate_count = 1;
            }
            // Final byte (0x30-0x7E): dispatch ESC sequence
            0x30..=0x7e => {
                perform.esc(&[], byte);
                self.state = State::Ground;
            }
            // Control char during escape: execute and stay in escape
            _ if byte < 0x20 => {
                perform.execute(byte);
            }
            _ => {
                self.state = State::Ground;
            }
        }
    }

    fn escape_intermediate<P: Perform>(&mut self, byte: u8, perform: &mut P) {
        match byte {
            // More intermediates
            0x20..=0x2f => {
                if self.intermediate_count < self.intermediates.len() {
                    self.intermediates[self.intermediate_count] = byte;
                    self.intermediate_count += 1;
                }
            }
            // Final byte
            0x30..=0x7e => {
                let inter = unsafe { self.intermediates.get_unchecked(..self.intermediate_count) };
                perform.esc(inter, byte);
                self.state = State::Ground;
            }
            _ => {
                self.state = State::Ground;
            }
        }
    }

    fn csi_entry<P: Perform>(&mut self, byte: u8, perform: &mut P) {
        match byte {
            b'0'..=b'9' => {
                self.state = State::CsiParam;
                self.param_set = true;
                if self.param_count < self.params.len() {
                    self.params[self.param_count] = (byte - b'0') as u16;
                }
            }
            b';' => {
                self.state = State::CsiParam;
                self.param_count = self
                    .param_count
                    .saturating_add(1)
                    .min(self.params.len() - 1);
                self.param_set = false;
            }
            // Private mode prefixes: ?, <, >, = — treated as intermediates
            b'?' | b'<' | b'>' | b'=' => {
                if self.intermediate_count < self.intermediates.len() {
                    self.intermediates[self.intermediate_count] = byte;
                    self.intermediate_count += 1;
                }
                self.state = State::CsiParam;
            }
            // Intermediate bytes (0x20-0x2F)
            0x20..=0x2f => {
                if self.intermediate_count < self.intermediates.len() {
                    self.intermediates[self.intermediate_count] = byte;
                    self.intermediate_count += 1;
                }
                self.state = State::CsiIntermediate;
            }
            // Final byte (0x40-0x7E)
            0x40..=0x7e => {
                self.dispatch_csi(byte, perform);
                self.state = State::Ground;
            }
            _ => {
                self.state = State::Ground;
            }
        }
    }

    fn csi_param<P: Perform>(&mut self, byte: u8, perform: &mut P) {
        match byte {
            b'0'..=b'9' => {
                let idx = self.param_count.min(self.params.len() - 1);
                if self.in_sub_param {
                    // Colon sub-parameter: accumulate into param_sub.
                    self.param_sub[idx] = self.param_sub[idx]
                        .saturating_mul(10)
                        .saturating_add((byte - b'0') as u16);
                } else {
                    self.params[idx] = self.params[idx]
                        .saturating_mul(10)
                        .saturating_add((byte - b'0') as u16);
                }
                self.param_set = true;
            }
            b';' => {
                if self.param_count < self.params.len() - 1 {
                    self.param_count += 1;
                }
                self.param_set = false;
                self.in_sub_param = false;
            }
            // Colon sub-parameter separator (e.g. SGR 4:1 for underline style).
            b':' => {
                self.in_sub_param = true;
            }
            // Intermediate bytes
            0x20..=0x2f => {
                if self.intermediate_count < self.intermediates.len() {
                    self.intermediates[self.intermediate_count] = byte;
                    self.intermediate_count += 1;
                }
                self.state = State::CsiIntermediate;
            }
            // Final byte
            0x40..=0x7e => {
                self.dispatch_csi(byte, perform);
                self.state = State::Ground;
            }
            _ => {
                self.state = State::Ground;
            }
        }
    }

    fn csi_intermediate<P: Perform>(&mut self, byte: u8, perform: &mut P) {
        match byte {
            0x20..=0x2f => {
                if self.intermediate_count < self.intermediates.len() {
                    self.intermediates[self.intermediate_count] = byte;
                    self.intermediate_count += 1;
                }
            }
            0x40..=0x7e => {
                self.dispatch_csi(byte, perform);
                self.state = State::Ground;
            }
            _ => {
                self.state = State::Ground;
            }
        }
    }

    fn osc_string<P: Perform>(&mut self, byte: u8, perform: &mut P) {
        match byte {
            0x07 => {
                // BEL terminates OSC
                perform.osc(&self.string_buffer);
                self.state = State::Ground;
            }
            0x1b => {
                self.state = State::OscEsc;
            }
            c if c >= 0x20 => {
                // Cap OSC string at 64KB to prevent memory exhaustion from
                // malformed/malicious sequences without a terminator.
                if self.string_buffer.len() < 65536 {
                    self.string_buffer.push(byte);
                }
            }
            _ => {}
        }
    }

    // -- Helpers -------------------------------------------------------------

    // -- DCS state handlers -------------------------------------------------

    fn dcs_entry<P: Perform>(&mut self, byte: u8, _perform: &mut P) {
        match byte {
            b'0'..=b'9' => {
                self.state = State::DcsParam;
                self.param_set = true;
                if self.param_count < self.params.len() {
                    self.params[self.param_count] = (byte - b'0') as u16;
                }
            }
            b';' => {
                self.state = State::DcsParam;
                self.param_count = self
                    .param_count
                    .saturating_add(1)
                    .min(self.params.len() - 1);
                self.param_set = false;
            }
            // Intermediate bytes (0x20-0x2F)
            0x20..=0x2f => {
                if self.intermediate_count < self.intermediates.len() {
                    self.intermediates[self.intermediate_count] = byte;
                    self.intermediate_count += 1;
                }
                self.state = State::DcsIntermediate;
            }
            // Final byte (0x40-0x7E) → enter string passthrough
            0x40..=0x7e => {
                self.dcs_final = byte;
                self.string_buffer.clear();
                self.state = State::DcsString;
            }
            0x1b => {
                // Empty DCS: ESC P ST — dispatch with empty data
                self.dcs_final = 0;
                self.string_buffer.clear();
                self.state = State::DcsEsc;
            }
            _ => {
                self.state = State::Ground;
            }
        }
    }

    fn dcs_param<P: Perform>(&mut self, byte: u8, _perform: &mut P) {
        match byte {
            b'0'..=b'9' => {
                let idx = self.param_count.min(self.params.len() - 1);
                self.params[idx] = self.params[idx]
                    .saturating_mul(10)
                    .saturating_add((byte - b'0') as u16);
                self.param_set = true;
            }
            b';' => {
                if self.param_count < self.params.len() - 1 {
                    self.param_count += 1;
                }
                self.param_set = false;
            }
            0x20..=0x2f => {
                if self.intermediate_count < self.intermediates.len() {
                    self.intermediates[self.intermediate_count] = byte;
                    self.intermediate_count += 1;
                }
                self.state = State::DcsIntermediate;
            }
            0x40..=0x7e => {
                self.dcs_final = byte;
                self.string_buffer.clear();
                self.state = State::DcsString;
            }
            _ => {
                self.state = State::Ground;
            }
        }
    }

    fn dcs_intermediate<P: Perform>(&mut self, byte: u8, _perform: &mut P) {
        match byte {
            0x20..=0x2f => {
                if self.intermediate_count < self.intermediates.len() {
                    self.intermediates[self.intermediate_count] = byte;
                    self.intermediate_count += 1;
                }
            }
            0x40..=0x7e => {
                self.dcs_final = byte;
                self.string_buffer.clear();
                self.state = State::DcsString;
            }
            _ => {
                self.state = State::Ground;
            }
        }
    }

    fn dcs_string<P: Perform>(&mut self, byte: u8, perform: &mut P) {
        match byte {
            0x1b => {
                self.state = State::DcsEsc;
            }
            0x07 => {
                // BEL can also terminate DCS
                self.dispatch_dcs(perform);
                self.state = State::Ground;
            }
            c if c >= 0x20 => {
                // Cap DCS string at 1MB — Sixel images can be large.
                if self.string_buffer.len() < 1048576 {
                    self.string_buffer.push(byte);
                }
            }
            _ => {}
        }
    }

    /// Dispatch a completed DCS sequence to the Perform callback.
    fn dispatch_dcs<P: Perform>(&mut self, perform: &mut P) {
        let inter = unsafe { self.intermediates.get_unchecked(..self.intermediate_count) };
        let count = if self.param_set {
            self.param_count + 1
        } else {
            self.param_count
        };
        let n = count.min(self.params.len());
        let params = unsafe { self.params.get_unchecked(..n) };
        let final_byte = self.dcs_final;
        let data = self.string_buffer.as_slice();
        perform.dcs(inter, params, final_byte, data);
    }

    // -- CSI dispatch -------------------------------------------------------

    /// Dispatch a CSI sequence to the Perform callback.
    fn dispatch_csi<P: Perform>(&mut self, final_byte: u8, perform: &mut P) {
        let count = if self.param_set {
            self.param_count + 1
        } else {
            self.param_count
        };
        let inter = unsafe { self.intermediates.get_unchecked(..self.intermediate_count) };
        let n = count.min(self.params.len());
        let params = unsafe { self.params.get_unchecked(..n) };
        let subs = unsafe { self.param_sub.get_unchecked(..n) };
        perform.csi_with_subs(inter, params, subs, final_byte);
    }

    /// Reset sequence accumulation state.
    fn reset_seq(&mut self) {
        self.intermediate_count = 0;
        self.param_count = 0;
        self.params = [0; 16];
        self.param_sub = [0; 16];
        self.param_set = false;
        self.in_sub_param = false;
    }

    // -- UTF-8 handling ------------------------------------------------------

    fn utf8_start<P: Perform>(&mut self, byte: u8, perform: &mut P) {
        // Determine sequence length from leading byte
        let expected = if byte & 0xe0 == 0xc0 {
            2
        } else if byte & 0xf0 == 0xe0 {
            3
        } else if byte & 0xf8 == 0xf0 {
            4
        } else {
            // Invalid UTF-8 leading byte — treat as single byte
            perform.print(byte);
            return;
        };

        self.utf8_buf[0] = byte;
        self.utf8_len = 1;
        self.utf8_expected = expected;
        self.state = State::Utf8Sequence;
    }

    fn utf8_continue<P: Perform>(&mut self, byte: u8, perform: &mut P) {
        if byte & 0xc0 != 0x80 {
            // Not a continuation byte — abort, treat first byte as raw
            // Re-process this byte from Ground state
            self.state = State::Ground;
            // Print the bytes we accumulated as-is (fallback)
            for i in 0..self.utf8_len {
                perform.print(self.utf8_buf[i]);
            }
            self.utf8_len = 0;
            self.advance(byte, perform);
            return;
        }

        self.utf8_buf[self.utf8_len] = byte;
        self.utf8_len += 1;

        if self.utf8_len == self.utf8_expected {
            // Complete UTF-8 sequence — decode and print each byte
            // (Perform trait takes bytes, the Terminal reassembles)
            let buf = &self.utf8_buf[..self.utf8_len];
            for &b in buf {
                perform.print(b);
            }
            self.utf8_len = 0;
            self.state = State::Ground;
        }
    }
}

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Ground,
    Escape,
    EscapeIntermediate,
    CsiEntry,
    CsiParam,
    CsiIntermediate,
    OscString,
    OscEsc,
    /// Accumulating a multi-byte UTF-8 character.
    Utf8Sequence,
    /// DCS entry: after ESC P, collecting params/intermediates.
    DcsEntry,
    /// DCS param collection.
    DcsParam,
    /// DCS intermediate collection.
    DcsIntermediate,
    /// Inside DCS string payload — collect data until ST.
    DcsString,
    /// After ESC inside a DCS — checking for ST (ESC \).
    DcsEsc,
}
