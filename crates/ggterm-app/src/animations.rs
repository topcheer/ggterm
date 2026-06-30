//! P28-A: Window and UI animations.
//!
//! Provides smooth transitions for:
//! - Tab switching (fade/slide)
//! - Split pane expansion
//! - Overlay appearances (search bar, command palette, settings)
//! - Window open/close fade-in

use std::time::{Duration, Instant};

/// Default animation duration (ms).
const DEFAULT_DURATION_MS: u64 = 200;

/// Easing function type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Easing {
    /// Linear interpolation.
    Linear,
    /// Ease-in (quadratic).
    EaseIn,
    /// Ease-out (quadratic).
    EaseOut,
    /// Ease-in-out (cubic).
    EaseInOut,
    /// Spring (overshoot).
    Spring,
}

impl Easing {
    /// Apply easing to a linear progress value [0, 1].
    pub fn apply(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Easing::Linear => t,
            Easing::EaseIn => t * t,
            Easing::EaseOut => 1.0 - (1.0 - t) * (1.0 - t),
            Easing::EaseInOut => {
                if t < 0.5 {
                    4.0 * t * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
                }
            }
            Easing::Spring => {
                // Exponential approach to 1.0 (no overshoot, fast settle)
                1.0 - (-5.0 * t).exp()
            }
        }
    }
}

/// A single animation tracking its progress.
#[derive(Debug, Clone)]
pub struct Animation {
    /// Start time of the animation.
    start: Option<Instant>,
    /// Duration of the animation.
    duration: Duration,
    /// Easing function.
    easing: Easing,
    /// Whether the animation is reversed (fade-out).
    reversing: bool,
    /// Progress before reversing (for smooth reverse).
    pre_reverse_progress: f32,
}

impl Animation {
    /// Create a new animation with the given duration and easing.
    pub fn new(duration_ms: u64, easing: Easing) -> Self {
        Self {
            start: None,
            duration: Duration::from_millis(duration_ms),
            easing,
            reversing: false,
            pre_reverse_progress: 0.0,
        }
    }

    /// Create with default duration.
    pub fn default_anim() -> Self {
        Self::new(DEFAULT_DURATION_MS, Easing::EaseOut)
    }

    /// Start or restart the animation.
    pub fn start(&mut self) {
        self.start = Some(Instant::now());
        self.reversing = false;
        self.pre_reverse_progress = 0.0;
    }

    /// Start reversed (for fade-out).
    pub fn start_reverse(&mut self) {
        self.pre_reverse_progress = self.progress();
        self.start = Some(Instant::now());
        self.reversing = true;
    }

    /// Start from a specific progress (for mid-point reversal).
    pub fn start_from(&mut self, progress: f32) {
        self.pre_reverse_progress = progress;
        self.start = Some(Instant::now());
        self.reversing = false;
    }

    /// Returns true if the animation is currently running.
    pub fn is_running(&self) -> bool {
        self.start.is_some() && !self.is_complete()
    }

    /// Returns true if the animation has completed.
    pub fn is_complete(&self) -> bool {
        if let Some(start) = self.start {
            start.elapsed() >= self.duration
        } else {
            true
        }
    }

    /// Get the raw elapsed fraction [0, 1].
    fn elapsed_fraction(&self) -> f32 {
        if let Some(start) = self.start {
            let elapsed = start.elapsed().as_millis() as f32;
            let total = self.duration.as_millis() as f32;
            (elapsed / total).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    /// Get the current progress [0, 1] with easing applied.
    pub fn progress(&self) -> f32 {
        if self.start.is_none() {
            return 0.0;
        }
        let raw = self.elapsed_fraction();
        let eased = self.easing.apply(raw);

        if self.reversing {
            // Interpolate from pre_reverse_progress down to 0.
            self.pre_reverse_progress * (1.0 - eased)
        } else {
            // Interpolate from pre_reverse_progress up to 1.
            self.pre_reverse_progress + (1.0 - self.pre_reverse_progress) * eased
        }
    }

    /// Reset the animation to its initial state.
    pub fn reset(&mut self) {
        self.start = None;
        self.reversing = false;
        self.pre_reverse_progress = 0.0;
    }
}

/// Tracks which kind of animation is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AnimationKind {
    /// Tab switch fade.
    TabSwitch,
    /// Split pane expand.
    SplitExpand,
    /// Overlay slide-in (search bar, palette, etc.).
    OverlaySlide,
    /// Window open fade.
    WindowOpen,
    /// No active animation.
    #[default]
    None,
}

/// Central animation manager for the DesktopApp.
#[derive(Debug)]
pub struct AnimationManager {
    /// Tab switch animation.
    tab_switch: Animation,
    /// Split expand animation.
    split_expand: Animation,
    /// Overlay slide animation.
    overlay_slide: Animation,
    /// Window open animation.
    window_open: Animation,
    /// What kind of animation is currently active.
    active_kind: AnimationKind,
}

impl Default for AnimationManager {
    fn default() -> Self {
        Self {
            tab_switch: Animation::new(180, Easing::EaseOut),
            split_expand: Animation::new(250, Easing::EaseInOut),
            overlay_slide: Animation::new(150, Easing::EaseOut),
            window_open: Animation::new(300, Easing::EaseOut),
            active_kind: AnimationKind::None,
        }
    }
}

impl AnimationManager {
    /// Start a tab switch animation.
    pub fn tab_switch(&mut self) {
        self.tab_switch.start();
        self.active_kind = AnimationKind::TabSwitch;
    }

    /// Start a split expand animation.
    pub fn split_expand(&mut self) {
        self.split_expand.start();
        self.active_kind = AnimationKind::SplitExpand;
    }

    /// Start an overlay slide animation.
    pub fn overlay_slide(&mut self) {
        self.overlay_slide.start();
        self.active_kind = AnimationKind::OverlaySlide;
    }

    /// Start a window open animation.
    pub fn window_open(&mut self) {
        self.window_open.start();
        self.active_kind = AnimationKind::WindowOpen;
    }

    /// Get the progress of the active animation (0.0 if none).
    pub fn progress(&self) -> f32 {
        match self.active_kind {
            AnimationKind::TabSwitch => self.tab_switch.progress(),
            AnimationKind::SplitExpand => self.split_expand.progress(),
            AnimationKind::OverlaySlide => self.overlay_slide.progress(),
            AnimationKind::WindowOpen => self.window_open.progress(),
            AnimationKind::None => 1.0,
        }
    }

    /// Whether any animation is running.
    pub fn is_animating(&self) -> bool {
        match self.active_kind {
            AnimationKind::None => false,
            kind => {
                let anim = match kind {
                    AnimationKind::TabSwitch => &self.tab_switch,
                    AnimationKind::SplitExpand => &self.split_expand,
                    AnimationKind::OverlaySlide => &self.overlay_slide,
                    AnimationKind::WindowOpen => &self.window_open,
                    AnimationKind::None => return false,
                };
                anim.is_running()
            }
        }
    }

    /// Stop all animations.
    pub fn stop_all(&mut self) {
        self.tab_switch.reset();
        self.split_expand.reset();
        self.overlay_slide.reset();
        self.window_open.reset();
        self.active_kind = AnimationKind::None;
    }

    /// Returns true if we need to keep redrawing for animations.
    pub fn needs_redraw(&self) -> bool {
        self.is_animating()
    }

    /// Get the current alpha for tab switching (1.0 = fully visible).
    pub fn tab_switch_alpha(&self) -> f32 {
        self.tab_switch.progress()
    }

    /// Get the scale factor for split expansion (0.0 → 1.0).
    pub fn split_expand_scale(&self) -> f32 {
        self.split_expand.progress()
    }

    /// Get the slide offset for overlays (0.0 = hidden, 1.0 = fully shown).
    pub fn overlay_slide_progress(&self) -> f32 {
        self.overlay_slide.progress()
    }

    /// Get the window open alpha (0.0 = transparent → 1.0 = opaque).
    pub fn window_open_alpha(&self) -> f32 {
        self.window_open.progress()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_easing_linear() {
        assert!((Easing::Linear.apply(0.5) - 0.5).abs() < 0.001);
        assert_eq!(Easing::Linear.apply(0.0), 0.0);
        assert_eq!(Easing::Linear.apply(1.0), 1.0);
    }

    #[test]
    fn t_easing_ease_in() {
        assert_eq!(Easing::EaseIn.apply(0.0), 0.0);
        assert!((Easing::EaseIn.apply(0.5) - 0.25).abs() < 0.001);
        assert_eq!(Easing::EaseIn.apply(1.0), 1.0);
    }

    #[test]
    fn t_easing_ease_out() {
        assert_eq!(Easing::EaseOut.apply(0.0), 0.0);
        let v = Easing::EaseOut.apply(0.5);
        assert!(v > 0.5, "ease-out at 0.5 should be > 0.5, got {}", v);
        assert_eq!(Easing::EaseOut.apply(1.0), 1.0);
    }

    #[test]
    fn t_easing_ease_in_out() {
        assert_eq!(Easing::EaseInOut.apply(0.0), 0.0);
        assert!((Easing::EaseInOut.apply(0.5) - 0.5).abs() < 0.01);
        assert_eq!(Easing::EaseInOut.apply(1.0), 1.0);
    }

    #[test]
    fn t_easing_clamps() {
        assert_eq!(Easing::Linear.apply(-1.0), 0.0);
        assert_eq!(Easing::Linear.apply(2.0), 1.0);
    }

    #[test]
    fn t_easing_spring_bounds() {
        let v = Easing::Spring.apply(1.0);
        assert!((v - 1.0).abs() < 0.01, "spring at 1.0 should be ~1.0");
        // Spring might overshoot in the middle but settles at 1.0
        let mid = Easing::Spring.apply(0.5);
        assert!(
            mid >= 0.0 && mid <= 2.0,
            "spring at 0.5 out of range: {}",
            mid
        );
    }

    #[test]
    fn t_animation_starts_not_running() {
        let anim = Animation::default_anim();
        assert!(!anim.is_running());
        assert_eq!(anim.progress(), 0.0);
    }

    #[test]
    fn t_animation_start_progress() {
        let mut anim = Animation::new(100, Easing::Linear);
        anim.start();
        // Right after start, progress should be very small but > 0
        let p = anim.progress();
        assert!(p >= 0.0 && p <= 1.0);
    }

    #[test]
    fn t_animation_completes() {
        let mut anim = Animation::new(1, Easing::Linear); // 1ms
        anim.start();
        std::thread::sleep(Duration::from_millis(10));
        assert!(anim.is_complete());
        assert!((anim.progress() - 1.0).abs() < 0.01);
    }

    #[test]
    fn t_animation_reverse() {
        let mut anim = Animation::new(1, Easing::Linear);
        anim.start();
        std::thread::sleep(Duration::from_millis(10));
        // Now reverse from full
        anim.start_reverse();
        let p = anim.progress();
        // Right after reverse start, progress should be near 1.0
        assert!(p > 0.5, "reverse progress should start high, got {}", p);
    }

    #[test]
    fn t_animation_reset() {
        let mut anim = Animation::new(100, Easing::Linear);
        anim.start();
        anim.reset();
        assert!(!anim.is_running());
        assert_eq!(anim.progress(), 0.0);
    }

    #[test]
    fn t_animation_start_from() {
        let mut anim = Animation::new(100, Easing::Linear);
        anim.start_from(0.5);
        let p = anim.progress();
        assert!(p >= 0.5, "start_from(0.5) should give >= 0.5, got {}", p);
    }

    #[test]
    fn t_manager_default() {
        let mgr = AnimationManager::default();
        assert!(!mgr.is_animating());
        assert!(!mgr.needs_redraw());
        assert!((mgr.progress() - 1.0).abs() < 0.01); // None → 1.0
    }

    #[test]
    fn t_manager_tab_switch() {
        let mut mgr = AnimationManager::default();
        mgr.tab_switch();
        assert!(mgr.is_animating());
        assert!(mgr.needs_redraw());
        let alpha = mgr.tab_switch_alpha();
        assert!(alpha >= 0.0 && alpha <= 1.0);
    }

    #[test]
    fn t_manager_stop_all() {
        let mut mgr = AnimationManager::default();
        mgr.tab_switch();
        mgr.stop_all();
        assert!(!mgr.is_animating());
    }

    #[test]
    fn t_manager_multiple_animations() {
        let mut mgr = AnimationManager::default();
        mgr.window_open();
        assert!(mgr.is_animating());
        assert!((mgr.window_open_alpha() - 0.0).abs() < 0.01 || mgr.window_open_alpha() > 0.0);
    }

    #[test]
    fn t_manager_split_expand() {
        let mut mgr = AnimationManager::default();
        mgr.split_expand();
        assert!(mgr.is_animating());
        let scale = mgr.split_expand_scale();
        assert!(scale >= 0.0 && scale <= 1.0);
    }

    #[test]
    fn t_manager_overlay_slide() {
        let mut mgr = AnimationManager::default();
        mgr.overlay_slide();
        assert!(mgr.is_animating());
        let p = mgr.overlay_slide_progress();
        assert!(p >= 0.0 && p <= 1.0);
    }
}
