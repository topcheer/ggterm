//! P28-F: Performance monitor — FPS graph and frame time tracking.
//!
//! Tracks frame timing history for an on-screen FPS monitor overlay.
//! Also includes cursor particle effects configuration.

use std::collections::VecDeque;
use std::time::Instant;

/// Number of frame samples to keep for averaging.
const FRAME_HISTORY: usize = 120;

/// Performance monitor state.
#[derive(Debug)]
pub struct PerfMonitor {
    /// Whether the perf overlay is visible.
    pub visible: bool,
    /// Frame time history in microseconds.
    frame_times_us: VecDeque<u64>,
    /// Last frame timestamp.
    last_frame: Option<Instant>,
    /// Current FPS (smoothed).
    fps: f32,
    /// Current frame time in ms.
    frame_time_ms: f32,
    /// Total frames rendered.
    total_frames: u64,
    /// Min frame time observed (ms).
    min_frame_ms: f32,
    /// Max frame time observed (ms).
    max_frame_ms: f32,
    /// GPU render pass count.
    pub render_passes: u32,
    /// Draw call count.
    pub draw_calls: u32,
}

impl Default for PerfMonitor {
    fn default() -> Self {
        Self {
            visible: false,
            frame_times_us: VecDeque::with_capacity(FRAME_HISTORY),
            last_frame: None,
            fps: 0.0,
            frame_time_ms: 0.0,
            total_frames: 0,
            min_frame_ms: f32::MAX,
            max_frame_ms: 0.0,
            render_passes: 0,
            draw_calls: 0,
        }
    }
}

impl PerfMonitor {
    /// Create new performance monitor.
    pub fn new() -> Self {
        Self::default()
    }

    /// Toggle visibility.
    pub fn toggle(&mut self) {
        self.visible = !self.visible;
    }

    /// Record a frame. Call at the start of each render_frame().
    pub fn record_frame(&mut self) {
        let now = Instant::now();
        if let Some(last) = self.last_frame {
            let elapsed = now.duration_since(last);
            let us = elapsed.as_micros() as u64;

            self.frame_times_us.push_back(us);
            if self.frame_times_us.len() > FRAME_HISTORY {
                self.frame_times_us.pop_front();
            }

            // Update stats
            let ms = us as f32 / 1000.0;
            self.frame_time_ms = ms;
            self.min_frame_ms = self.min_frame_ms.min(ms);
            self.max_frame_ms = self.max_frame_ms.max(ms);
        }
        self.last_frame = Some(now);
        self.total_frames += 1;

        // Calculate smoothed FPS
        if let Some(avg_us) = self.avg_frame_time_us() {
            self.fps = if avg_us > 0 {
                1_000_000.0 / avg_us as f32
            } else {
                0.0
            };
        }
    }

    /// Average frame time in microseconds over history.
    fn avg_frame_time_us(&self) -> Option<u64> {
        if self.frame_times_us.is_empty() {
            return None;
        }
        let sum: u64 = self.frame_times_us.iter().sum();
        Some(sum / self.frame_times_us.len() as u64)
    }

    /// Current FPS.
    pub fn fps(&self) -> f32 {
        self.fps
    }

    /// Current frame time in ms.
    pub fn frame_time_ms(&self) -> f32 {
        self.frame_time_ms
    }

    /// Min frame time in ms.
    pub fn min_frame_ms(&self) -> f32 {
        if self.min_frame_ms == f32::MAX {
            0.0
        } else {
            self.min_frame_ms
        }
    }

    /// Max frame time in ms.
    pub fn max_frame_ms(&self) -> f32 {
        self.max_frame_ms
    }

    /// Total frames rendered.
    pub fn total_frames(&self) -> u64 {
        self.total_frames
    }

    /// Get frame time history for graph (as f32 ms values).
    pub fn frame_time_graph(&self) -> Vec<f32> {
        self.frame_times_us
            .iter()
            .map(|&us| us as f32 / 1000.0)
            .collect()
    }

    /// Reset statistics.
    pub fn reset(&mut self) {
        self.frame_times_us.clear();
        self.last_frame = None;
        self.fps = 0.0;
        self.frame_time_ms = 0.0;
        self.total_frames = 0;
        self.min_frame_ms = f32::MAX;
        self.max_frame_ms = 0.0;
    }

    /// Format the perf info for display.
    pub fn format_display(&self) -> String {
        format!(
            "{:.0} FPS | {:.1}ms (min {:.1} / max {:.1}) | {} frames",
            self.fps(),
            self.frame_time_ms(),
            self.min_frame_ms(),
            self.max_frame_ms(),
            self.total_frames()
        )
    }
}

/// Cursor particle effect type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CursorEffect {
    /// No particle effect.
    #[default]
    None,
    /// Trail particles behind cursor movement.
    Trail,
    /// Glow halo around cursor.
    Glow,
    /// Blinking pixel sparks.
    Sparks,
}

impl CursorEffect {
    /// Get the particle count for this effect.
    pub fn particle_count(self) -> usize {
        match self {
            CursorEffect::None => 0,
            CursorEffect::Trail => 8,
            CursorEffect::Glow => 16,
            CursorEffect::Sparks => 12,
        }
    }

    /// Get the particle lifetime in frames.
    pub fn lifetime_frames(self) -> u32 {
        match self {
            CursorEffect::None => 0,
            CursorEffect::Trail => 20,
            CursorEffect::Glow => 30,
            CursorEffect::Sparks => 15,
        }
    }
}

/// A single cursor particle.
#[derive(Debug, Clone, Copy)]
pub struct CursorParticle {
    /// X position in pixels.
    pub x: f32,
    /// Y position in pixels.
    pub y: f32,
    /// Velocity X.
    pub vx: f32,
    /// Velocity Y.
    pub vy: f32,
    /// Alpha (0.0 to 1.0).
    pub alpha: f32,
    /// Remaining life frames.
    pub life: u32,
    /// Max life (for alpha interpolation).
    pub max_life: u32,
    /// Particle size.
    pub size: f32,
}

/// Manages cursor particle effects.
#[derive(Debug)]
pub struct CursorParticleSystem {
    /// Current effect.
    effect: CursorEffect,
    /// Active particles.
    particles: Vec<CursorParticle>,
    /// Last cursor position.
    last_cursor: Option<(f32, f32)>,
}

impl Default for CursorParticleSystem {
    fn default() -> Self {
        Self {
            effect: CursorEffect::None,
            particles: Vec::new(),
            last_cursor: None,
        }
    }
}

impl CursorParticleSystem {
    /// Create new particle system.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the cursor effect.
    pub fn set_effect(&mut self, effect: CursorEffect) {
        self.effect = effect;
        if effect == CursorEffect::None {
            self.particles.clear();
        }
    }

    /// Get current effect.
    pub fn effect(&self) -> CursorEffect {
        self.effect
    }

    /// Whether any particles are active.
    pub fn has_particles(&self) -> bool {
        !self.particles.is_empty()
    }

    /// Whether this effect needs GPU rendering.
    pub fn needs_render(&self) -> bool {
        self.effect != CursorEffect::None && self.has_particles()
    }

    /// Update cursor position and spawn particles.
    pub fn update_cursor(&mut self, x: f32, y: f32) {
        if let Some((lx, ly)) = self.last_cursor {
            let dx = x - lx;
            let dy = y - ly;
            let dist = (dx * dx + dy * dy).sqrt();

            if self.effect == CursorEffect::Trail && dist > 1.0 {
                let count = (dist / 3.0).ceil() as usize;
                for _ in 0..count.min(4) {
                    self.spawn_particle(x, y, dx * 0.1, dy * 0.1);
                }
            }
        }
        self.last_cursor = Some((x, y));
    }

    /// Spawn a particle.
    fn spawn_particle(&mut self, x: f32, y: f32, vx: f32, vy: f32) {
        let max_life = self.effect.lifetime_frames();
        let jitter = |lo: f32, hi: f32| -> f32 {
            let seed = x as u64 + y as u64 + self.particles.len() as u64;
            let range = hi - lo;
            lo + ((seed.wrapping_mul(2654435761) % 1000) as f32 / 1000.0) * range
        };

        self.particles.push(CursorParticle {
            x: x + jitter(-2.0, 2.0),
            y: y + jitter(-2.0, 2.0),
            vx: vx + jitter(-0.5, 0.5),
            vy: vy + jitter(-0.5, 0.5),
            alpha: 0.8,
            life: max_life,
            max_life,
            size: jitter(1.5, 3.0),
        });

        // Cap particle count
        let max = self.effect.particle_count() * 4; // Allow some headroom
        while self.particles.len() > max {
            self.particles.remove(0);
        }
    }

    /// Tick the particle system (called each frame).
    pub fn tick(&mut self) {
        for p in &mut self.particles {
            p.x += p.vx;
            p.y += p.vy;
            p.vy += 0.05; // slight gravity
            p.life = p.life.saturating_sub(1);
            p.alpha = (p.life as f32 / p.max_life as f32) * 0.8;
        }
        self.particles.retain(|p| p.life > 0);
    }

    /// Get all active particles.
    pub fn particles(&self) -> &[CursorParticle] {
        &self.particles
    }

    /// Clear all particles.
    pub fn clear(&mut self) {
        self.particles.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_perf_monitor_default() {
        let m = PerfMonitor::new();
        assert!(!m.visible);
        assert_eq!(m.fps(), 0.0);
        assert_eq!(m.total_frames(), 0);
    }

    #[test]
    fn t_perf_monitor_toggle() {
        let mut m = PerfMonitor::new();
        assert!(!m.visible);
        m.toggle();
        assert!(m.visible);
    }

    #[test]
    fn t_perf_monitor_record_frames() {
        let mut m = PerfMonitor::new();
        m.record_frame();
        std::thread::sleep(std::time::Duration::from_micros(1000));
        m.record_frame();
        assert!(m.total_frames() >= 2);
        assert!(m.fps() > 0.0);
    }

    #[test]
    fn t_perf_monitor_frame_time_graph() {
        let mut m = PerfMonitor::new();
        for _ in 0..5 {
            m.record_frame();
            std::thread::sleep(std::time::Duration::from_micros(500));
        }
        let graph = m.frame_time_graph();
        assert!(graph.len() >= 4); // at least 4 intervals
    }

    #[test]
    fn t_perf_monitor_reset() {
        let mut m = PerfMonitor::new();
        m.record_frame();
        m.reset();
        assert_eq!(m.total_frames(), 0);
        assert_eq!(m.fps(), 0.0);
    }

    #[test]
    fn t_perf_monitor_format_display() {
        let m = PerfMonitor::new();
        let s = m.format_display();
        assert!(s.contains("FPS"));
        assert!(s.contains("frames"));
    }

    #[test]
    fn t_perf_monitor_min_max() {
        let mut m = PerfMonitor::new();
        for _ in 0..3 {
            m.record_frame();
            std::thread::sleep(std::time::Duration::from_micros(1000));
        }
        let min = m.min_frame_ms();
        let max = m.max_frame_ms();
        assert!(min <= max);
    }

    #[test]
    fn t_cursor_effect_default() {
        assert_eq!(CursorEffect::None, CursorEffect::default());
    }

    #[test]
    fn t_cursor_effect_particle_count() {
        assert_eq!(CursorEffect::None.particle_count(), 0);
        assert!(CursorEffect::Trail.particle_count() > 0);
        assert!(CursorEffect::Glow.particle_count() > 0);
    }

    #[test]
    fn t_cursor_effect_lifetime() {
        assert_eq!(CursorEffect::None.lifetime_frames(), 0);
        assert!(CursorEffect::Trail.lifetime_frames() > 0);
    }

    #[test]
    fn t_particle_system_default() {
        let ps = CursorParticleSystem::new();
        assert_eq!(ps.effect(), CursorEffect::None);
        assert!(!ps.has_particles());
        assert!(!ps.needs_render());
    }

    #[test]
    fn t_particle_system_set_effect() {
        let mut ps = CursorParticleSystem::new();
        ps.set_effect(CursorEffect::Glow);
        assert_eq!(ps.effect(), CursorEffect::Glow);
    }

    #[test]
    fn t_particle_system_clear_on_none() {
        let mut ps = CursorParticleSystem::new();
        ps.set_effect(CursorEffect::Trail);
        ps.set_effect(CursorEffect::None);
        assert!(!ps.has_particles());
    }

    #[test]
    fn t_particle_system_tick_decay() {
        let mut ps = CursorParticleSystem::new();
        ps.set_effect(CursorEffect::Trail);
        ps.update_cursor(10.0, 10.0);
        ps.update_cursor(20.0, 20.0); // should spawn trail particles
        let initial_count = ps.particles().len();
        assert!(initial_count > 0, "should have trail particles");
        for _ in 0..50 {
            ps.tick();
        }
        assert_eq!(ps.particles().len(), 0); // all expired
    }

    #[test]
    fn t_particle_system_alpha_decay() {
        let mut ps = CursorParticleSystem::new();
        ps.set_effect(CursorEffect::Trail);
        ps.update_cursor(10.0, 10.0);
        ps.update_cursor(30.0, 30.0); // spawn particles
        if let Some(p) = ps.particles().first() {
            let initial_alpha = p.alpha;
            ps.tick();
            let after_alpha = ps.particles().first().unwrap().alpha;
            assert!(after_alpha <= initial_alpha);
        }
    }
}
