//! Events flowing through the application event loop.
//!
//! Extended in Phase 5 with tab management, theme switching, and AI events.

use std::sync::mpsc;

#[cfg(feature = "ai")]
use ggterm_ai::Action;

/// Events that can arrive in the main event loop.
#[derive(Debug)]
pub enum AppEvent {
    // ── Core terminal events (Phase 1) ──

    /// Raw bytes read from the PTY (sent from the PTY reader thread).
    PtyBytes(Vec<u8>),

    /// Terminal was resized (cols, rows).
    Resize { cols: u16, rows: u16 },

    /// Keyboard input (encoded as ANSI bytes, ready to write to PTY).
    Keyboard(Vec<u8>),

    /// The PTY child process has exited.
    PtyExit,

    /// Application should quit.
    Quit,

    // ── Tab management events (Phase 5-B) ──

    /// Open a new tab.
    NewTab,

    /// Close the tab at the given index (or active tab if None).
    CloseTab(Option<usize>),

    /// Switch to the tab at the given index.
    SwitchTab(usize),

    /// Switch to the next tab (wraps around).
    NextTab,

    /// Switch to the previous tab (wraps around).
    PrevTab,

    // ── Theme events (Phase 5-A) ──

    /// Set the theme by name (e.g., "dark", "light", "dracula").
    SetTheme(String),

    /// Cycle to the next built-in theme.
    CycleTheme,

    // ── AI events (Phase 5-C, feature-gated) ──

    /// Request an AI action (explain, suggest, error help, NL2cmd).
    #[cfg(feature = "ai")]
    AIRequest(Action),

    /// AI response arrived (text or error).
    #[cfg(feature = "ai")]
    AIResponse(String),

    /// AI request failed.
    #[cfg(feature = "ai")]
    AIError(String),
}

/// Channel type for sending events to the main loop.
pub type EventSender = mpsc::Sender<AppEvent>;

/// Channel type for receiving events in the main loop.
pub type EventReceiver = mpsc::Receiver<AppEvent>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_event_channel_basic() {
        let (tx, rx) = mpsc::channel();
        tx.send(AppEvent::Quit).unwrap();
        match rx.recv() {
            Ok(AppEvent::Quit) => {}
            other => panic!("expected Quit, got {:?}", other),
        }
    }

    #[test]
    fn t_event_resize() {
        let (tx, rx) = mpsc::channel();
        tx.send(AppEvent::Resize { cols: 120, rows: 40 }).unwrap();
        match rx.recv() {
            Ok(AppEvent::Resize { cols, rows }) => {
                assert_eq!(cols, 120);
                assert_eq!(rows, 40);
            }
            other => panic!("expected Resize, got {:?}", other),
        }
    }

    #[test]
    fn t_event_new_tab() {
        let (tx, rx) = mpsc::channel();
        tx.send(AppEvent::NewTab).unwrap();
        assert!(matches!(rx.recv(), Ok(AppEvent::NewTab)));
    }

    #[test]
    fn t_event_close_tab() {
        let (tx, rx) = mpsc::channel();
        tx.send(AppEvent::CloseTab(Some(2))).unwrap();
        assert!(matches!(rx.recv(), Ok(AppEvent::CloseTab(Some(2)))));
    }

    #[test]
    fn t_event_close_active_tab() {
        let (tx, rx) = mpsc::channel();
        tx.send(AppEvent::CloseTab(None)).unwrap();
        assert!(matches!(rx.recv(), Ok(AppEvent::CloseTab(None))));
    }

    #[test]
    fn t_event_switch_tab() {
        let (tx, rx) = mpsc::channel();
        tx.send(AppEvent::SwitchTab(3)).unwrap();
        assert!(matches!(rx.recv(), Ok(AppEvent::SwitchTab(3))));
    }

    #[test]
    fn t_event_next_tab() {
        let (tx, rx) = mpsc::channel();
        tx.send(AppEvent::NextTab).unwrap();
        assert!(matches!(rx.recv(), Ok(AppEvent::NextTab)));
    }

    #[test]
    fn t_event_prev_tab() {
        let (tx, rx) = mpsc::channel();
        tx.send(AppEvent::PrevTab).unwrap();
        assert!(matches!(rx.recv(), Ok(AppEvent::PrevTab)));
    }

    #[test]
    fn t_event_set_theme() {
        let (tx, rx) = mpsc::channel();
        tx.send(AppEvent::SetTheme("dracula".to_string())).unwrap();
        match rx.recv() {
            Ok(AppEvent::SetTheme(name)) => assert_eq!(name, "dracula"),
            other => panic!("expected SetTheme, got {:?}", other),
        }
    }

    #[test]
    fn t_event_cycle_theme() {
        let (tx, rx) = mpsc::channel();
        tx.send(AppEvent::CycleTheme).unwrap();
        assert!(matches!(rx.recv(), Ok(AppEvent::CycleTheme)));
    }

    #[test]
    fn t_event_multiple_events() {
        let (tx, rx) = mpsc::channel();
        tx.send(AppEvent::NewTab).unwrap();
        tx.send(AppEvent::SwitchTab(0)).unwrap();
        tx.send(AppEvent::SetTheme("light".to_string())).unwrap();
        tx.send(AppEvent::Quit).unwrap();

        let events: Vec<_> = rx.try_iter().collect();
        assert_eq!(events.len(), 4);
    }
}
