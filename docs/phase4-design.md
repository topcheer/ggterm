# Phase 4: AI Engine — Design Document

## Overview

GGTerm Phases 1-3 built a complete terminal emulator: VTE parser, grid model,
PTY integration, GPU rendering, shell integration (OSC 133), and command
navigation. Phase 4 adds the **AI Engine** — the key differentiator that
makes GGTerm "AI-native."

**Goal**: Provide AI-powered features that leverage command blocks, exit codes,
and terminal context to help users understand output, debug errors, and discover
commands.

## Architecture

New crate: `crates/ggterm-ai/`

```
crates/ggterm-ai/
├── Cargo.toml          # deps: ggterm-core, serde, serde_json, log
├── src/
│   ├── lib.rs          # Re-exports
│   ├── context.rs      # AIContext builder — extracts terminal state for LLM
│   ├── prompt.rs       # Prompt templates + ChatMessage/Role types
│   ├── engine.rs       # AIEngine — high-level API (LLMProvider trait)
│   └── client.rs       # LLM HTTP client (reqwest, OpenAI-compatible) [http feature]
```

### Data Flow

```
Terminal + CommandNavigator
    ↓
AIContext (context.rs)        ← extracts command blocks, exit codes, output
    ↓
Prompt Templates (prompt.rs)  ← builds system + user messages
    ↓
LLM Client (client.rs)        ← OpenAI-compatible HTTP + SSE streaming
    ↓
AIEngine (engine.rs)          ← high-level API: explain/suggest/error_help/nl2cmd
```

## Modules

### context.rs — AIContext

Extracts structured snapshot from Terminal state:

| Field | Type | Description |
|-------|------|-------------|
| `last_command` | `Option<String>` | Text of the most recent command |
| `last_exit_code` | `Option<i32>` | Exit code (None = still running) |
| `last_output` | `Option<String>` | Output text (truncated to budget) |
| `recent_commands` | `Vec<String>` | Up to N recent completed commands |
| `cwd` | `Option<String>` | Working directory |
| `shell` | `Option<String>` | Shell name |

**Budget-aware**: Output truncated to 2000 chars (configurable). History capped
at 10 commands.

### prompt.rs — Prompt Templates

- `Action::Explain` — "Explain the command output"
- `Action::Suggest` — "Suggest 3-5 useful next commands"
- `Action::ErrorHelp` — "The last command failed. Explain cause + fix"
- `Action::NL2Command` — "Translate natural language to shell command"

**Security**: System prompts include explicit rules:
1. Never suggest destructive commands without warning
2. Prefix dangerous suggestions with "WARNING:"
3. Prefer safe, non-destructive alternatives
4. Be concise

### engine.rs — AIEngine

High-level API with mockable `LLMProvider` trait:

```rust
pub trait LLMProvider: Send + Sync {
    fn complete(&self, messages: &[ChatMessage]) -> AIResult<String>;
}

pub struct AIEngine { /* owns Box<dyn LLMProvider> */ }

impl AIEngine {
    pub fn explain(&self, ctx: &AIContext) -> AIResult<String>;
    pub fn suggest(&self, ctx: &AIContext) -> AIResult<String>;
    pub fn error_help(&self, ctx: &AIContext) -> AIResult<String>;
    pub fn nl2cmd(&self, text: &str, ctx: &AIContext) -> AIResult<String>;
}
```

**Testing**: `MockLLM` returns canned responses. `FailingLLM` returns errors.
Both implement `LLMProvider`.

### client.rs — LLM HTTP Client (feature-gated)

OpenAI-compatible HTTP client with SSE streaming:

```rust
pub struct AIConfig {
    pub api_key: String,
    pub base_url: String,  // default: ZAI GLM endpoint
    pub model: String,     // default: glm-4-flash
    pub timeout: Duration,
    pub temperature: f32,
    pub max_tokens: Option<u32>,
}

pub struct LLMClient { /* reqwest::blocking::Client */ }

impl LLMClient {
    pub fn chat(&self, messages: &[ChatMessage]) -> Result<String, String>;
    pub fn chat_stream<F>(&self, messages: &[ChatMessage], on_delta: F) -> Result<String, String>;
}
```

**Environment variables**:
- `GGTERM_AI_API_KEY` (or `OPENAI_API_KEY` fallback)
- `GGTERM_AI_BASE_URL` (default: `https://open.bigmodel.cn/api/paas/v4`)
- `GGTERM_AI_MODEL` (default: `glm-4-flash`)
- `GGTERM_AI_TIMEOUT` (seconds, default: 60)
- `GGTERM_AI_TEMPERATURE` (default: 0.7)

**Compatible with**: OpenAI, DeepSeek, ZAI (GLM), local llama.cpp, any
OpenAI-style `/v1/chat/completions` or `/v4/chat/completions` endpoint.

## Feature Flags

```toml
[features]
default = []     # No HTTP deps — context + prompts + engine work standalone
http = ["dep:reqwest"]  # LLM HTTP client + SSE streaming
```

This means the AI context building and prompt construction work without any
HTTP dependencies. The `http` feature adds the live LLM client.

## Test Coverage

| Module | Tests | Description |
|--------|-------|-------------|
| context.rs | 19 | Context building, truncation, history, prompt string |
| prompt.rs | 14 | Message construction, system prompts, all 4 actions |
| engine.rs | 15 | Mock/failing providers, all actions, error handling |
| client.rs | 30 | Config from env, URL building, SSE parsing, API types |
| **Total** | **78** | All pass with `--features http` |

## Future Work (Phase 5+)

- Wire AIEngine into `ggterm-app` event loop (background AI worker thread)
- Add `AppEvent::AIRequest` / `AppEvent::AIResponse` events
- Terminal UI overlay for AI response display
- Streaming display (show tokens as they arrive)
- Multi-turn conversation (follow-up questions)
- Command history analysis patterns
