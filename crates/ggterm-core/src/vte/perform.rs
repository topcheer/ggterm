//! Perform trait — callback interface for VTE parser.

/// Trait for handling parsed terminal output.
///
/// Each method corresponds to a category of terminal data.
/// Default implementations are no-ops, so implementors only
/// need to override the methods they care about.
///
/// # Categories
///
/// - `print`: Printable characters (0x20-0x7E, plus UTF-8 sequences)
/// - `execute`: Control characters (C0/C1: BEL, BS, HT, LF, CR, etc.)
/// - `csi`: Control Sequence Introducer (`ESC [` ... final)
/// - `esc`: Miscellaneous escape sequences (`ESC` + intermediate + final)
/// - `osc`: Operating System Commands (`ESC ]` ... `BEL` / `ST`)
pub trait Perform {
    /// Printable character received (single ASCII byte or UTF-8 byte).
    fn print(&mut self, _byte: u8) {}

    /// C0/C1 control character received (e.g. BEL=0x07, LF=0x0a, CR=0x0d).
    fn execute(&mut self, _byte: u8) {}

    /// CSI sequence: `ESC [` params intermediates final.
    ///
    /// - `params`: semicolon-separated numeric parameters (e.g. `[1;2` → [1, 2])
    /// - `intermediates`: bytes 0x20-0x2F after params
    /// - `final_byte`: the final byte 0x40-0x7E that identifies the command
    fn csi(&mut self, _intermediates: &[u8], _params: &[u16], _final_byte: u8) {}

    /// CSI with colon sub-parameters.
    ///
    /// Called when the CSI sequence contains colon-separated sub-parameters
    /// (e.g. `CSI 4:1 m` for single underline). `subs` is parallel to `params`:
    /// `subs[i]` is the sub-parameter value for `params[i]`, or 0 if none.
    ///
    /// Default implementation delegates to `csi()` for backwards compatibility.
    fn csi_with_subs(
        &mut self,
        intermediates: &[u8],
        params: &[u16],
        _subs: &[u16],
        final_byte: u8,
    ) {
        self.csi(intermediates, params, final_byte);
    }

    /// ESC sequence: `ESC` intermediates final.
    fn esc(&mut self, _intermediates: &[u8], _final_byte: u8) {}

    /// OSC sequence: `ESC ]` data `BEL` or `ESC \`.
    /// `data` is the raw OSC payload (may contain semicolons).
    fn osc(&mut self, _data: &[u8]) {}

    /// DCS sequence: `ESC P` params intermediates final `data` `ST`.
    ///
    /// - `params`: semicolon-separated numeric parameters before the final byte
    /// - `intermediates`: bytes 0x20-0x2F between params and the final byte
    /// - `final_byte`: the final byte 0x40-0x7E that identifies the DCS command
    /// - `data`: the string payload after the final byte (before ST)
    ///
    /// Used by XTGETTCAP, Sixel, tmux passthrough, etc.
    fn dcs(&mut self, _intermediates: &[u8], _params: &[u16], _final_byte: u8, _data: &[u8]) {}
}
