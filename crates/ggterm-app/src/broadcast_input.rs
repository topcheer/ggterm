//! # Broadcast Input
//!
//! Simultaneous input across multiple panes or tabs.
//! P25-D.

/// Broadcast mode controlling where input is replicated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BroadcastMode {
    /// No broadcasting — input goes to active pane only.
    #[default]
    None,
    /// Broadcast to all panes in the current tab.
    AllPanes,
    /// Broadcast to all tabs.
    AllTabs,
}

impl BroadcastMode {
    /// Cycle to the next broadcast mode.
    pub fn cycle(self) -> Self {
        match self {
            Self::None => Self::AllPanes,
            Self::AllPanes => Self::AllTabs,
            Self::AllTabs => Self::None,
        }
    }

    /// Get a display label for the status bar.
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "",
            Self::AllPanes => "BCAST:PANES",
            Self::AllTabs => "BCAST:TABS",
        }
    }

    /// Get a short icon for compact display.
    pub fn icon(self) -> &'static str {
        match self {
            Self::None => "",
            Self::AllPanes => "",
            Self::AllTabs => "",
        }
    }

    /// Whether broadcasting is active.
    pub fn is_active(self) -> bool {
        !matches!(self, Self::None)
    }
}

/// Broadcast state tracker.
#[derive(Debug, Clone, Default)]
pub struct BroadcastState {
    /// Current broadcast mode.
    pub mode: BroadcastMode,
    /// Count of panes/tabs that received the last broadcast (for feedback).
    pub last_target_count: usize,
}

impl BroadcastState {
    /// Create a new broadcast state with no broadcasting.
    pub fn new() -> Self {
        Self::default()
    }

    /// Toggle through broadcast modes.
    pub fn cycle(&mut self) {
        self.mode = self.mode.cycle();
    }

    /// Set the broadcast mode directly.
    pub fn set(&mut self, mode: BroadcastMode) {
        self.mode = mode;
    }

    /// Determine which target indices should receive input.
    ///
    /// For `AllPanes`, returns all pane indices in the current tab.
    /// For `AllTabs`, returns all tab indices.
    /// For `None`, returns just the active index.
    pub fn targets(&self, active: usize, count: usize) -> Vec<usize> {
        if count == 0 {
            return vec![];
        }
        match self.mode {
            BroadcastMode::None => vec![active.min(count - 1)],
            BroadcastMode::AllPanes | BroadcastMode::AllTabs => (0..count).collect(),
        }
    }

    /// Record how many targets received the last broadcast.
    pub fn record_targets(&mut self, count: usize) {
        self.last_target_count = count;
    }

    /// Status string for the status bar.
    pub fn status(&self) -> String {
        if self.mode.is_active() {
            format!("{} {}", self.mode.label(), self.last_target_count)
        } else {
            String::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_mode_cycle() {
        assert_eq!(BroadcastMode::None.cycle(), BroadcastMode::AllPanes);
        assert_eq!(BroadcastMode::AllPanes.cycle(), BroadcastMode::AllTabs);
        assert_eq!(BroadcastMode::AllTabs.cycle(), BroadcastMode::None);
    }

    #[test]
    fn t_mode_is_active() {
        assert!(!BroadcastMode::None.is_active());
        assert!(BroadcastMode::AllPanes.is_active());
        assert!(BroadcastMode::AllTabs.is_active());
    }

    #[test]
    fn t_mode_label() {
        assert_eq!(BroadcastMode::None.label(), "");
        assert!(!BroadcastMode::AllPanes.label().is_empty());
        assert!(!BroadcastMode::AllTabs.label().is_empty());
    }

    #[test]
    fn t_targets_none() {
        let s = BroadcastState::new();
        let targets = s.targets(2, 5);
        assert_eq!(targets, vec![2]);
    }

    #[test]
    fn t_targets_all_panes() {
        let mut s = BroadcastState::new();
        s.set(BroadcastMode::AllPanes);
        let targets = s.targets(2, 5);
        assert_eq!(targets, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn t_targets_all_tabs() {
        let mut s = BroadcastState::new();
        s.set(BroadcastMode::AllTabs);
        let targets = s.targets(0, 3);
        assert_eq!(targets, vec![0, 1, 2]);
    }

    #[test]
    fn t_targets_empty() {
        let s = BroadcastState::new();
        assert!(s.targets(0, 0).is_empty());
    }

    #[test]
    fn t_targets_none_active_clamped() {
        let s = BroadcastState::new();
        let targets = s.targets(10, 3);
        assert_eq!(targets, vec![2]); // clamped to last
    }

    #[test]
    fn t_state_cycle_updates_mode() {
        let mut s = BroadcastState::new();
        assert!(!s.mode.is_active());
        s.cycle();
        assert_eq!(s.mode, BroadcastMode::AllPanes);
        s.cycle();
        assert_eq!(s.mode, BroadcastMode::AllTabs);
        s.cycle();
        assert_eq!(s.mode, BroadcastMode::None);
    }

    #[test]
    fn t_status_string() {
        let mut s = BroadcastState::new();
        assert!(s.status().is_empty());
        s.set(BroadcastMode::AllPanes);
        s.record_targets(4);
        assert!(s.status().contains("BCAST"));
        assert!(s.status().contains("4"));
    }

    #[test]
    fn t_record_targets() {
        let mut s = BroadcastState::new();
        s.record_targets(7);
        assert_eq!(s.last_target_count, 7);
    }
}
