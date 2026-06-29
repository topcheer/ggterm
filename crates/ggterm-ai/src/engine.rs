//! AI Engine — high-level API tying context, prompts, and LLM client.
//!
//! The [`AIEngine`] is the main entry point for AI features.
//! It owns the LLM client and provides convenience methods that:
//! 1. Build context from terminal state
//! 2. Construct appropriate prompt messages
//! 3. Call the LLM (via the client)
//! 4. Return the response

use crate::context::AIContext;
use crate::prompt::{self, Action, ChatMessage};

use thiserror::Error;

/// Result type for AI operations.
pub type AIResult<T> = Result<T, AIError>;

/// Errors that can occur during AI operations.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AIError {
    /// No API key configured.
    #[error("no API key configured")]
    NoApiKey,
    /// The LLM request failed (network, HTTP, or parse error).
    #[error("request failed: {0}")]
    RequestFailed(String),
    /// The LLM returned an empty response.
    #[error("empty response from LLM")]
    EmptyResponse,
}

/// Trait for LLM clients (mockable for testing).
pub trait LLMProvider: Send + Sync {
    /// Send messages and get a completion.
    fn complete(&self, messages: &[ChatMessage]) -> AIResult<String>;
}

/// A mock LLM provider for testing.
pub struct MockLLM {
    /// The canned response to return.
    pub response: String,
}

impl MockLLM {
    pub fn new(response: impl Into<String>) -> Self {
        Self {
            response: response.into(),
        }
    }
}

impl LLMProvider for MockLLM {
    fn complete(&self, _messages: &[ChatMessage]) -> AIResult<String> {
        if self.response.is_empty() {
            return Err(AIError::EmptyResponse);
        }
        Ok(self.response.clone())
    }
}

/// A provider that always fails (for error testing).
pub struct FailingLLM {
    pub error: AIError,
}

impl FailingLLM {
    pub fn new(error: AIError) -> Self {
        Self { error }
    }
}

impl LLMProvider for FailingLLM {
    fn complete(&self, _messages: &[ChatMessage]) -> AIResult<String> {
        Err(self.error.clone())
    }
}

/// The AI engine. Owns the LLM client and provides high-level methods.
pub struct AIEngine {
    provider: Box<dyn LLMProvider>,
}

impl AIEngine {
    /// Create an engine with a custom LLM provider.
    pub fn with_provider(provider: Box<dyn LLMProvider>) -> Self {
        Self { provider }
    }

    /// Create an engine with a mock provider (for testing).
    pub fn with_mock(response: impl Into<String>) -> Self {
        Self::with_provider(Box::new(MockLLM::new(response)))
    }

    /// Create an engine that always fails (for testing error paths).
    pub fn failing(error: AIError) -> Self {
        Self::with_provider(Box::new(FailingLLM::new(error)))
    }

    /// Execute an AI action against terminal context.
    pub fn execute(&self, action: Action, ctx: &AIContext) -> AIResult<String> {
        let messages = prompt::build_messages(action, ctx);
        let response = self.provider.complete(&messages)?;
        if response.trim().is_empty() {
            return Err(AIError::EmptyResponse);
        }
        Ok(response)
    }

    /// Explain the last command's output.
    pub fn explain(&self, ctx: &AIContext) -> AIResult<String> {
        self.execute(Action::Explain, ctx)
    }

    /// Suggest next commands based on context.
    pub fn suggest(&self, ctx: &AIContext) -> AIResult<String> {
        self.execute(Action::Suggest, ctx)
    }

    /// Get help for a failed command.
    pub fn error_help(&self, ctx: &AIContext) -> AIResult<String> {
        self.execute(Action::ErrorHelp, ctx)
    }

    /// Translate natural language to a shell command.
    pub fn nl2cmd(&self, natural_language: &str, ctx: &AIContext) -> AIResult<String> {
        let messages = prompt::build_nl2cmd_messages(natural_language, ctx);
        let response = self.provider.complete(&messages)?;
        let trimmed = response.trim();
        if trimmed.is_empty() {
            return Err(AIError::EmptyResponse);
        }
        Ok(trimmed.to_string())
    }

    /// Return the raw messages for an action without calling the LLM.
    /// Useful for debugging or custom client usage.
    pub fn build_messages(&self, action: Action, ctx: &AIContext) -> Vec<ChatMessage> {
        prompt::build_messages(action, ctx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_ctx() -> AIContext {
        AIContext {
            last_command: Some("docker build .".to_string()),
            last_exit_code: Some(1),
            last_output: Some("Cannot connect to the Docker daemon".to_string()),
            recent_commands: vec!["docker ps".to_string()],
            cwd: Some("/app".to_string()),
            shell: Some("bash".to_string()),
        }
    }

    #[test]
    fn t_engine_explain_success() {
        let engine = AIEngine::with_mock("The build failed because Docker is not running.");
        let result = engine.explain(&sample_ctx()).unwrap();
        assert!(result.contains("Docker is not running"));
    }

    #[test]
    fn t_engine_suggest_success() {
        let engine = AIEngine::with_mock("1. docker ps\n2. docker images");
        let result = engine.suggest(&sample_ctx()).unwrap();
        assert!(result.contains("docker ps"));
    }

    #[test]
    fn t_engine_error_help_success() {
        let engine = AIEngine::with_mock("Cause: Docker not running\nFix:\n1. Start Docker");
        let result = engine.error_help(&sample_ctx()).unwrap();
        assert!(result.contains("Docker not running"));
    }

    #[test]
    fn t_engine_nl2cmd_success() {
        let engine = AIEngine::with_mock("find . -type f -size +100M");
        let result = engine
            .nl2cmd("find files larger than 100MB", &sample_ctx())
            .unwrap();
        assert_eq!(result, "find . -type f -size +100M");
    }

    #[test]
    fn t_engine_nl2cmd_trims_whitespace() {
        let engine = AIEngine::with_mock("  ls -la  \n");
        let result = engine.nl2cmd("list files", &sample_ctx()).unwrap();
        assert_eq!(result, "ls -la");
    }

    #[test]
    fn t_engine_request_failed() {
        let engine = AIEngine::failing(AIError::RequestFailed("network error".to_string()));
        let result = engine.explain(&sample_ctx());
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            AIError::RequestFailed("network error".to_string())
        );
    }

    #[test]
    fn t_engine_no_api_key() {
        let engine = AIEngine::failing(AIError::NoApiKey);
        let result = engine.suggest(&sample_ctx());
        assert_eq!(result.unwrap_err(), AIError::NoApiKey);
    }

    #[test]
    fn t_engine_empty_response() {
        let engine = AIEngine::failing(AIError::EmptyResponse);
        let result = engine.explain(&sample_ctx());
        assert_eq!(result.unwrap_err(), AIError::EmptyResponse);
    }

    #[test]
    fn t_engine_empty_mock_response() {
        let engine = AIEngine::with_mock("");
        let result = engine.explain(&sample_ctx());
        assert_eq!(result.unwrap_err(), AIError::EmptyResponse);
    }

    #[test]
    fn t_engine_whitespace_response() {
        let engine = AIEngine::with_mock("   \n  \t  ");
        let result = engine.explain(&sample_ctx());
        assert_eq!(result.unwrap_err(), AIError::EmptyResponse);
    }

    #[test]
    fn t_engine_build_messages_debug() {
        let engine = AIEngine::with_mock("test");
        let msgs = engine.build_messages(Action::Explain, &sample_ctx());
        assert_eq!(msgs.len(), 2);
        assert!(msgs[1].content.contains("docker build"));
    }

    #[test]
    fn t_ai_error_display() {
        assert_eq!(AIError::NoApiKey.to_string(), "no API key configured");
        assert_eq!(
            AIError::RequestFailed("timeout".to_string()).to_string(),
            "request failed: timeout"
        );
        assert_eq!(
            AIError::EmptyResponse.to_string(),
            "empty response from LLM"
        );
    }

    #[test]
    fn t_mock_llm_returns_canned_response() {
        let mock = MockLLM::new("hello world");
        let result = mock.complete(&[]);
        assert_eq!(result.unwrap(), "hello world");
    }

    #[test]
    fn t_failing_llm_returns_error() {
        let failing = FailingLLM::new(AIError::NoApiKey);
        let result = failing.complete(&[]);
        assert_eq!(result.unwrap_err(), AIError::NoApiKey);
    }

    #[test]
    fn t_engine_execute_all_actions() {
        let engine = AIEngine::with_mock("generic response");
        let ctx = sample_ctx();
        for action in [Action::Explain, Action::Suggest, Action::ErrorHelp] {
            let result = engine.execute(action, &ctx);
            assert!(result.is_ok(), "action {action:?} should succeed");
        }
    }
}
