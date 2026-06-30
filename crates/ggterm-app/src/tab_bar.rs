//! Tab bar: visual tab strip data model with pill-shaped layout.
//!
//! Provides [`TabInfo`] for each open tab, [`TabBarState`] which aggregates
//! the display-ready tab list, and [`TabBarLayout`] for pill-shaped geometric
//! layout (corner radius, padding, close buttons, + button).
//! The actual wgpu rendering happens in `render_frame()` via the renderer
//! overlay; this module owns the data model.

// ── Layout constants ───────────────────────────────────────────────────

/// Padding inside the tab bar strip (top and bottom).
const TAB_BAR_PADDING_V: f32 = 4.0;
/// Horizontal padding at the start/end of the tab bar.
const TAB_BAR_PADDING_H: f32 = 8.0;
/// Horizontal gap between adjacent tab pills.
const TAB_GAP: f32 = 4.0;
/// Horizontal padding inside each tab pill.
const TAB_INNER_PADDING_H: f32 = 10.0;
/// Corner radius for pill-shaped tabs (half of tab height = fully rounded).
const TAB_CORNER_RADIUS: f32 = 8.0;
/// Width of the close button area on each tab.
const CLOSE_BUTTON_SIZE: f32 = 16.0;
/// Gap between tab text and close button.
const CLOSE_BUTTON_GAP: f32 = 4.0;
/// Size of the "+" new tab button.
const NEW_TAB_BUTTON_SIZE: f32 = 20.0;
/// Estimated average character width in pixels (at 14px monospace).
const CHAR_WIDTH_ESTIMATE: f32 = 8.4;
/// Maximum characters to show in a tab title before truncation.
const MAX_TAB_TITLE_CHARS: usize = 14;

// ── TabInfo ─────────────────────────────────────────────────────────────

/// Display info for a single tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabInfo {
    /// Tab title (from OSC 0/2 or shell name).
    pub title: String,
    /// 1-based tab index.
    pub index: usize,
    /// Whether this is the active tab.
    pub active: bool,
    /// Whether the tab has unseen output (dirty / bell).
    pub dirty: bool,
}

impl TabInfo {
    /// Create a new TabInfo.
    pub fn new(title: impl Into<String>, index: usize, active: bool) -> Self {
        Self {
            title: title.into(),
            index,
            active,
            dirty: false,
        }
    }

    /// Truncate title to `max_chars` characters, appending an ellipsis.
    pub fn truncated_title(&self, max_chars: usize) -> String {
        let count = self.title.chars().count();
        if count > max_chars {
            format!(
                "{}\u{2026}",
                self.title
                    .chars()
                    .take(max_chars.saturating_sub(1))
                    .collect::<String>()
            )
        } else {
            self.title.clone()
        }
    }

    /// Format as a display string: `1:zsh*` (active) or `2:vim` (inactive).
    pub fn format(&self) -> String {
        let title = self.truncated_title(12);
        let suffix = if self.active { "*" } else { "" };
        let dirty = if self.dirty && !self.active { "!" } else { "" };
        format!("{}:{}{}{}", self.index, title, suffix, dirty)
    }

    /// Estimated pixel width of the tab text content (index + title + close button).
    pub fn estimated_width(&self) -> f32 {
        let title = self.truncated_title(MAX_TAB_TITLE_CHARS);
        // "N:" prefix + title text.
        let prefix_chars = 2 + self.index.to_string().len().saturating_sub(1);
        let text_width = (prefix_chars + title.chars().count()) as f32 * CHAR_WIDTH_ESTIMATE;
        // inner padding + close button.
        text_width + TAB_INNER_PADDING_H * 2.0 + CLOSE_BUTTON_GAP + CLOSE_BUTTON_SIZE
    }
}

// ── Tab pill geometry ──────────────────────────────────────────────────

/// Pixel-space rectangle for a rendered tab pill.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TabRect {
    /// Left edge in pixels.
    pub x: f32,
    /// Top edge in pixels.
    pub y: f32,
    /// Width in pixels.
    pub w: f32,
    /// Height in pixels.
    pub h: f32,
    /// Corner radius for rounded-rect SDF.
    pub radius: f32,
}

/// Close button position for a tab.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CloseButtonRect {
    /// Center X.
    pub cx: f32,
    /// Center Y.
    pub cy: f32,
    /// Button size (square, size×size).
    pub size: f32,
}

/// A single tab's geometric layout for rendering.
#[derive(Debug, Clone, PartialEq)]
pub struct TabLayout {
    /// Pill background rectangle.
    pub rect: TabRect,
    /// Close button position (if tab is hovered or active).
    pub close: CloseButtonRect,
    /// Text left position in pixels.
    pub text_x: f32,
    /// Whether the close button is visible.
    pub close_visible: bool,
    /// Reference to the tab info.
    pub info: TabInfo,
}

// ── TabBarLayout ───────────────────────────────────────────────────────

/// Complete geometric layout for the entire tab bar.
///
/// Computed by [`TabBarState::compute_layout`] from the tab list and
/// available width.  All coordinates are in surface pixels with (0,0) at
/// the top-left corner.
#[derive(Debug, Clone, PartialEq)]
pub struct TabBarLayout {
    /// Laid-out tab pills (left to right).
    pub tabs: Vec<TabLayout>,
    /// "+" new tab button position.
    pub new_tab_button: CloseButtonRect,
    /// Total bar height in pixels.
    pub bar_height: f32,
    /// Whether the layout is empty / hidden.
    pub visible: bool,
}

impl Default for TabBarLayout {
    fn default() -> Self {
        Self {
            tabs: Vec::new(),
            new_tab_button: CloseButtonRect {
                cx: 0.0,
                cy: 0.0,
                size: NEW_TAB_BUTTON_SIZE,
            },
            bar_height: 0.0,
            visible: false,
        }
    }
}

// ── TabBarState ─────────────────────────────────────────────────────────

/// Aggregated tab bar state for rendering.
///
/// Updated every frame from the DesktopApp's session list.
#[derive(Debug, Clone, Default)]
pub struct TabBarState {
    /// Display info for each tab.
    pub tabs: Vec<TabInfo>,
    /// Whether the tab bar is visible (more than 1 tab + setting enabled).
    pub visible: bool,
}

impl TabBarState {
    /// Create an empty tab bar state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Rebuild the tab list from session titles and active index.
    pub fn update(&mut self, titles: &[&str], active: usize) {
        self.tabs = titles
            .iter()
            .enumerate()
            .map(|(i, &title)| TabInfo {
                title: title.to_string(),
                index: i + 1,
                active: i == active,
                dirty: false,
            })
            .collect();
        self.visible = self.tabs.len() > 1;
    }

    /// Format the entire tab bar as a single string: `1:zsh* | 2:vim | 3:logs`.
    pub fn format(&self) -> String {
        if self.tabs.is_empty() {
            return String::new();
        }
        self.tabs
            .iter()
            .map(|t| t.format())
            .collect::<Vec<_>>()
            .join(" | ")
    }

    /// Compute the pill-shaped geometric layout for rendering.
    ///
    /// `surface_width` is the total render surface width in pixels.
    /// `font_size` is the current terminal font size (used to derive bar height).
    pub fn compute_layout(&self, _surface_width: f32, font_size: f32) -> TabBarLayout {
        if !self.visible || self.tabs.is_empty() {
            return TabBarLayout::default();
        }

        let bar_height = font_size + TAB_BAR_PADDING_V * 2.0;
        let tab_height = bar_height - TAB_BAR_PADDING_V;
        let tab_y = TAB_BAR_PADDING_V;

        let mut layouts = Vec::with_capacity(self.tabs.len());
        let mut x = TAB_BAR_PADDING_H;

        for tab in &self.tabs {
            let w = tab.estimated_width();
            let radius = TAB_CORNER_RADIUS.min(tab_height / 2.0);

            // Close button: right-aligned inside the pill.
            let close_cx = x + w - TAB_INNER_PADDING_H - CLOSE_BUTTON_SIZE / 2.0;
            let close_cy = tab_y + tab_height / 2.0;
            let close_visible = tab.active || tab.dirty;

            // Text starts after inner left padding.
            let text_x = x + TAB_INNER_PADDING_H;

            layouts.push(TabLayout {
                rect: TabRect {
                    x,
                    y: tab_y,
                    w,
                    h: tab_height,
                    radius,
                },
                close: CloseButtonRect {
                    cx: close_cx,
                    cy: close_cy,
                    size: CLOSE_BUTTON_SIZE,
                },
                text_x,
                close_visible,
                info: tab.clone(),
            });

            x += w + TAB_GAP;
        }

        // "+" button after the last tab.
        let new_tab_x = x + NEW_TAB_BUTTON_SIZE / 2.0;
        let new_tab_y = tab_y + tab_height / 2.0;

        TabBarLayout {
            tabs: layouts,
            new_tab_button: CloseButtonRect {
                cx: new_tab_x,
                cy: new_tab_y,
                size: NEW_TAB_BUTTON_SIZE,
            },
            bar_height,
            visible: true,
        }
    }

    /// Find which tab is at a given pixel x position (for click handling).
    pub fn tab_at_x(&self, layout: &TabBarLayout, x: f32) -> Option<usize> {
        for (i, tl) in layout.tabs.iter().enumerate() {
            if x >= tl.rect.x && x < tl.rect.x + tl.rect.w {
                return Some(i);
            }
        }
        None
    }

    /// Check if a pixel position is over a tab's close button.
    pub fn close_button_at(&self, layout: &TabBarLayout, x: f32, y: f32) -> Option<usize> {
        for (i, tl) in layout.tabs.iter().enumerate() {
            if !tl.close_visible {
                continue;
            }
            let half = tl.close.size / 2.0;
            if (x - tl.close.cx).abs() <= half && (y - tl.close.cy).abs() <= half {
                return Some(i);
            }
        }
        None
    }

    /// Check if a pixel position is over the "+" new tab button.
    pub fn is_new_tab_button_at(&self, layout: &TabBarLayout, x: f32, y: f32) -> bool {
        let half = layout.new_tab_button.size / 2.0;
        (x - layout.new_tab_button.cx).abs() <= half && (y - layout.new_tab_button.cy).abs() <= half
    }
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_tab_info_format_active() {
        let tab = TabInfo::new("zsh", 1, true);
        assert_eq!(tab.format(), "1:zsh*");
    }

    #[test]
    fn t_tab_info_format_inactive() {
        let tab = TabInfo::new("vim", 2, false);
        assert_eq!(tab.format(), "2:vim");
    }

    #[test]
    fn t_tab_info_truncated_title() {
        let tab = TabInfo::new("very_long_process_name", 1, true);
        let formatted = tab.format();
        // Should be truncated to 12 chars + ellipsis
        assert!(formatted.starts_with("1:"));
        assert!(formatted.contains('\u{2026}'));
        assert!(formatted.ends_with('*'));
    }

    #[test]
    fn t_tab_info_dirty_marker() {
        let mut tab = TabInfo::new("logs", 3, false);
        tab.dirty = true;
        assert_eq!(tab.format(), "3:logs!");
    }

    #[test]
    fn t_tab_bar_state_single_tab_hidden() {
        let mut state = TabBarState::new();
        state.update(&["zsh"], 0);
        assert!(!state.visible);
    }

    #[test]
    fn t_tab_bar_state_multi_tab_visible() {
        let mut state = TabBarState::new();
        state.update(&["zsh", "vim", "logs"], 1);
        assert!(state.visible);
        assert_eq!(state.format(), "1:zsh | 2:vim* | 3:logs");
    }

    #[test]
    fn t_tab_bar_state_empty() {
        let state = TabBarState::new();
        assert!(!state.visible);
        assert_eq!(state.format(), "");
    }

    #[test]
    fn t_tab_bar_state_update_changes_active() {
        let mut state = TabBarState::new();
        state.update(&["a", "b"], 0);
        assert!(state.tabs[0].active);
        assert!(!state.tabs[1].active);

        state.update(&["a", "b"], 1);
        assert!(!state.tabs[0].active);
        assert!(state.tabs[1].active);
    }

    #[test]
    fn t_tab_info_new_default_not_dirty() {
        let tab = TabInfo::new("test", 1, true);
        assert!(!tab.dirty);
    }

    #[test]
    fn t_truncated_title_short_unchanged() {
        let tab = TabInfo::new("sh", 1, false);
        assert_eq!(tab.truncated_title(10), "sh");
    }

    // ── P19-H: Integration edge cases ─────────────────────────

    #[test]
    fn t_active_dirty_suppresses_marker() {
        // When active=true and dirty=true, dirty marker (!) is suppressed.
        let mut tab = TabInfo::new("logs", 1, true);
        tab.dirty = true;
        // Should be "1:logs*" not "1:logs!*"
        assert_eq!(tab.format(), "1:logs*");
    }

    #[test]
    fn t_truncated_title_exact_boundary() {
        // 12 chars exactly should NOT be truncated.
        let tab = TabInfo::new("twelvechars!", 1, false);
        assert_eq!(tab.truncated_title(12), "twelvechars!");
        // 13 chars should be truncated.
        let tab2 = TabInfo::new("thirteenchars", 1, false);
        assert!(tab2.truncated_title(12).contains('\u{2026}'));
    }

    #[test]
    fn t_tab_bar_many_tabs() {
        let mut state = TabBarState::new();
        let titles: Vec<&str> = (0..10).map(|_| "sh").collect();
        state.update(&titles, 5);
        assert!(state.visible);
        assert_eq!(state.tabs.len(), 10);
        // Tab 6 (index 5) should be active.
        assert!(state.tabs[5].active);
        assert!(!state.tabs[0].active);
    }

    #[test]
    fn t_tab_info_clone_eq() {
        let t1 = TabInfo::new("vim", 2, true);
        let t2 = t1.clone();
        assert_eq!(t1, t2);
    }

    #[test]
    fn t_update_resets_dirty() {
        // Even if a previous tab was dirty, update() resets all dirty flags.
        let mut state = TabBarState::new();
        state.update(&["a", "b"], 0);
        state.tabs[1].dirty = true;
        state.update(&["a", "b"], 0);
        assert!(!state.tabs[1].dirty);
    }

    #[test]
    fn t_format_inactive_last_tab() {
        let mut state = TabBarState::new();
        state.update(&["sh", "vim"], 0);
        // Active is 0, so tab 1 is inactive.
        let formatted = state.format();
        assert!(formatted.contains("1:sh*"));
        assert!(formatted.contains("2:vim"));
    }

    // ── Pill layout tests ──────────────────────────────────────

    #[test]
    fn t_estimated_width_includes_close_button() {
        let tab = TabInfo::new("vim", 2, true);
        let w = tab.estimated_width();
        assert!(w > CLOSE_BUTTON_SIZE + TAB_INNER_PADDING_H * 2.0);
    }

    #[test]
    fn t_compute_layout_single_tab_hidden() {
        let mut state = TabBarState::new();
        state.update(&["zsh"], 0);
        let layout = state.compute_layout(800.0, 14.0);
        assert!(!layout.visible);
    }

    #[test]
    fn t_compute_layout_multi_tab() {
        let mut state = TabBarState::new();
        state.update(&["a", "b", "c"], 1);
        let layout = state.compute_layout(800.0, 14.0);
        assert!(layout.visible);
        assert_eq!(layout.tabs.len(), 3);
        assert!(layout.tabs[0].rect.x < layout.tabs[1].rect.x);
        assert!(layout.tabs[1].rect.x < layout.tabs[2].rect.x);
    }

    #[test]
    fn t_compute_layout_bar_height_scales_with_font() {
        let mut state = TabBarState::new();
        state.update(&["a", "b"], 0);
        let l14 = state.compute_layout(800.0, 14.0);
        let l24 = state.compute_layout(800.0, 24.0);
        assert!(l24.bar_height > l14.bar_height);
        assert!((l14.bar_height - (14.0 + TAB_BAR_PADDING_V * 2.0)).abs() < 0.01);
    }

    #[test]
    fn t_compute_layout_close_button_active_only() {
        let mut state = TabBarState::new();
        state.update(&["a", "b"], 0);
        let layout = state.compute_layout(800.0, 14.0);
        assert!(layout.tabs[0].close_visible);
        assert!(!layout.tabs[1].close_visible);
    }

    #[test]
    fn t_compute_layout_close_button_dirty_visible() {
        let mut state = TabBarState::new();
        state.update(&["a", "b"], 0);
        state.tabs[1].dirty = true;
        let layout = state.compute_layout(800.0, 14.0);
        assert!(layout.tabs[1].close_visible);
    }

    #[test]
    fn t_tab_at_x_finds_correct_tab() {
        let mut state = TabBarState::new();
        state.update(&["a", "b", "c"], 0);
        let layout = state.compute_layout(800.0, 14.0);
        let idx = state.tab_at_x(&layout, layout.tabs[0].rect.x + 5.0);
        assert_eq!(idx, Some(0));
        let idx2 = state.tab_at_x(&layout, layout.tabs[1].rect.x + 5.0);
        assert_eq!(idx2, Some(1));
    }

    #[test]
    fn t_tab_at_x_misses_between_tabs() {
        let mut state = TabBarState::new();
        state.update(&["a", "b"], 0);
        let layout = state.compute_layout(800.0, 14.0);
        let gap_x = layout.tabs[0].rect.x + layout.tabs[0].rect.w + TAB_GAP / 2.0;
        let idx = state.tab_at_x(&layout, gap_x);
        assert_eq!(idx, None);
    }

    #[test]
    fn t_close_button_at_finds_button() {
        let mut state = TabBarState::new();
        state.update(&["a", "b"], 0);
        let layout = state.compute_layout(800.0, 14.0);
        let idx = state.close_button_at(&layout, layout.tabs[0].close.cx, layout.tabs[0].close.cy);
        assert_eq!(idx, Some(0));
    }

    #[test]
    fn t_close_button_at_misses_inactive() {
        let mut state = TabBarState::new();
        state.update(&["a", "b"], 0);
        let layout = state.compute_layout(800.0, 14.0);
        let idx = state.close_button_at(&layout, layout.tabs[1].close.cx, layout.tabs[1].close.cy);
        assert_eq!(idx, None);
    }

    #[test]
    fn t_new_tab_button_detected() {
        let mut state = TabBarState::new();
        state.update(&["a", "b"], 0);
        let layout = state.compute_layout(800.0, 14.0);
        assert!(state.is_new_tab_button_at(
            &layout,
            layout.new_tab_button.cx,
            layout.new_tab_button.cy
        ));
    }

    #[test]
    fn t_new_tab_button_miss() {
        let mut state = TabBarState::new();
        state.update(&["a", "b"], 0);
        let layout = state.compute_layout(800.0, 14.0);
        assert!(!state.is_new_tab_button_at(&layout, 5.0, 5.0));
    }

    #[test]
    fn t_layout_corner_radius_capped() {
        let mut state = TabBarState::new();
        state.update(&["a", "b"], 0);
        let layout = state.compute_layout(800.0, 14.0);
        let max_radius = layout.tabs[0].rect.h / 2.0;
        assert!(layout.tabs[0].rect.radius <= max_radius + 0.01);
    }
}
