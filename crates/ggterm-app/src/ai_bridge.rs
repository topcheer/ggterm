//! AI Bridge — connects the Phase 4 AIEngine to the app event loop.
//!
//! When the user triggers an AI action (explain, suggest, error help, NL2cmd),
//! the `AIBridge` runs the AIEngine in a background thread and delivers the
//! response via a channel. This keeps the terminal responsive while the LLM
//! is thinking.
//!
//! This module is gated behind the `ai` feature flag.

#![cfg(feature = "ai")]

use std::sync::mpsc;
use std::thread;

use ggterm_ai::{AIContext, AIEngine, AIResult, Action};

/// A request for an AI action.
#[derive(Debug, Clone)]
pub struct AIRequest {
    /// The action to perform.
    pub action: Action,
    /// The terminal context snapshot.
    pub context: AIContext,
    /// For NL2Command, the natural language input.
    pub natural_language: Option<String>,
}

impl AIRequest {
    /// Create a request for explain/suggest/error_help.
    pub fn new(action: Action, context: AIContext) -> Self {
        Self {
            action,
            context,
            natural_language: None,
        }
    }

    /// Create an NL2Command request.
    pub fn nl2cmd(natural_language: impl Into<String>, context: AIContext) -> Self {
        Self {
            action: Action::NL2Command,
            context,
            natural_language: Some(natural_language.into()),
        }
    }
}

/// Response from an AI request.
#[derive(Debug, Clone)]
pub struct AIResponse {
    /// The action that was requested.
    pub action: Action,
    /// The result (Ok with text, or Err with error).
    pub result: AIResult<String>,
}

/// The AI bridge: owns the engine and manages background requests.
///
/// Only one request can be in-flight at a time. If a new request arrives
/// while one is pending, the bridge returns `false` (caller should show
/// "AI is thinking..." feedback).
///
/// The engine ownership is transferred to the worker thread during
/// execution, then returned when the result arrives. This avoids
/// cloning `Box<dyn LLMProvider>`.
pub struct AIBridge {
    engine: Option<AIEngine>,
    /// Receiver for results from the background worker.
    result_rx: Option<mpsc::Receiver<(AIEngine, AIResponse)>>,
    /// Whether a request is currently in-flight.
    busy: bool,
}

impl AIBridge {
    /// Create an AIBridge with the given engine.
    pub fn new(engine: AIEngine) -> Self {
        Self {
            engine: Some(engine),
            result_rx: None,
            busy: false,
        }
    }

    /// Create with a mock engine (for testing).
    pub fn with_mock(response: impl Into<String>) -> Self {
        Self::new(AIEngine::with_mock(response))
    }

    /// Create with a failing engine (for testing).
    pub fn with_error(error: ggterm_ai::AIError) -> Self {
        Self::new(AIEngine::failing(error))
    }

    /// Check if a request is currently in-flight.
    pub fn is_busy(&self) -> bool {
        self.busy
    }

    /// Submit a new AI request. Returns `false` if a request is already pending.
    pub fn request(&mut self, req: AIRequest) -> bool {
        if self.busy || self.engine.is_none() {
            return false;
        }

        let engine = self.engine.take().expect("engine must exist");
        let (tx, rx) = mpsc::channel::<(AIEngine, AIResponse)>();
        self.result_rx = Some(rx);
        self.busy = true;

        thread::spawn(move || {
            // Execute the request. For NL2Command with natural_language, use nl2cmd.
            // Otherwise, use execute with the action.
            let result = if let Some(ref nl) = req.natural_language {
                engine.nl2cmd(nl, &req.context)
            } else {
                engine.execute(req.action.clone(), &req.context)
            };

            let _ = tx.send((
                engine,
                AIResponse {
                    action: req.action,
                    result,
                },
            ));
        });

        true
    }

    /// Poll for a completed result. Returns `Some(response)` if the request
    /// completed, or `None` if still pending.
    pub fn poll_result(&mut self) -> Option<AIResponse> {
        if !self.busy {
            return None;
        }
        let rx = self.result_rx.as_ref()?;
        match rx.try_recv() {
            Ok((engine, response)) => {
                self.engine = Some(engine);
                self.result_rx = None;
                self.busy = false;
                Some(response)
            }
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.result_rx = None;
                self.busy = false;
                Some(AIResponse {
                    action: Action::Explain,
                    result: Err(ggterm_ai::AIError::RequestFailed(
                        "AI worker thread crashed".to_string(),
                    )),
                })
            }
        }
    }

    /// Block until the result is ready (for testing).
    #[cfg(test)]
    pub fn wait_result(&mut self) -> AIResponse {
        assert!(self.busy, "no request in flight");
        let rx = self.result_rx.as_ref().expect("receiver must exist");
        let (engine, response) = rx.recv().expect("worker thread crashed");
        self.engine = Some(engine);
        self.result_rx = None;
        self.busy = false;
        response
    }

    /// Get a reference to the underlying engine (only when not busy).
    pub fn engine(&self) -> Option<&AIEngine> {
        self.engine.as_ref()
    }
}

impl Default for AIBridge {
    fn default() -> Self {
        Self::with_mock("AI not configured")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_ctx() -> AIContext {
        AIContext {
            last_command: Some("ls -la".to_string()),
            last_exit_code: Some(0),
            last_output: Some("total 0\ndrwxr-xr-x  2 user  staff  64 Jan 1 00:00 .".to_string()),
            recent_commands: vec!["pwd".to_string()],
            cwd: Some("/home/user".to_string()),
            shell: Some("bash".to_string()),
        }
    }

    // ── AIRequest ──

    #[test]
    fn t_request_new_explain() {
        let ctx = sample_ctx();
        let req = AIRequest::new(Action::Explain, ctx);
        assert_eq!(req.action, Action::Explain);
        assert!(req.natural_language.is_none());
    }

    #[test]
    fn t_request_new_suggest() {
        let ctx = sample_ctx();
        let req = AIRequest::new(Action::Suggest, ctx);
        assert_eq!(req.action, Action::Suggest);
    }

    #[test]
    fn t_request_new_error_help() {
        let ctx = sample_ctx();
        let req = AIRequest::new(Action::ErrorHelp, ctx);
        assert_eq!(req.action, Action::ErrorHelp);
    }

    #[test]
    fn t_request_nl2cmd() {
        let ctx = sample_ctx();
        let req = AIRequest::nl2cmd("list all files", ctx);
        assert_eq!(req.action, Action::NL2Command);
        assert_eq!(req.natural_language.as_deref(), Some("list all files"));
    }

    // ── AIBridge construction ──

    #[test]
    fn t_bridge_new() {
        let bridge = AIBridge::with_mock("test response");
        assert!(!bridge.is_busy());
        assert!(bridge.engine().is_some());
    }

    #[test]
    fn t_bridge_default() {
        let bridge = AIBridge::default();
        assert!(!bridge.is_busy());
        assert!(bridge.engine().is_some());
    }

    #[test]
    fn t_bridge_not_busy_initially() {
        let bridge = AIBridge::with_mock("response");
        assert!(!bridge.is_busy());
    }

    #[test]
    fn t_bridge_with_error() {
        let bridge = AIBridge::with_error(ggterm_ai::AIError::NoApiKey);
        assert!(bridge.engine().is_some());
    }

    // ── request/wait ──

    #[test]
    fn t_bridge_request_explain() {
        let mut bridge = AIBridge::with_mock("This lists files.");
        let ctx = sample_ctx();
        let req = AIRequest::new(Action::Explain, ctx);
        assert!(bridge.request(req));
        assert!(bridge.is_busy());

        let response = bridge.wait_result();
        assert!(!bridge.is_busy());
        assert_eq!(response.action, Action::Explain);
        assert!(response.result.is_ok());
        assert_eq!(response.result.unwrap(), "This lists files.");
    }

    #[test]
    fn t_bridge_request_suggest() {
        let mut bridge = AIBridge::with_mock("Try: cat file.txt");
        let ctx = sample_ctx();
        let req = AIRequest::new(Action::Suggest, ctx);
        assert!(bridge.request(req));

        let response = bridge.wait_result();
        assert_eq!(response.action, Action::Suggest);
        assert_eq!(response.result.unwrap(), "Try: cat file.txt");
    }

    #[test]
    fn t_bridge_request_error_help() {
        let mut bridge = AIBridge::with_mock("The error means...");
        let ctx = sample_ctx();
        let req = AIRequest::new(Action::ErrorHelp, ctx);
        assert!(bridge.request(req));

        let response = bridge.wait_result();
        assert_eq!(response.action, Action::ErrorHelp);
        assert!(response.result.is_ok());
    }

    #[test]
    fn t_bridge_request_nl2cmd() {
        let mut bridge = AIBridge::with_mock("find . -name '*.txt'");
        let ctx = sample_ctx();
        let req = AIRequest::nl2cmd("find all text files", ctx);
        assert!(bridge.request(req));

        let response = bridge.wait_result();
        assert_eq!(response.action, Action::NL2Command);
        assert!(response.result.is_ok());
    }

    #[test]
    fn t_bridge_request_rejected_when_busy() {
        let mut bridge = AIBridge::with_mock("response");
        let ctx = sample_ctx();
        let req1 = AIRequest::new(Action::Explain, ctx.clone());
        let req2 = AIRequest::new(Action::Suggest, ctx);
        assert!(bridge.request(req1));
        assert!(!bridge.request(req2), "second request should be rejected");
    }

    #[test]
    fn t_bridge_engine_taken_during_request() {
        let mut bridge = AIBridge::with_mock("response");
        let ctx = sample_ctx();
        let req = AIRequest::new(Action::Explain, ctx);
        bridge.request(req);
        assert!(bridge.engine().is_none(), "engine should be in worker thread");
    }

    #[test]
    fn t_bridge_engine_returned_after_result() {
        let mut bridge = AIBridge::with_mock("response");
        let ctx = sample_ctx();
        let req = AIRequest::new(Action::Explain, ctx);
        bridge.request(req);
        assert!(bridge.engine().is_none());

        bridge.wait_result();
        assert!(bridge.engine().is_some(), "engine should be returned");
    }

    #[test]
    fn t_bridge_multiple_sequential_requests() {
        let mut bridge = AIBridge::with_mock("answer");
        let ctx = sample_ctx();

        // First request
        bridge.request(AIRequest::new(Action::Explain, ctx.clone()));
        let r1 = bridge.wait_result();
        assert!(r1.result.is_ok());

        // Second request (engine should be available again)
        bridge.request(AIRequest::new(Action::Suggest, ctx));
        let r2 = bridge.wait_result();
        assert!(r2.result.is_ok());
    }

    // ── poll_result ──

    #[test]
    fn t_bridge_poll_empty_when_not_busy() {
        let mut bridge = AIBridge::with_mock("response");
        assert!(bridge.poll_result().is_none());
    }

    #[test]
    fn t_bridge_poll_returns_none_while_pending() {
        let mut bridge = AIBridge::with_mock("response");
        let ctx = sample_ctx();
        bridge.request(AIRequest::new(Action::Explain, ctx));
        // Immediately poll — likely no result yet
        let p = bridge.poll_result();
        // Might be Some on very fast machines, so just check it doesn't panic
        let _ = p;
    }

    #[test]
    fn t_bridge_poll_after_completion() {
        let mut bridge = AIBridge::with_mock("response");
        let ctx = sample_ctx();
        bridge.request(AIRequest::new(Action::Explain, ctx));

        // Wait for completion
        bridge.wait_result();
        // After wait, poll should return None (already consumed)
        assert!(bridge.poll_result().is_none());
        assert!(!bridge.is_busy());
    }

    // ── Error handling ──

    #[test]
    fn t_bridge_failing_engine() {
        let mut bridge = AIBridge::with_error(ggterm_ai::AIError::NoApiKey);
        let ctx = sample_ctx();
        bridge.request(AIRequest::new(Action::Explain, ctx));

        let response = bridge.wait_result();
        assert!(response.result.is_err());
        assert_eq!(response.result.unwrap_err(), ggterm_ai::AIError::NoApiKey);
    }

    #[test]
    fn t_bridge_empty_response() {
        let mut bridge = AIBridge::with_mock("");
        let ctx = sample_ctx();
        bridge.request(AIRequest::new(Action::Explain, ctx));

        let response = bridge.wait_result();
        assert!(response.result.is_err());
        assert_eq!(response.result.unwrap_err(), ggterm_ai::AIError::EmptyResponse);
    }

    #[test]
    fn t_bridge_error_then_success() {
        // First request fails, second succeeds
        let mut bridge = AIBridge::with_error(ggterm_ai::AIError::NoApiKey);
        let ctx = sample_ctx();
        bridge.request(AIRequest::new(Action::Explain, ctx.clone()));
        let r1 = bridge.wait_result();
        assert!(r1.result.is_err());

        // Replace engine with a working mock
        bridge = AIBridge::with_mock("success");
        bridge.request(AIRequest::new(Action::Suggest, ctx));
        let r2 = bridge.wait_result();
        assert!(r2.result.is_ok());
    }
}
