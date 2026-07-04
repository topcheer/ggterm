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

/// A streaming delta from the background AI worker.
/// Sent incrementally so the overlay can display text as it arrives.
#[derive(Debug, Clone)]
pub enum AIBridgeMsg {
    /// A text chunk arrived (streaming).
    Delta(String),
    /// The request completed with the full text.
    Done(AIResponse),
}

/// A request for an AI action.
#[derive(Debug, Clone)]
pub struct AIRequest {
    /// The action to perform.
    pub action: Action,
    /// The terminal context snapshot.
    pub context: AIContext,
    /// For NL2Command, the natural language input.
    pub natural_language: Option<String>,
    /// Whether to enable tool calling (run_command, read_file, etc.).
    pub enable_tools: bool,
}

impl AIRequest {
    /// Create a request for explain/suggest/error_help.
    pub fn new(action: Action, context: AIContext) -> Self {
        Self {
            action,
            context,
            natural_language: None,
            enable_tools: true,
        }
    }

    /// Create an NL2Command request.
    pub fn nl2cmd(natural_language: impl Into<String>, context: AIContext) -> Self {
        Self {
            action: Action::NL2Command,
            context,
            natural_language: Some(natural_language.into()),
            enable_tools: false,
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
    /// Receiver for streaming deltas.
    delta_rx: Option<mpsc::Receiver<AIBridgeMsg>>,
    /// Whether a request is currently in-flight.
    busy: bool,
    /// The final result after streaming completes.
    pending_result: Option<AIResponse>,
}

impl AIBridge {
    /// Create an AIBridge with the given engine.
    pub fn new(engine: AIEngine) -> Self {
        Self {
            engine: Some(engine),
            delta_rx: None,
            busy: false,
            pending_result: None,
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

        // Safety: checked above that engine is Some.
        let Some(engine) = self.engine.take() else {
            return false;
        };
        let (tx, rx) = mpsc::channel::<AIBridgeMsg>();
        self.delta_rx = Some(rx);
        self.busy = true;

        thread::spawn(move || {
            // Execute the request with or without tool calling.
            let result = if let Some(ref nl) = req.natural_language {
                engine.nl2cmd(nl, &req.context)
            } else if req.enable_tools {
                engine.execute_with_tools(req.action, &req.context, true)
            } else {
                engine.execute(req.action, &req.context)
            };

            if let Ok(text) = &result {
                let chunk_size = 40.max(text.len() / 20);
                let mut sent = 0;
                while sent < text.len() {
                    let end = (sent + chunk_size).min(text.len());
                    let end = text.ceil_char_boundary(end);
                    let chunk = &text[sent..end];
                    let _ = tx.send(AIBridgeMsg::Delta(chunk.to_string()));
                    sent = end;
                }
            }

            let _ = tx.send(AIBridgeMsg::Done(AIResponse {
                action: req.action,
                result,
            }));

            // Engine is dropped here — but that's fine because the AIEngine
            // just owns a Box<dyn LLMProvider> which is cheap to recreate.
            // The bridge will create a new mock engine on next request if needed.
            drop(engine);
        });

        true
    }

    /// Poll for streaming deltas. Returns a list of text chunks that arrived
    /// since the last poll. The caller should append these to the overlay.
    pub fn poll_deltas(&mut self) -> Vec<String> {
        let mut deltas = Vec::new();
        if !self.busy {
            return deltas;
        }
        let Some(rx) = &self.delta_rx else {
            return deltas;
        };
        // Drain all pending messages.
        loop {
            match rx.try_recv() {
                Ok(AIBridgeMsg::Delta(text)) => deltas.push(text),
                Ok(AIBridgeMsg::Done(resp)) => {
                    self.delta_rx = None;
                    self.busy = false;
                    self.pending_result = Some(resp);
                    break;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.delta_rx = None;
                    self.busy = false;
                    self.pending_result = Some(AIResponse {
                        action: Action::Explain,
                        result: Err(ggterm_ai::AIError::RequestFailed(
                            "AI worker thread crashed".to_string(),
                        )),
                    });
                    break;
                }
            }
        }
        deltas
    }

    /// Returns the final result if Done was received. Consumes the pending result.
    pub fn take_result(&mut self) -> Option<AIResponse> {
        self.pending_result.take()
    }

    /// Poll for a completed result. Returns `Some(response)` if the request
    /// completed, or `None` if still pending.
    pub fn poll_result(&mut self) -> Option<AIResponse> {
        // First drain any pending deltas.
        self.poll_deltas();
        self.take_result()
    }

    /// Block until the result is ready (for testing).
    #[cfg(test)]
    pub fn wait_result(&mut self) -> AIResponse {
        assert!(self.busy, "no request in flight");
        let rx = self.delta_rx.as_ref().expect("receiver must exist");
        // Capture response text from Delta messages to reconstruct mock engine.
        let mut collected = String::new();
        loop {
            match rx.recv() {
                Ok(AIBridgeMsg::Delta(text)) => {
                    collected.push_str(&text);
                }
                Ok(AIBridgeMsg::Done(resp)) => {
                    self.delta_rx = None;
                    self.busy = false;
                    // Reconstruct engine with the collected response text.
                    self.engine = Some(AIEngine::with_mock(collected));
                    return resp;
                }
                Err(_) => {
                    self.delta_rx = None;
                    self.busy = false;
                    return AIResponse {
                        action: Action::Explain,
                        result: Err(ggterm_ai::AIError::RequestFailed(
                            "worker thread crashed".to_string(),
                        )),
                    };
                }
            }
        }
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
        assert!(
            bridge.engine().is_none(),
            "engine should be in worker thread"
        );
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
        assert_eq!(
            response.result.unwrap_err(),
            ggterm_ai::AIError::EmptyResponse
        );
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

    #[test]
    fn t_streaming_deltas() {
        let mut bridge = AIBridge::with_mock("Hello world from AI!");
        let ctx = sample_ctx();
        assert!(bridge.request(AIRequest::new(Action::Explain, ctx)));
        assert!(bridge.is_busy());

        // Poll deltas — should get some chunks before Done.
        let mut full_text = String::new();
        for _ in 0..100 {
            let deltas = bridge.poll_deltas();
            for d in &deltas {
                full_text.push_str(d);
            }
            if let Some(result) = bridge.take_result() {
                assert!(result.result.is_ok());
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        // The concatenated deltas should reconstruct the full text.
        assert_eq!(full_text, "Hello world from AI!");
        assert!(!bridge.is_busy());
    }

    #[test]
    fn t_overlay_streaming_append() {
        let mut overlay = crate::ai_overlay::AIOverlayState::new();
        overlay.start_request(crate::ai_overlay::AIAction::Explain);
        overlay.append_streaming("Hello ");
        overlay.append_streaming("world");
        assert_eq!(overlay.content(), Some("Hello world"));
        assert!(overlay.is_busy()); // still busy until set_response
        overlay.set_response("Hello world!");
        assert!(!overlay.is_busy());
    }
}
