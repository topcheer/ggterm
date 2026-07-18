//! P27-D: Smooth inertial scrolling for the terminal scrollback.
//!
//! Instead of jumping directly by N lines, scroll events set a target
//! offset and each frame interpolates toward it with exponential decay.
//! This gives a natural "momentum" feel similar to trackpad scrolling.

use std::time::Instant;

/// Smooth scroll state machine.
#[derive(Debug, Clone)]
pub struct SmoothScroller {
    /// Current visual scroll offset (in lines, can be fractional).
    current: f32,
    /// Target scroll offset we're interpolating toward.
    target: f32,
    /// Velocity from trackpad flick (pixels/sec equivalent in lines).
    velocity: f32,
    /// Last update timestamp.
    last_update: Option<Instant>,
    /// Whether we have pending animation.
    animating: bool,
}

impl Default for SmoothScroller {
    fn default() -> Self {
        Self {
            current: 0.0,
            target: 0.0,
            velocity: 0.0,
            last_update: None,
            animating: false,
        }
    }
}

impl SmoothScroller {
    /// How much integer scroll to apply this frame (and in which direction).
    /// Returns the delta in lines (positive = scroll up/older, negative = down/newer).
    /// Returns None when there's no movement.
    pub fn tick(&mut self) -> Option<i32> {
        if !self.animating {
            return None;
        }

        let now = Instant::now();
        let dt = self
            .last_update
            .map(|t| now.duration_since(t).as_secs_f32())
            .unwrap_or(0.016); // ~60fps default
        self.last_update = Some(now);

        // Exponential decay factor (higher = snappier, lower = smoother).
        // 0.15 gives a smooth ~200ms settle time at 60fps.
        let decay = 0.15_f32;
        let alpha = 1.0 - (-decay / dt.max(0.001)).exp();

        let old = self.current;
        self.current += (self.target - self.current) * alpha;

        // Apply velocity decay.
        self.velocity *= 0.92; // friction

        // If velocity is significant, adjust target.
        if self.velocity.abs() > 0.1 {
            self.target += self.velocity * dt;
        }

        // Integer delta to apply to grid viewport.
        let old_int = old.floor() as i32;
        let new_int = self.current.floor() as i32;
        let delta = new_int - old_int;

        // Check if we've settled.
        let dist = (self.target - self.current).abs();
        if dist < 0.5 && self.velocity.abs() < 0.1 {
            self.animating = false;
            self.current = self.target;
        }

        if delta != 0 { Some(delta) } else { None }
    }

    /// Add a discrete line scroll (mouse wheel click).
    pub fn add_lines(&mut self, lines: i32) {
        self.target += lines as f32;
        self.animating = true;
        self.last_update = Some(Instant::now());
    }

    /// Add precise pixel-based scroll (trackpad).
    /// `pixels` is the raw scroll delta, `line_height` converts to lines.
    pub fn add_pixels(&mut self, pixels: f32, line_height: f32) {
        let lines = pixels / line_height.max(1.0);
        self.target += lines;

        // Track velocity for momentum.
        let new_vel = self.velocity * 0.3 + lines * 8.0;

        // Cancel momentum on direction reversal: if the user flicks the
        // opposite direction, don't add to existing velocity — replace it.
        // This prevents "bouncy" behavior when changing scroll direction.
        let velocity = if self.velocity.abs() > 1.0 && new_vel.signum() != self.velocity.signum() {
            new_vel * 0.5 // Dampen the reversal
        } else {
            new_vel
        };

        // Cap velocity to prevent scrolling past entire scrollback in one frame.
        // Max ~30 lines/frame at 60fps = ~1800 lines/sec.
        self.velocity = velocity.clamp(-30.0, 30.0);

        self.animating = true;
        self.last_update = Some(Instant::now());
    }

    /// Whether we have ongoing animation (need redraw).
    pub fn is_animating(&self) -> bool {
        self.animating
    }

    /// Snap to target immediately (no animation).
    pub fn snap(&mut self) {
        self.current = self.target;
        self.velocity = 0.0;
        self.animating = false;
    }

    /// Reset to zero (e.g. when new output arrives).
    pub fn reset(&mut self) {
        self.current = 0.0;
        self.target = 0.0;
        self.velocity = 0.0;
        self.animating = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_default_not_animating() {
        let mut s = SmoothScroller::default();
        assert!(!s.is_animating());
        assert_eq!(s.tick(), None);
    }

    #[test]
    fn t_add_lines_starts_animation() {
        let mut s = SmoothScroller::default();
        s.add_lines(5);
        assert!(s.is_animating());
    }

    #[test]
    fn t_snap_immediately() {
        let mut s = SmoothScroller::default();
        s.add_lines(10);
        s.snap();
        assert!(!s.is_animating());
        assert_eq!(s.tick(), None);
    }

    #[test]
    fn t_reset_clears() {
        let mut s = SmoothScroller::default();
        s.add_lines(10);
        s.reset();
        assert!(!s.is_animating());
        assert_eq!(s.current, 0.0);
        assert_eq!(s.target, 0.0);
    }

    #[test]
    fn t_add_pixels_accumulates() {
        let mut s = SmoothScroller::default();
        s.add_pixels(32.0, 16.0); // 2 lines
        assert!(s.is_animating());
        assert!((s.target - 2.0).abs() < 0.01);
    }

    #[test]
    fn t_tick_returns_none_when_settled() {
        let mut s = SmoothScroller::default();
        s.add_lines(1);
        s.snap();
        assert_eq!(s.tick(), None);
    }

    #[test]
    fn test_tick_eventually_settles() {
        let mut s = SmoothScroller::default();
        s.add_lines(10);
        // After enough ticks, should settle.
        for _ in 0..200 {
            let _ = s.tick();
        }
        assert!(!s.is_animating());
    }

    #[test]
    fn t_negative_lines() {
        let mut s = SmoothScroller::default();
        s.add_lines(-5);
        assert!(s.is_animating());
        assert_eq!(s.target, -5.0);
    }
}
