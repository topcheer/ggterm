//! Prompt templates for AI features.
//!
//! Each function returns a `Vec<ChatMessage>` (system + user messages)
//! ready to send to an LLM.
//!
//! All prompts are security-conscious: the system message instructs the
//! LLM to never suggest destructive commands without explicit warnings.

use crate::context::AIContext;

/// A role for a chat message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

impl Role {
    /// Convert to the OpenAI API string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        }
    }
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A single message in a chat conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
}

impl ChatMessage {
    /// Create a system message.
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
        }
    }

    /// Create a user message.
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }

    /// Create an assistant message.
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
        }
    }

    /// Create a tool result message.
    pub fn tool_result(content: impl Into<String>, _tool_call_id: &str) -> Self {
        Self {
            role: Role::Tool,
            content: content.into(),
        }
    }
}

/// The type of AI action to perform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Explain the last command's output.
    Explain,
    /// Suggest next commands based on context.
    Suggest,
    /// Explain why a command failed and suggest fixes.
    ErrorHelp,
    /// Translate natural language to a shell command.
    NL2Command,
}

/// Base system prompt shared by all actions.
const BASE_SYSTEM_PROMPT: &str = concat!(
    "You are an AI assistant integrated into a terminal emulator. ",
    "You help users understand command output, debug errors, and discover useful commands.\n\n",
    "IMPORTANT SECURITY RULES:\n",
    "1. NEVER suggest destructive commands (rm -rf, dd, mkfs, fork bombs) without an explicit warning.\n",
    "2. If a command might cause data loss, prefix your suggestion with 'WARNING:'.\n",
    "3. Always prefer safe, non-destructive alternatives.\n",
    "4. Be concise — terminal users value brevity.",
);

/// Build the message list for a given action and context.
///
/// Returns a `Vec<ChatMessage>` starting with the system message,
/// followed by the user message containing the context and request.
pub fn build_messages(action: Action, ctx: &AIContext) -> Vec<ChatMessage> {
    let system = build_system_prompt(action);
    let user = build_user_prompt(action, ctx);
    vec![ChatMessage::system(system), ChatMessage::user(user)]
}

/// Build the system prompt for a given action.
pub fn build_system_prompt(action: Action) -> String {
    let suffix = match action {
        Action::Explain => concat!(
            " Your task: Explain the command output in clear, concise terms. ",
            "If the output contains an error, explain what went wrong. ",
            "Format your response as plain text, not markdown.",
        ),
        Action::Suggest => concat!(
            " Your task: Suggest 3-5 useful next commands based on the context. ",
            "Format your response as a numbered list, one command per line, with a brief explanation.\n",
            "Example:\n",
            "1. git add -A  — Stage all changes\n",
            "2. git commit -m 'message'  — Commit staged changes",
        ),
        Action::ErrorHelp => concat!(
            " Your task: The last command failed. Explain why it failed and suggest fixes.\n",
            "Format:\n",
            "Cause: <brief explanation>\n",
            "Fix:\n",
            "1. <step 1>\n",
            "2. <step 2>",
        ),
        Action::NL2Command => concat!(
            " Your task: Translate the user's natural language request into a shell command. ",
            "Respond with ONLY the command — no explanation, no markdown, no backticks. ",
            "If the request is ambiguous, provide the most likely command and note assumptions.",
        ),
    };
    format!("{BASE_SYSTEM_PROMPT}{suffix}")
}

/// Build the user prompt for a given action and context.
pub fn build_user_prompt(action: Action, ctx: &AIContext) -> String {
    let context_str = ctx.to_prompt_string();
    let instruction = match action {
        Action::Explain => "Explain the output of the last command.",
        Action::Suggest => "Suggest useful next commands.",
        Action::ErrorHelp => "The last command failed. Help me understand and fix the error.",
        Action::NL2Command => "Translate my request into a shell command.",
    };
    format!("{context_str}\n\n---\n\nRequest: {instruction}")
}

/// Build messages for NL2Command with the user's natural language text.
pub fn build_nl2cmd_messages(natural_language: &str, ctx: &AIContext) -> Vec<ChatMessage> {
    let system = build_system_prompt(Action::NL2Command);
    let context_str = ctx.to_prompt_string();
    let user = format!(
        "{context_str}\n\n---\n\nMy request: {natural_language}\n\nProvide ONLY the shell command, nothing else."
    );
    vec![ChatMessage::system(system), ChatMessage::user(user)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_ctx() -> AIContext {
        AIContext {
            last_command: Some("git push".to_string()),
            last_exit_code: Some(1),
            last_output: Some("error: failed to push some refs".to_string()),
            recent_commands: vec![
                "git add -A".to_string(),
                "git commit -m \"init\"".to_string(),
            ],
            cwd: Some("/home/user/project".to_string()),
            shell: Some("zsh".to_string()),
        }
    }

    #[test]
    fn t_role_as_str() {
        assert_eq!(Role::System.as_str(), "system");
        assert_eq!(Role::User.as_str(), "user");
        assert_eq!(Role::Assistant.as_str(), "assistant");
    }

    #[test]
    fn t_role_display() {
        assert_eq!(format!("{}", Role::System), "system");
        assert_eq!(format!("{}", Role::User), "user");
    }

    #[test]
    fn t_chat_message_constructors() {
        let s = ChatMessage::system("hello");
        assert_eq!(s.role, Role::System);
        assert_eq!(s.content, "hello");

        let u = ChatMessage::user("world");
        assert_eq!(u.role, Role::User);

        let a = ChatMessage::assistant("ok");
        assert_eq!(a.role, Role::Assistant);
    }

    #[test]
    fn t_build_messages_explain() {
        let ctx = sample_ctx();
        let msgs = build_messages(Action::Explain, &ctx);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, Role::System);
        assert_eq!(msgs[1].role, Role::User);
        assert!(msgs[0].content.contains("Explain the command output"));
        assert!(msgs[1].content.contains("git push"));
        assert!(msgs[1].content.contains("Explain the output"));
    }

    #[test]
    fn t_build_messages_suggest() {
        let ctx = sample_ctx();
        let msgs = build_messages(Action::Suggest, &ctx);
        assert!(msgs[0].content.contains("Suggest 3-5 useful next commands"));
        assert!(msgs[1].content.contains("Suggest useful next commands"));
    }

    #[test]
    fn t_build_messages_error_help() {
        let ctx = sample_ctx();
        let msgs = build_messages(Action::ErrorHelp, &ctx);
        assert!(msgs[0].content.contains("failed"));
        assert!(msgs[1].content.contains("failed"));
    }

    #[test]
    fn t_build_messages_nl2cmd() {
        let ctx = sample_ctx();
        let msgs = build_messages(Action::NL2Command, &ctx);
        assert!(msgs[0].content.contains("ONLY the command"));
    }

    #[test]
    fn t_build_nl2cmd_with_text() {
        let ctx = sample_ctx();
        let msgs = build_nl2cmd_messages("list all files larger than 100MB", &ctx);
        assert_eq!(msgs.len(), 2);
        assert!(msgs[1].content.contains("list all files larger than 100MB"));
        assert!(msgs[1].content.contains("ONLY the shell command"));
    }

    #[test]
    fn t_system_prompt_contains_security_rules() {
        let prompt = build_system_prompt(Action::Explain);
        assert!(prompt.contains("SECURITY RULES"));
        assert!(prompt.contains("rm -rf"));
        assert!(prompt.contains("WARNING"));
    }

    #[test]
    fn t_user_prompt_contains_context() {
        let ctx = sample_ctx();
        let prompt = build_user_prompt(Action::Explain, &ctx);
        assert!(prompt.contains("Shell: zsh"));
        assert!(prompt.contains("Working directory: /home/user/project"));
        assert!(prompt.contains("git push"));
    }

    #[test]
    fn t_user_prompt_explain_instruction() {
        let ctx = AIContext::default();
        let prompt = build_user_prompt(Action::Explain, &ctx);
        assert!(prompt.contains("Explain the output"));
    }

    #[test]
    fn t_messages_always_start_with_system() {
        let ctx = sample_ctx();
        for action in [
            Action::Explain,
            Action::Suggest,
            Action::ErrorHelp,
            Action::NL2Command,
        ] {
            let msgs = build_messages(action, &ctx);
            assert_eq!(
                msgs[0].role,
                Role::System,
                "action {action:?} should start with system message"
            );
        }
    }

    #[test]
    fn t_messages_always_have_two_messages() {
        let ctx = sample_ctx();
        for action in [
            Action::Explain,
            Action::Suggest,
            Action::ErrorHelp,
            Action::NL2Command,
        ] {
            let msgs = build_messages(action, &ctx);
            assert_eq!(msgs.len(), 2);
        }
    }

    #[test]
    fn t_empty_context_still_builds() {
        let ctx = AIContext::default();
        let msgs = build_messages(Action::Explain, &ctx);
        assert_eq!(msgs.len(), 2);
        // Should contain "(none)" for last command
        assert!(msgs[1].content.contains("(none)"));
    }
}
