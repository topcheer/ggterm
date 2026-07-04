//! LLM HTTP client — OpenAI-compatible API with SSE streaming support.
//!
//! Works with any OpenAI-style `/v1/chat/completions` endpoint:
//! OpenAI, DeepSeek, ZAI (GLM), local llama.cpp, etc.
//!
//! Configuration via environment variables:
//! - `GGTERM_AI_API_KEY` — API key (required)
//! - `GGTERM_AI_BASE_URL` — base URL (default: `https://open.bigmodel.cn/api/paas/v4`)
//! - `GGTERM_AI_MODEL` — model name (default: `glm-4-flash`)

use std::time::Duration;

use log::{debug, warn};
use serde::{Deserialize, Serialize};

use crate::prompt::ChatMessage;
use crate::tools::Tool;

// Re-export ChatMessage/Role for convenience
pub use crate::prompt::{ChatMessage as PromptMessage, Role as MsgRole};

/// Configuration for the LLM client.
#[derive(Debug, Clone)]
pub struct AIConfig {
    /// API key for authentication.
    pub api_key: String,
    /// Base URL (e.g. `https://api.openai.com/v1`).
    pub base_url: String,
    /// Model name (e.g. `gpt-4o`, `glm-4-flash`).
    pub model: String,
    /// Request timeout.
    pub timeout: Duration,
    /// Temperature (0.0 = deterministic, 1.0 = creative).
    pub temperature: f32,
    /// Maximum tokens to generate.
    pub max_tokens: Option<u32>,
}

impl Default for AIConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            base_url: "https://open.bigmodel.cn/api/paas/v4".to_string(),
            model: "glm-4-flash".to_string(),
            timeout: Duration::from_secs(60),
            temperature: 0.7,
            max_tokens: Some(2048),
        }
    }
}

impl AIConfig {
    /// Load configuration from environment variables.
    ///
    /// Reads:
    /// - `GGTERM_AI_API_KEY` (or `OPENAI_API_KEY` as fallback)
    /// - `GGTERM_AI_BASE_URL` (default: ZAI GLM endpoint)
    /// - `GGTERM_AI_MODEL` (default: `glm-4-flash`)
    /// - `GGTERM_AI_TIMEOUT` (seconds, default: 60)
    /// - `GGTERM_AI_TEMPERATURE` (default: 0.7)
    pub fn from_env() -> Self {
        let mut config = AIConfig::default();

        if let Ok(key) = std::env::var("GGTERM_AI_API_KEY") {
            config.api_key = key;
        } else if let Ok(key) = std::env::var("OPENAI_API_KEY") {
            config.api_key = key;
        }

        if let Ok(url) = std::env::var("GGTERM_AI_BASE_URL") {
            config.base_url = url;
        }

        if let Ok(model) = std::env::var("GGTERM_AI_MODEL") {
            config.model = model;
        }

        if let Ok(timeout_str) = std::env::var("GGTERM_AI_TIMEOUT")
            && let Ok(secs) = timeout_str.parse::<u64>()
        {
            config.timeout = Duration::from_secs(secs);
        }

        if let Ok(temp_str) = std::env::var("GGTERM_AI_TEMPERATURE")
            && let Ok(temp) = temp_str.parse::<f32>()
        {
            config.temperature = temp;
        }

        config
    }

    /// Check if the API key is set.
    pub fn has_api_key(&self) -> bool {
        !self.api_key.is_empty()
    }

    /// Return the chat completions URL.
    pub fn chat_url(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        if base.ends_with("/v1") || base.ends_with("/v4") {
            format!("{base}/chat/completions")
        } else {
            format!("{base}/v1/chat/completions")
        }
    }

    /// Return a masked version of the API key for logging.
    pub fn masked_api_key(&self) -> String {
        if self.api_key.len() <= 8 {
            return "***".to_string();
        }
        let first4 = &self.api_key[..4];
        let last4 = &self.api_key[self.api_key.len() - 4..];
        format!("{first4}...{last4}")
    }
}

// --- OpenAI API types ---

#[derive(Debug, Serialize)]
struct ApiRequest {
    model: String,
    messages: Vec<ApiMessage>,
    temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    stream: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<Tool>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ApiMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct ApiResponse {
    choices: Vec<ApiChoice>,
}

#[derive(Debug, Deserialize)]
struct ApiChoice {
    message: Option<ApiMessage>,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ApiStreamChunk {
    choices: Vec<ApiStreamChoice>,
}

#[derive(Debug, Deserialize)]
struct ApiStreamChoice {
    delta: Option<ApiDelta>,
}

#[derive(Debug, Deserialize)]
struct ApiDelta {
    content: Option<String>,
}

/// The LLM HTTP client.
pub struct LLMClient {
    config: AIConfig,
    client: reqwest::blocking::Client,
}

impl LLMClient {
    /// Create a new client from configuration.
    pub fn new(config: AIConfig) -> Self {
        let client = reqwest::blocking::Client::builder()
            .timeout(config.timeout)
            .build()
            .expect("failed to build HTTP client");

        Self { config, client }
    }

    /// Create a client from environment variables.
    pub fn from_env() -> Self {
        Self::new(AIConfig::from_env())
    }

    /// Send a chat completion request and return the full response.
    pub fn chat(&self, messages: &[ChatMessage]) -> Result<String, String> {
        self.chat_with_tools(messages, &[])
    }

    /// Send a chat completion with tool definitions.
    ///
    /// Returns the text content. If the LLM requests tool calls,
    /// the `tool_calls` JSON is embedded in the response prefixed with
    /// `__TOOL_CALLS__:`.
    pub fn chat_with_tools(
        &self,
        messages: &[ChatMessage],
        tools: &[Tool],
    ) -> Result<String, String> {
        if !self.config.has_api_key() {
            return Err("no API key configured".to_string());
        }

        let api_messages: Vec<ApiMessage> = messages
            .iter()
            .map(|m| ApiMessage {
                role: m.role.as_str().to_string(),
                content: m.content.clone(),
            })
            .collect();

        let request = ApiRequest {
            model: self.config.model.clone(),
            messages: api_messages,
            temperature: self.config.temperature,
            max_tokens: self.config.max_tokens,
            stream: false,
            tools: tools.to_vec(),
        };

        let url = self.config.chat_url();
        debug!(
            "LLM request to {url} (model: {}, key: {}, tools: {})",
            self.config.model,
            self.config.masked_api_key(),
            tools.len()
        );

        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .map_err(|e| format!("HTTP error: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            warn!("LLM request failed: {status} — {body}");
            return Err(format!("HTTP {status}: {body}"));
        }

        let api_resp: ApiResponse = resp.json().map_err(|e| format!("JSON parse error: {e}"))?;

        let choice = api_resp
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| "empty response from API".to_string())?;

        // Check for tool calls
        if let Some(reason) = &choice.finish_reason
            && reason == "tool_calls"
        {
            // Return marker so caller knows to handle tool calls
            return Ok(format!(
                "__TOOL_CALLS__:{}",
                choice.message.map(|m| m.content).unwrap_or_default()
            ));
        }

        choice
            .message
            .map(|m| m.content)
            .ok_or_else(|| "empty response from API".to_string())
    }

    /// Send a streaming chat completion, calling `on_delta` for each chunk.
    ///
    /// Returns the full concatenated response.
    pub fn chat_stream<F>(
        &self,
        messages: &[ChatMessage],
        mut on_delta: F,
    ) -> Result<String, String>
    where
        F: FnMut(&str),
    {
        if !self.config.has_api_key() {
            return Err("no API key configured".to_string());
        }

        let api_messages: Vec<ApiMessage> = messages
            .iter()
            .map(|m| ApiMessage {
                role: m.role.as_str().to_string(),
                content: m.content.clone(),
            })
            .collect();

        let request = ApiRequest {
            model: self.config.model.clone(),
            messages: api_messages,
            temperature: self.config.temperature,
            max_tokens: self.config.max_tokens,
            stream: true,
            tools: vec![],
        };

        let url = self.config.chat_url();

        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .map_err(|e| format!("HTTP error: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            return Err(format!("HTTP {status}: {body}"));
        }

        let body = resp.bytes().map_err(|e| format!("read error: {e}"))?;
        let text = String::from_utf8_lossy(&body);
        let mut full_response = String::new();

        for line in text.lines() {
            if let Some(data) = line.strip_prefix("data: ") {
                if data == "[DONE]" {
                    break;
                }
                if let Ok(chunk) = serde_json::from_str::<ApiStreamChunk>(data)
                    && let Some(choice) = chunk.choices.into_iter().next()
                    && let Some(delta) = choice.delta
                    && let Some(content) = delta.content
                {
                    on_delta(&content);
                    full_response.push_str(&content);
                }
            }
        }

        if full_response.is_empty() {
            return Err("empty streaming response".to_string());
        }

        Ok(full_response)
    }
}

/// Implement LLMProvider trait for LLMClient so it can be used with AIEngine.
impl crate::engine::LLMProvider for LLMClient {
    fn complete(&self, messages: &[ChatMessage]) -> crate::engine::AIResult<String> {
        self.chat(messages)
            .map_err(crate::engine::AIError::RequestFailed)
    }

    fn complete_with_tools(
        &self,
        messages: &[ChatMessage],
        tools: &[Tool],
    ) -> crate::engine::AIResult<crate::engine::CompletionResponse> {
        let response = self
            .chat_with_tools(messages, tools)
            .map_err(crate::engine::AIError::RequestFailed)?;

        // Check for tool calls marker.
        if let Some(json) = response.strip_prefix("__TOOL_CALLS__:") {
            let tool_calls: Vec<crate::tools::ToolCall> =
                serde_json::from_str(json).unwrap_or_default();
            return Ok(crate::engine::CompletionResponse::ToolCalls {
                content: String::new(),
                tool_calls,
            });
        }

        Ok(crate::engine::CompletionResponse::Text(response))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mutex to serialize tests that touch global env vars, preventing races.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// RAII guard that holds the env mutex for the duration of a test.
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn t_config_default() {
        let config = AIConfig::default();
        assert!(!config.has_api_key());
        assert!(!config.base_url.is_empty());
        assert!(!config.model.is_empty());
    }

    // Helper: safely set env var (Rust 2024 makes set_var unsafe).
    fn set_env(key: &str, val: &str) {
        unsafe {
            std::env::set_var(key, val);
        }
    }

    // Helper: safely remove env var (Rust 2024 makes remove_var unsafe).
    fn remove_env(key: &str) {
        unsafe {
            std::env::remove_var(key);
        }
    }

    #[test]
    fn t_config_from_env_no_vars() {
        let _g = env_lock();
        // Clear any existing env vars
        remove_env("GGTERM_AI_API_KEY");
        remove_env("OPENAI_API_KEY");

        let config = AIConfig::from_env();
        assert!(!config.has_api_key());
    }

    #[test]
    fn t_config_from_env_with_key() {
        let _g = env_lock();
        set_env("GGTERM_AI_API_KEY", "test-key-1234567890");
        let config = AIConfig::from_env();
        assert!(config.has_api_key());
        assert_eq!(config.api_key, "test-key-1234567890");
        remove_env("GGTERM_AI_API_KEY");
    }

    #[test]
    fn t_config_openai_fallback() {
        let _g = env_lock();
        remove_env("GGTERM_AI_API_KEY");
        set_env("OPENAI_API_KEY", "sk-fallback-key");
        let config = AIConfig::from_env();
        assert!(config.has_api_key());
        assert_eq!(config.api_key, "sk-fallback-key");
        remove_env("OPENAI_API_KEY");
    }

    #[test]
    fn t_config_ggterm_takes_priority() {
        let _g = env_lock();
        set_env("GGTERM_AI_API_KEY", "primary-key");
        set_env("OPENAI_API_KEY", "fallback-key");
        let config = AIConfig::from_env();
        assert_eq!(config.api_key, "primary-key");
        remove_env("GGTERM_AI_API_KEY");
        remove_env("OPENAI_API_KEY");
    }

    #[test]
    fn t_config_custom_base_url() {
        let _g = env_lock();
        set_env("GGTERM_AI_BASE_URL", "https://api.deepseek.com/v1");
        let config = AIConfig::from_env();
        assert_eq!(config.base_url, "https://api.deepseek.com/v1");
        remove_env("GGTERM_AI_BASE_URL");
    }

    #[test]
    fn t_config_custom_model() {
        let _g = env_lock();
        set_env("GGTERM_AI_MODEL", "gpt-4o");
        let config = AIConfig::from_env();
        assert_eq!(config.model, "gpt-4o");
        remove_env("GGTERM_AI_MODEL");
    }

    #[test]
    fn t_config_custom_timeout() {
        let _g = env_lock();
        set_env("GGTERM_AI_TIMEOUT", "30");
        let config = AIConfig::from_env();
        assert_eq!(config.timeout, Duration::from_secs(30));
        remove_env("GGTERM_AI_TIMEOUT");
    }

    #[test]
    fn t_config_invalid_timeout_falls_back() {
        let _g = env_lock();
        set_env("GGTERM_AI_TIMEOUT", "not-a-number");
        let config = AIConfig::from_env();
        assert_eq!(config.timeout, Duration::from_secs(60));
        remove_env("GGTERM_AI_TIMEOUT");
    }

    #[test]
    fn t_config_custom_temperature() {
        let _g = env_lock();
        set_env("GGTERM_AI_TEMPERATURE", "0.1");
        let config = AIConfig::from_env();
        assert!((config.temperature - 0.1).abs() < 0.01);
        remove_env("GGTERM_AI_TEMPERATURE");
    }

    #[test]
    fn t_config_chat_url_v1() {
        let config = AIConfig {
            base_url: "https://api.openai.com/v1".to_string(),
            ..Default::default()
        };
        assert_eq!(
            config.chat_url(),
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn t_config_chat_url_v4() {
        let config = AIConfig {
            base_url: "https://open.bigmodel.cn/api/paas/v4".to_string(),
            ..Default::default()
        };
        assert_eq!(
            config.chat_url(),
            "https://open.bigmodel.cn/api/paas/v4/chat/completions"
        );
    }

    #[test]
    fn t_config_chat_url_appends_v1() {
        let config = AIConfig {
            base_url: "https://custom.api.com".to_string(),
            ..Default::default()
        };
        assert_eq!(
            config.chat_url(),
            "https://custom.api.com/v1/chat/completions"
        );
    }

    #[test]
    fn t_config_chat_url_trims_trailing_slash() {
        let config = AIConfig {
            base_url: "https://api.openai.com/v1/".to_string(),
            ..Default::default()
        };
        assert_eq!(
            config.chat_url(),
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn t_config_masked_key_long() {
        let config = AIConfig {
            api_key: "sk-abcdefghijklmnop1234567890".to_string(),
            ..Default::default()
        };
        let masked = config.masked_api_key();
        assert!(masked.starts_with("sk-a"));
        assert!(masked.ends_with("7890"));
        assert!(masked.contains("..."));
    }

    #[test]
    fn t_config_masked_key_short() {
        let config = AIConfig {
            api_key: "short".to_string(),
            ..Default::default()
        };
        assert_eq!(config.masked_api_key(), "***");
    }

    #[test]
    fn t_config_has_api_key_empty() {
        let config = AIConfig::default();
        assert!(!config.has_api_key());
    }

    #[test]
    fn t_config_has_api_key_set() {
        let config = AIConfig {
            api_key: "some-key".to_string(),
            ..Default::default()
        };
        assert!(config.has_api_key());
    }

    // --- SSE parsing tests ---

    #[test]
    fn t_sse_parse_single_chunk() {
        let json = r#"{"choices":[{"delta":{"content":"hello"}}]}"#;
        let chunk: ApiStreamChunk = serde_json::from_str(json).unwrap();
        assert_eq!(chunk.choices.len(), 1);
        assert_eq!(
            chunk.choices[0].delta.as_ref().unwrap().content.as_deref(),
            Some("hello")
        );
    }

    #[test]
    fn t_sse_parse_no_content() {
        let json = r#"{"choices":[{"delta":{}}]}"#;
        let chunk: ApiStreamChunk = serde_json::from_str(json).unwrap();
        assert!(chunk.choices[0].delta.as_ref().unwrap().content.is_none());
    }

    #[test]
    fn t_sse_parse_multiple_choices() {
        let json = r#"{"choices":[{"delta":{"content":"a"}},{"delta":{"content":"b"}}]}"#;
        let chunk: ApiStreamChunk = serde_json::from_str(json).unwrap();
        assert_eq!(chunk.choices.len(), 2);
    }

    #[test]
    fn t_sse_parse_empty_choices() {
        let json = r#"{"choices":[]}"#;
        let chunk: ApiStreamChunk = serde_json::from_str(json).unwrap();
        assert!(chunk.choices.is_empty());
    }

    #[test]
    fn t_sse_parse_invalid_json() {
        let json = r#"not json"#;
        let result: Result<ApiStreamChunk, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    // --- API response parsing ---

    #[test]
    fn t_api_response_parse() {
        let json = r#"{"choices":[{"message":{"role":"assistant","content":"hello world"}}]}"#;
        let resp: ApiResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.choices.len(), 1);
        assert_eq!(
            resp.choices[0].message.as_ref().unwrap().content,
            "hello world"
        );
    }

    #[test]
    fn t_api_response_empty_choices() {
        let json = r#"{"choices":[]}"#;
        let resp: ApiResponse = serde_json::from_str(json).unwrap();
        assert!(resp.choices.is_empty());
    }

    #[test]
    fn t_api_response_null_message() {
        let json = r#"{"choices":[{}]}"#;
        let resp: ApiResponse = serde_json::from_str(json).unwrap();
        assert!(resp.choices[0].message.is_none());
    }

    #[test]
    fn t_api_request_serialization() {
        let request = ApiRequest {
            model: "gpt-4".to_string(),
            messages: vec![ApiMessage {
                role: "user".to_string(),
                content: "hi".to_string(),
            }],
            temperature: 0.5,
            max_tokens: Some(100),
            stream: false,
            tools: vec![],
        };
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"model\":\"gpt-4\""));
        assert!(json.contains("\"temperature\":0.5"));
        assert!(json.contains("\"max_tokens\":100"));
        assert!(json.contains("\"stream\":false"));
    }

    #[test]
    fn t_api_request_skip_max_tokens_when_none() {
        let request = ApiRequest {
            model: "gpt-4".to_string(),
            messages: vec![],
            temperature: 0.5,
            max_tokens: None,
            stream: false,
            tools: vec![],
        };
        let json = serde_json::to_string(&request).unwrap();
        assert!(!json.contains("max_tokens"));
    }
}
