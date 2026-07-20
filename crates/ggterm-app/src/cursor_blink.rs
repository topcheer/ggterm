//! Cursor blink animation state.
//!
//! Tracks elapsed time to modulate cursor alpha for blink animation.
//! The blink cycle is ~1 second: 500ms visible, 500ms hidden,
//! matching standard terminal cursor blink behavior.

use std::time::{Duration, Instant};

/// Duration of one blink phase (visible or hidden).
const BLINK_PHASE_MS: u64 = 500;

/// Duration of the copy/paste visual feedback flash.
const FEEDBACK_DURATION_MS: u64 = 200;

/// Tracks cursor blink timing for smooth animation.
#[derive(Debug, Clone)]
pub struct CursorBlink {
    /// When the current blink cycle started.
    cycle_start: Instant,
    /// Whether blink is enabled (cursor style is Blink*).
    enabled: bool,
}

impl Default for CursorBlink {
    fn default() -> Self {
        Self {
            cycle_start: Instant::now(),
            enabled: true,
        }
    }
}

impl CursorBlink {
    /// Create a new cursor blink tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Enable or disable blinking (based on cursor style).
    pub fn set_enabled(&mut self, enabled: bool) {
        if self.enabled != enabled {
            self.enabled = enabled;
            self.cycle_start = Instant::now();
        }
    }

    /// Returns true if the cursor should be visible in this frame.
    ///
    /// For blinking cursors: alternates true/false every 500ms.
    /// For steady cursors: always true.
    pub fn is_visible(&self) -> bool {
        if !self.enabled {
            return true;
        }
        let elapsed = self.cycle_start.elapsed().as_millis() as u64;
        let phase = elapsed % (BLINK_PHASE_MS * 2);
        phase < BLINK_PHASE_MS
    }

    /// Returns a smooth alpha value (0.0 to 1.0) for cursor rendering.
    ///
    /// Uses a sine wave for smooth fade in/out.
    /// For steady cursors, returns 1.0.
    /// When `focused` is false, returns a dim steady value (no blinking).
    pub fn alpha_focused(&self, focused: bool) -> f32 {
        if !self.enabled {
            return 1.0;
        }
        if !focused {
            return 0.5;
        }
        let elapsed = self.cycle_start.elapsed().as_secs_f32();
        let period = (BLINK_PHASE_MS * 2) as f32 / 1000.0;
        let phase = (elapsed % period) / period;
        let raw = (std::f32::consts::TAU * phase).cos() * 0.5 + 0.5;
        0.15 + raw * 0.85
    }

    /// Returns a smooth alpha value (0.0 to 1.0) for cursor rendering.
    /// Assumes window is focused (blinking active).
    pub fn alpha(&self) -> f32 {
        self.alpha_focused(true)
    }

    /// Reset the blink cycle (e.g., on user input — cursor becomes solid).
    pub fn reset(&mut self) {
        self.cycle_start = Instant::now();
    }

    /// Check if blink is currently enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

/// Visual feedback for clipboard operations (copy/paste).
///
/// When triggered, shows a brief green flash on the terminal border
/// for ~200ms to confirm the operation succeeded.
#[derive(Debug, Clone, Default)]
pub struct ClipboardFeedback {
    /// When the feedback started.
    start: Option<Instant>,
}

impl ClipboardFeedback {
    /// Create a new clipboard feedback tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Trigger the feedback flash.
    pub fn trigger(&mut self) {
        self.start = Some(Instant::now());
    }

    /// Returns the alpha intensity (0.0 to 1.0) of the flash.
    /// Returns 0.0 when no feedback is active.
    pub fn intensity(&self) -> f32 {
        match self.start {
            Some(t) => {
                let elapsed = t.elapsed();
                if elapsed > Duration::from_millis(FEEDBACK_DURATION_MS) {
                    0.0
                } else {
                    // Fade out: start at 1.0, go to 0.0
                    let progress = elapsed.as_secs_f32()
                        / Duration::from_millis(FEEDBACK_DURATION_MS).as_secs_f32();
                    1.0 - progress
                }
            }
            None => 0.0,
        }
    }

    /// Whether the feedback is currently active (non-zero intensity).
    pub fn is_active(&self) -> bool {
        self.intensity() > 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_cursor_blink_default() {
        let cb = CursorBlink::new();
        assert!(cb.is_enabled());
        // At time 0, should be visible
        assert!(cb.is_visible());
        // Alpha should be near 1.0 at start
        assert!(cb.alpha() > 0.5);
    }

    #[test]
    fn t_cursor_blink_disabled() {
        let mut cb = CursorBlink::new();
        cb.set_enabled(false);
        assert!(!cb.is_enabled());
        // Always visible when disabled
        assert!(cb.is_visible());
        assert!((cb.alpha() - 1.0).abs() < 0.001);
    }

    #[test]
    fn t_cursor_blink_alpha_range() {
        let cb = CursorBlink::new();
        let alpha = cb.alpha();
        // Alpha should be between 0.15 and 1.0
        assert!(alpha >= 0.15);
        assert!(alpha <= 1.0);
    }

    #[test]
    fn t_cursor_blink_reset() {
        let mut cb = CursorBlink::new();
        cb.reset();
        // After reset, cycle_start is now
        assert!(cb.is_visible());
    }

    #[test]
    fn t_cursor_blink_toggle() {
        let mut cb = CursorBlink::new();
        assert!(cb.is_enabled());
        cb.set_enabled(false);
        assert!(!cb.is_enabled());
        cb.set_enabled(true);
        assert!(cb.is_enabled());
    }

    #[test]
    fn t_clipboard_feedback_default_inactive() {
        let cf = ClipboardFeedback::new();
        assert!(!cf.is_active());
        assert_eq!(cf.intensity(), 0.0);
    }

    #[test]
    fn t_clipboard_feedback_trigger() {
        let mut cf = ClipboardFeedback::new();
        cf.trigger();
        assert!(cf.is_active());
        assert!(cf.intensity() > 0.0);
    }

    #[test]
    fn t_clipboard_feedback_fade() {
        let mut cf = ClipboardFeedback::new();
        cf.trigger();
        // Immediately after trigger, intensity should be near 1.0
        let i1 = cf.intensity();
        // It should be positive
        assert!(i1 > 0.0);
    }

    #[test]
    fn t_clipboard_feedback_range() {
        let cf = ClipboardFeedback::new();
        // Without trigger, intensity is 0
        assert_eq!(cf.intensity(), 0.0);

        let mut cf2 = ClipboardFeedback::new();
        cf2.trigger();
        // With trigger, intensity should be in [0, 1]
        let i = cf2.intensity();
        assert!((0.0..=1.0).contains(&i));
    }
}
