//! Tool-use loop for the model to interact with the filesystem and shell.
//!
//! When tool mode is active, the model can call tools like `read`, `write`,
//! `edit`, `bash`, `glob`, and `grep`. Tool calls are parsed from the model's
//! response, executed, and the results are fed back as a follow-up message.
//! The loop continues until the model produces a final text response with no
//! tool calls.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::Result;
use serde::Serialize;
use wait_timeout::ChildExt;

use crate::sandbox::Sandbox;

/// Maximum number of tool call iterations per prompt.
const MAX_TOOL_ITERATIONS: usize = 25;

/// Timeout for shell commands executed by the tool-use loop (seconds).
const SHELL_TIMEOUT_SECS: u64 = 60;

/// A tool definition sent to the model.
#[derive(Debug, Serialize)]
pub struct ToolDefinition {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: serde_json::Value,
}

/// A tool call parsed from the model's response.
#[derive(Debug)]
pub struct ToolCall {
    pub name: String,
    pub arguments: serde_json::Value,
}

/// The result of executing a tool.
#[derive(Debug)]
pub struct ToolResult {
    pub tool_name: String,
    pub output: String,
    pub is_error: bool,
}

/// Get the tool definitions for the model.
pub fn tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "read",
            description: "Read the contents of a file. Use this when you need to examine the contents of an existing file.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "The path of the file to read (relative to project root or absolute)"
                    }
                },
                "required": ["path"]
            }),
        },
        ToolDefinition {
            name: "write",
            description: "Write content to a file. Creates the file if it doesn't exist, overwrites if it does. Use this when you need to create a new file or completely replace an existing one.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "The path of the file to write (relative to project root or absolute)"
                    },
                    "content": {
                        "type": "string",
                        "description": "The full content to write to the file"
                    }
                },
                "required": ["path", "content"]
            }),
        },
        ToolDefinition {
            name: "edit",
            description: "Edit a file by finding and replacing text. Use this for targeted changes to existing files without rewriting the entire file.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "The path of the file to edit (relative to project root or absolute)"
                    },
                    "old_string": {
                        "type": "string",
                        "description": "The exact text to find and replace (must match the file exactly)"
                    },
                    "new_string": {
                        "type": "string",
                        "description": "The new text to replace it with"
                    }
                },
                "required": ["path", "old_string", "new_string"]
            }),
        },
        ToolDefinition {
            name: "bash",
            description: "Run a shell command. Use this to execute commands like git, cargo, npm, ls, etc. The command runs in the project root directory.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The shell command to run"
                    },
                    "description": {
                        "type": "string",
                        "description": "A brief description of what this command does"
                    }
                },
                "required": ["command", "description"]
            }),
        },
        ToolDefinition {
            name: "glob",
            description: "List files matching a glob pattern. Use this to discover files in the project.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "The glob pattern to match (e.g. '**/*.rs', 'src/**/*.ts')"
                    }
                },
                "required": ["pattern"]
            }),
        },
        ToolDefinition {
            name: "grep",
            description: "Search for text in files using a regex pattern. Use this to find where functions are defined, where strings appear, etc.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "The regex pattern to search for"
                    },
                    "path": {
                        "type": "string",
                        "description": "Optional: path or glob to limit the search (e.g. 'src/**/*.rs')"
                    }
                },
                "required": ["pattern"]
            }),
        },
    ]
}

/// Build the tool-use system instruction that tells the model how to call tools.
pub fn tool_use_system_instruction() -> String {
    let defs = tool_definitions();
    let mut instruction = String::from(
        "You have access to tools that let you read, write, and edit files, run shell commands, \
         and search the project. When you need to perform an action, call the appropriate tool.\n\n\
         Available tools:\n",
    );
    for tool in &defs {
        instruction.push_str(&format!(
            "- **{}**: {}  \n  Input: {}\n",
            tool.name,
            tool.description,
            serde_json::to_string_pretty(&tool.input_schema).unwrap_or_default()
        ));
    }
    instruction.push_str(
        "\nTo call a tool, output a JSON block on its own line like:\n\
         <tool>{\"name\":\"read\",\"arguments\":{\"path\":\"src/main.rs\"}}</tool>\n\n\
         After receiving the tool result, you can call another tool or provide your final response. \
         When you are done, just respond normally without a <tool> block.",
    );
    instruction
}

/// Parse tool calls from the model's response text.
pub fn parse_tool_calls(text: &str) -> Vec<ToolCall> {
    let mut calls = Vec::new();
    let mut remaining = text;

    while let Some(start) = remaining.find("<tool>") {
        let after_start = &remaining[start + 6..];
        if let Some(end) = after_start.find("</tool>") {
            let json_str = &after_start[..end];
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(json_str) {
                let name = value
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let arguments = value
                    .get("arguments")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                if !name.is_empty() {
                    calls.push(ToolCall { name, arguments });
                }
            }
            remaining = &after_start[end + 7..];
        } else {
            break;
        }
    }

    calls
}

/// Execute a tool call and return the result.
pub fn execute_tool(call: &ToolCall, sandbox: &Sandbox, project_root: &Path) -> ToolResult {
    match call.name.as_str() {
        "read" => execute_read(call, project_root),
        "write" => execute_write(call, sandbox, project_root),
        "edit" => execute_edit(call, sandbox, project_root),
        "bash" => execute_bash(call, sandbox, project_root),
        "glob" => execute_glob(call, project_root),
        "grep" => execute_grep(call, project_root),
        _ => ToolResult {
            tool_name: call.name.clone(),
            output: format!("unknown tool: {}", call.name),
            is_error: true,
        },
    }
}

fn execute_read(call: &ToolCall, project_root: &Path) -> ToolResult {
    let path = call
        .arguments
        .get("path")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let target = resolve_path(path, project_root);

    match std::fs::read_to_string(&target) {
        Ok(content) => {
            let line_count = content.lines().count();
            let size = content.len();
            ToolResult {
                tool_name: "read".to_string(),
                output: format!(
                    "File: {}\nLines: {}\nSize: {} bytes\n\n{}",
                    target.display(),
                    line_count,
                    size,
                    content
                ),
                is_error: false,
            }
        }
        Err(e) => ToolResult {
            tool_name: "read".to_string(),
            output: format!("Error reading {}: {}", target.display(), e),
            is_error: true,
        },
    }
}

fn execute_write(call: &ToolCall, sandbox: &Sandbox, project_root: &Path) -> ToolResult {
    let path = call
        .arguments
        .get("path")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let content = call
        .arguments
        .get("content")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let target = resolve_path(path, project_root);

    // Check sandbox
    let verdict = sandbox.evaluate(crate::permissions::Operation::WriteFile, Some(&target));
    if !matches!(
        verdict.decision,
        crate::permissions::PermissionDecision::Allow
    ) {
        return ToolResult {
            tool_name: "write".to_string(),
            output: format!(
                "Blocked by sandbox: {} ({})",
                target.display(),
                verdict.reason
            ),
            is_error: true,
        };
    }

    if let Some(parent) = target.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return ToolResult {
                tool_name: "write".to_string(),
                output: format!("Error creating directory {}: {}", parent.display(), e),
                is_error: true,
            };
        }
    }

    match std::fs::write(&target, content) {
        Ok(()) => {
            let size = content.len();
            ToolResult {
                tool_name: "write".to_string(),
                output: format!("Written {} bytes to {}", size, target.display()),
                is_error: false,
            }
        }
        Err(e) => ToolResult {
            tool_name: "write".to_string(),
            output: format!("Error writing {}: {}", target.display(), e),
            is_error: true,
        },
    }
}

fn execute_edit(call: &ToolCall, sandbox: &Sandbox, project_root: &Path) -> ToolResult {
    let path = call
        .arguments
        .get("path")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let old_string = call
        .arguments
        .get("old_string")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let new_string = call
        .arguments
        .get("new_string")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let target = resolve_path(path, project_root);

    // Check sandbox
    let verdict = sandbox.evaluate(crate::permissions::Operation::WriteFile, Some(&target));
    if !matches!(
        verdict.decision,
        crate::permissions::PermissionDecision::Allow
    ) {
        return ToolResult {
            tool_name: "edit".to_string(),
            output: format!(
                "Blocked by sandbox: {} ({})",
                target.display(),
                verdict.reason
            ),
            is_error: true,
        };
    }

    let content = match std::fs::read_to_string(&target) {
        Ok(c) => c,
        Err(e) => {
            return ToolResult {
                tool_name: "edit".to_string(),
                output: format!("Error reading {}: {}", target.display(), e),
                is_error: true,
            };
        }
    };

    if !content.contains(old_string) {
        return ToolResult {
            tool_name: "edit".to_string(),
            output: format!(
                "Error: could not find the exact text to replace in {}. The old_string must match exactly.",
                target.display()
            ),
            is_error: true,
        };
    }

    let new_content = content.replace(old_string, new_string);
    match std::fs::write(&target, &new_content) {
        Ok(()) => {
            let changes = content.len() as isize - new_content.len() as isize;
            let change_desc = if changes >= 0 {
                format!("removed {} bytes", changes)
            } else {
                format!("added {} bytes", -changes)
            };
            ToolResult {
                tool_name: "edit".to_string(),
                output: format!("Edited {} ({})", target.display(), change_desc),
                is_error: false,
            }
        }
        Err(e) => ToolResult {
            tool_name: "edit".to_string(),
            output: format!("Error writing {}: {}", target.display(), e),
            is_error: true,
        },
    }
}

fn execute_bash(call: &ToolCall, sandbox: &Sandbox, project_root: &Path) -> ToolResult {
    let command_str = call
        .arguments
        .get("command")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");

    if command_str.trim().is_empty() {
        return ToolResult {
            tool_name: "bash".to_string(),
            output: "Error: empty command".to_string(),
            is_error: true,
        };
    }

    // Check sandbox for shell operations
    let verdict = sandbox.evaluate(crate::permissions::Operation::Shell, None);
    if !matches!(
        verdict.decision,
        crate::permissions::PermissionDecision::Allow
    ) {
        return ToolResult {
            tool_name: "bash".to_string(),
            output: format!("Blocked by sandbox: {}", verdict.reason),
            is_error: true,
        };
    }

    // Spawn the command as a child so we can enforce a timeout and kill it if
    // it runs too long. This prevents a hanging command from freezing the CLI.
    let mut child = match Command::new("sh")
        .arg("-c")
        .arg(command_str)
        .current_dir(project_root)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            return ToolResult {
                tool_name: "bash".to_string(),
                output: format!("Error running command: {}", e),
                is_error: true,
            }
        }
    };

    let timeout = Duration::from_secs(SHELL_TIMEOUT_SECS);
    let wait_result = child.wait_timeout(timeout);
    match wait_result {
        Ok(Some(status)) => {
            let stdout = child.stdout.take();
            let stderr = child.stderr.take();
            let stdout_str = stdout
                .map(|mut s| {
                    let mut buf = String::new();
                    let _ = s.read_to_string(&mut buf);
                    buf
                })
                .unwrap_or_default();
            let stderr_str = stderr
                .map(|mut s| {
                    let mut buf = String::new();
                    let _ = s.read_to_string(&mut buf);
                    buf
                })
                .unwrap_or_default();
            if status.success() {
                let mut result = String::new();
                if !stdout_str.trim().is_empty() {
                    result.push_str(&stdout_str);
                }
                if !stderr_str.trim().is_empty() {
                    if !result.is_empty() {
                        result.push('\n');
                    }
                    result.push_str(&format!("(stderr) {}", stderr_str.trim()));
                }
                if result.trim().is_empty() {
                    result = "Command completed successfully (no output)".to_string();
                }
                ToolResult {
                    tool_name: "bash".to_string(),
                    output: result.trim().to_string(),
                    is_error: false,
                }
            } else {
                let mut result = String::new();
                if !stdout_str.trim().is_empty() {
                    result.push_str(&stdout_str);
                }
                if !stderr_str.trim().is_empty() {
                    if !result.is_empty() {
                        result.push('\n');
                    }
                    result.push_str(&stderr_str);
                }
                ToolResult {
                    tool_name: "bash".to_string(),
                    output: format!(
                        "Command exited with code {}:\n{}",
                        status.code().unwrap_or(-1),
                        result.trim()
                    ),
                    is_error: true,
                }
            }
        }
        Ok(None) => {
            // Timed out: kill the child and report the timeout.
            let _ = child.kill();
            let _ = child.wait();
            ToolResult {
                tool_name: "bash".to_string(),
                output: format!(
                    "Command timed out after {}s and was killed",
                    SHELL_TIMEOUT_SECS
                ),
                is_error: true,
            }
        }
        Err(e) => ToolResult {
            tool_name: "bash".to_string(),
            output: format!("Error running command: {}", e),
            is_error: true,
        },
    }
}

fn execute_glob(call: &ToolCall, project_root: &Path) -> ToolResult {
    let pattern = call
        .arguments
        .get("pattern")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");

    let full_pattern = project_root.join(pattern);
    let pattern_str = full_pattern.to_string_lossy().to_string();

    match glob::glob(&pattern_str) {
        Ok(entries) => {
            let paths: Vec<String> = entries
                .filter_map(|entry| entry.ok())
                .filter(|p| {
                    // Exclude secret-named files from glob results.
                    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    !secret_excludes().contains(&name)
                })
                .map(|p| {
                    p.strip_prefix(project_root)
                        .unwrap_or(&p)
                        .to_string_lossy()
                        .to_string()
                })
                .collect();

            if paths.is_empty() {
                ToolResult {
                    tool_name: "glob".to_string(),
                    output: format!("No files matching pattern: {}", pattern),
                    is_error: false,
                }
            } else {
                ToolResult {
                    tool_name: "glob".to_string(),
                    output: format!(
                        "Found {} file(s) matching '{}':\n{}",
                        paths.len(),
                        pattern,
                        paths.join("\n")
                    ),
                    is_error: false,
                }
            }
        }
        Err(e) => ToolResult {
            tool_name: "glob".to_string(),
            output: format!("Error globbing '{}': {}", pattern, e),
            is_error: true,
        },
    }
}

fn execute_grep(call: &ToolCall, project_root: &Path) -> ToolResult {
    let pattern = call
        .arguments
        .get("pattern")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let path_filter = call
        .arguments
        .get("path")
        .and_then(serde_json::Value::as_str);

    let mut cmd = Command::new("grep");
    cmd.arg("-rn")
        .arg("--with-filename")
        .arg("-E")
        .arg(pattern)
        .current_dir(project_root);

    // Exclude secret-named files so the model cannot read credentials through
    // grep results.
    for exclude in secret_excludes() {
        cmd.arg("--exclude").arg(exclude);
    }
    // Bound the search to avoid scanning enormous trees.
    cmd.arg("--exclude-dir").arg(".git");
    cmd.arg("--exclude-dir").arg("node_modules");
    cmd.arg("--exclude-dir").arg("target");

    if let Some(filter) = path_filter {
        cmd.arg("--include").arg(filter);
    }

    cmd.arg(".");

    let output = cmd.output();
    match output {
        Ok(output) => {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let lines: Vec<&str> = stdout.lines().collect();
                let total = lines.len();
                let display: Vec<&str> = lines.iter().take(50).copied().collect();
                let mut result = format!("Found {} match(es) for '{}':\n", total, pattern);
                result.push_str(&display.join("\n"));
                if total > 50 {
                    result.push_str(&format!("\n... and {} more matches", total - 50));
                }
                ToolResult {
                    tool_name: "grep".to_string(),
                    output: result,
                    is_error: false,
                }
            } else {
                ToolResult {
                    tool_name: "grep".to_string(),
                    output: format!("No matches found for '{}'", pattern),
                    is_error: false,
                }
            }
        }
        Err(e) => ToolResult {
            tool_name: "grep".to_string(),
            output: format!("Error searching: {}", e),
            is_error: true,
        },
    }
}

/// File names that are excluded from grep/glob results to avoid leaking secrets.
fn secret_excludes() -> &'static [&'static str] {
    crate::blocklist::secret_file_names()
}

/// Resolve a path relative to the project root.
fn resolve_path(path: &str, project_root: &Path) -> PathBuf {
    let p = PathBuf::from(path);
    if p.is_absolute() {
        p
    } else {
        project_root.join(&p)
    }
}

/// Run the tool-use loop: send the prompt, process tool calls, return the final
/// response text.
pub async fn run_tool_loop(
    prompt: &str,
    sandbox: &Sandbox,
    project_root: &Path,
    endpoint: &crate::config::EndpointConfig,
    model: &str,
    history: Vec<crate::providers::ChatMessage>,
    skill_prompt: Option<String>,
) -> Result<String> {
    let adapter = crate::providers::adapter_for(endpoint.provider.clone());

    let mut messages = vec![crate::providers::ChatMessage {
        role: "system".to_string(),
        content: tool_use_system_instruction(),
    }];
    // Inject the active skill's prompt as a system message if set.
    if let Some(skill) = skill_prompt {
        messages.push(crate::providers::ChatMessage {
            role: "system".to_string(),
            content: skill,
        });
    }
    // Inject prior session turns for multi-turn context.
    messages.extend(history);
    messages.push(crate::providers::ChatMessage {
        role: "user".to_string(),
        content: prompt.to_string(),
    });

    for iteration in 0..MAX_TOOL_ITERATIONS {
        let request = crate::providers::ChatRequest {
            model: model.to_string(),
            messages: messages.clone(),
            max_tokens: Some(4096),
        };

        let mut response = String::new();
        let mut first_token = true;
        let thinking = crate::ui::thinking_start();

        adapter
            .stream_chat(endpoint, request, &mut |delta| {
                if first_token {
                    first_token = false;
                    crate::ui::thinking_stop(thinking.clone());
                }
                response.push_str(&delta);
            })
            .await?;

        if first_token {
            crate::ui::thinking_stop(thinking);
        }

        let tool_calls = parse_tool_calls(&response);
        if tool_calls.is_empty() {
            // No more tool calls; this is the final response
            return Ok(response);
        }

        // Add the assistant's response (with tool calls) to the conversation
        messages.push(crate::providers::ChatMessage {
            role: "assistant".to_string(),
            content: response.clone(),
        });

        // Execute each tool call and add results
        for call in &tool_calls {
            let result = execute_tool(call, sandbox, project_root);
            let result_text = if result.is_error {
                format!("Error: {}", result.output)
            } else {
                result.output
            };
            messages.push(crate::providers::ChatMessage {
                role: "user".to_string(),
                content: format!("Tool result for '{}':\n{}", call.name, result_text),
            });
        }

        if iteration == MAX_TOOL_ITERATIONS - 1 {
            messages.push(crate::providers::ChatMessage {
                role: "user".to_string(),
                content: "You have reached the maximum number of tool calls. Please provide your final response now.".to_string(),
            });
        }
    }

    // Final call to get the response
    let request = crate::providers::ChatRequest {
        model: model.to_string(),
        messages,
        max_tokens: Some(4096),
    };

    let mut response = String::new();
    let mut first_token = true;
    let thinking = crate::ui::thinking_start();

    adapter
        .stream_chat(endpoint, request, &mut |delta| {
            if first_token {
                first_token = false;
                crate::ui::thinking_stop(thinking.clone());
            }
            response.push_str(&delta);
        })
        .await?;

    if first_token {
        crate::ui::thinking_stop(thinking);
    }

    Ok(response)
}
