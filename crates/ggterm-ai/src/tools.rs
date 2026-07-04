//! Tool definitions for AI function calling.
//!
//! Allows the AI to execute terminal tools (run_command, read_file, list_files)
//! during a conversation. The flow is:
//!
//! 1. Client sends messages + tool definitions to LLM
//! 2. LLM responds with `tool_calls` instead of (or alongside) text
//! 3. Client executes the tools locally
//! 4. Client sends tool results back to LLM for a final response

use serde::{Deserialize, Serialize};

/// A tool definition sent to the LLM.
#[derive(Debug, Clone, Serialize)]
pub struct Tool {
    /// The type — always "function" for OpenAI-compatible APIs.
    #[serde(rename = "type")]
    pub tool_type: String,
    /// The function definition.
    pub function: ToolFunction,
}

/// The function part of a tool definition.
#[derive(Debug, Clone, Serialize)]
pub struct ToolFunction {
    /// Function name (e.g. "run_command").
    pub name: String,
    /// Human-readable description for the LLM.
    pub description: String,
    /// JSON schema for parameters (as a serde_json::Value).
    pub parameters: serde_json::Value,
}

/// A tool call from the LLM response.
#[derive(Debug, Clone, Deserialize)]
pub struct ToolCall {
    /// Unique ID for this call (used to match results).
    pub id: String,
    /// Always "function".
    #[serde(rename = "type")]
    pub call_type: String,
    /// The function call details.
    pub function: ToolCallFunction,
}

/// The function part of a tool call.
#[derive(Debug, Clone, Deserialize)]
pub struct ToolCallFunction {
    /// Function name to invoke.
    pub name: String,
    /// JSON-encoded arguments string.
    pub arguments: String,
}

/// Result of executing a tool.
#[derive(Debug, Clone)]
pub struct ToolResult {
    /// The tool call ID this result corresponds to.
    pub tool_call_id: String,
    /// The output text (stdout for commands, file contents, etc.).
    pub content: String,
}

// ── Built-in terminal tools ──────────────────────────────────────

/// Get the built-in terminal tool definitions.
///
/// These tools let the AI interact with the local system:
/// - `run_command` — execute a shell command and return stdout
/// - `read_file` — read file contents (with size limit)
/// - `list_files` — list directory entries
pub fn builtin_tools() -> Vec<Tool> {
    vec![
        Tool {
            tool_type: "function".to_string(),
            function: ToolFunction {
                name: "run_command".to_string(),
                description: "Execute a shell command and return its stdout/stderr output. \
                    Use this to inspect the system state, run diagnostics, or gather information \
                    the user is asking about. Commands run with the current user's permissions."
                    .to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "command": {
                            "type": "string",
                            "description": "The shell command to execute"
                        }
                    },
                    "required": ["command"]
                }),
            },
        },
        Tool {
            tool_type: "function".to_string(),
            function: ToolFunction {
                name: "read_file".to_string(),
                description: "Read the contents of a file. Maximum 4000 characters returned. \
                    Use this to inspect configuration files, logs, or source code."
                    .to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Path to the file to read (relative or absolute)"
                        }
                    },
                    "required": ["path"]
                }),
            },
        },
        Tool {
            tool_type: "function".to_string(),
            function: ToolFunction {
                name: "list_files".to_string(),
                description: "List files and directories at the given path. \
                    Returns names with type indicators (/ for directories)."
                    .to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Directory path to list (default: current directory)"
                        }
                    },
                    "required": []
                }),
            },
        },
    ]
}

/// Execute a tool call locally.
///
/// # Safety
/// This executes arbitrary shell commands. The caller is responsible for
/// any confirmation UI before calling this function.
pub fn execute_tool(call: &ToolCall) -> ToolResult {
    let args: serde_json::Value =
        serde_json::from_str(&call.function.arguments).unwrap_or(serde_json::Value::Null);

    let output = match call.function.name.as_str() {
        "run_command" => {
            let cmd = args.get("command").and_then(|v| v.as_str()).unwrap_or("");
            run_shell_command(cmd)
        }
        "read_file" => {
            let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
            read_file(path)
        }
        "list_files" => {
            let path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
            list_files(path)
        }
        other => format!("Unknown tool: {other}"),
    };

    ToolResult {
        tool_call_id: call.id.clone(),
        content: output,
    }
}

/// Execute a shell command and return combined stdout+stderr (truncated).
fn run_shell_command(cmd: &str) -> String {
    if cmd.is_empty() {
        return "Error: empty command".to_string();
    }

    let output = std::process::Command::new("sh").arg("-c").arg(cmd).output();

    match output {
        Ok(out) => {
            let mut combined = String::new();
            if !out.stdout.is_empty() {
                combined.push_str(&String::from_utf8_lossy(&out.stdout));
            }
            if !out.stderr.is_empty() {
                if !combined.is_empty() {
                    combined.push('\n');
                }
                combined.push_str("[stderr] ");
                combined.push_str(&String::from_utf8_lossy(&out.stderr));
            }
            if combined.is_empty() {
                combined.push_str("(no output)");
            }
            truncate_output(&combined, 4000)
        }
        Err(e) => format!("Error executing command: {e}"),
    }
}

/// Read a file and return its contents (truncated to 4000 chars).
fn read_file(path: &str) -> String {
    match std::fs::read_to_string(path) {
        Ok(content) => truncate_output(&content, 4000),
        Err(e) => format!("Error reading file '{path}': {e}"),
    }
}

/// List files in a directory.
fn list_files(path: &str) -> String {
    match std::fs::read_dir(path) {
        Ok(entries) => {
            let mut items: Vec<String> = Vec::new();
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                let suffix = if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    "/"
                } else {
                    ""
                };
                items.push(format!("{name}{suffix}"));
            }
            items.sort();
            if items.is_empty() {
                "(empty directory)".to_string()
            } else {
                items.join("\n")
            }
        }
        Err(e) => format!("Error listing directory '{path}': {e}"),
    }
}

/// Truncate output to `max` chars with an indicator.
fn truncate_output(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let end = s.ceil_char_boundary(max);
        format!("{}\n... (truncated, {} total chars)", &s[..end], s.len())
    }
}

/// Parse tool calls from a JSON string (as returned in the LLM response).
pub fn parse_tool_calls(json: &str) -> Vec<ToolCall> {
    serde_json::from_str(json).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_tools_count() {
        let tools = builtin_tools();
        assert_eq!(tools.len(), 3);
        assert!(tools.iter().any(|t| t.function.name == "run_command"));
        assert!(tools.iter().any(|t| t.function.name == "read_file"));
        assert!(tools.iter().any(|t| t.function.name == "list_files"));
    }

    #[test]
    fn test_run_shell_command_echo() {
        let result = run_shell_command("echo hello");
        assert!(result.contains("hello"));
    }

    #[test]
    fn test_run_shell_command_empty() {
        let result = run_shell_command("");
        assert!(result.contains("empty command"));
    }

    #[test]
    fn test_run_shell_command_with_stderr() {
        let result = run_shell_command("ls /nonexistent_dir_xyz");
        assert!(result.contains("[stderr]") || result.contains("No such file"));
    }

    #[test]
    fn test_read_file_existing() {
        // Read Cargo.toml from crate root
        let result = read_file("Cargo.toml");
        assert!(result.contains("[package]") || result.contains("[lib]"));
    }

    #[test]
    fn test_read_file_nonexistent() {
        let result = read_file("/tmp/nonexistent_file_xyz.txt");
        assert!(result.contains("Error reading file"));
    }

    #[test]
    fn test_list_files_current_dir() {
        let result = list_files(".");
        assert!(!result.is_empty());
        assert!(result.contains("src/") || result.contains("Cargo.toml"));
    }

    #[test]
    fn test_list_files_nonexistent() {
        let result = list_files("/tmp/nonexistent_dir_xyz");
        assert!(result.contains("Error listing directory"));
    }

    #[test]
    fn test_truncate_output_short() {
        assert_eq!(truncate_output("hello", 100), "hello");
    }

    #[test]
    fn test_truncate_output_long() {
        let long = "a".repeat(5000);
        let result = truncate_output(&long, 4000);
        assert!(result.contains("(truncated"));
        assert!(result.len() < 5000);
    }

    #[test]
    fn test_execute_tool_run_command() {
        let call = ToolCall {
            id: "test-1".to_string(),
            call_type: "function".to_string(),
            function: ToolCallFunction {
                name: "run_command".to_string(),
                arguments: r#"{"command":"echo test123"}"#.to_string(),
            },
        };
        let result = execute_tool(&call);
        assert_eq!(result.tool_call_id, "test-1");
        assert!(result.content.contains("test123"));
    }

    #[test]
    fn test_execute_tool_unknown() {
        let call = ToolCall {
            id: "test-2".to_string(),
            call_type: "function".to_string(),
            function: ToolCallFunction {
                name: "nonexistent_tool".to_string(),
                arguments: "{}".to_string(),
            },
        };
        let result = execute_tool(&call);
        assert!(result.content.contains("Unknown tool"));
    }

    #[test]
    fn test_execute_tool_bad_json() {
        let call = ToolCall {
            id: "test-3".to_string(),
            call_type: "function".to_string(),
            function: ToolCallFunction {
                name: "run_command".to_string(),
                arguments: "not valid json".to_string(),
            },
        };
        // Should not panic — returns empty command error
        let result = execute_tool(&call);
        assert!(result.content.contains("empty command"));
    }
}
