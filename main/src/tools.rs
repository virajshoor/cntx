//! Tool-use loop for the model to interact with the filesystem and shell.
//!
//! When tool mode is active, the model can call tools like `read`, `write`,
//! `edit`, `bash`, `glob`, and `grep`. Tool calls are parsed from the model's
//! response, executed through the shared validation/permission boundary in
//! the C core, and the results are fed back as follow-up messages. The loop
//! continues until the model produces a final text response with no tool
//! calls.
//!
//! Every tool passes the same order:
//! validate → containment/exclusions → permission policy → dry-run block →
//! approval → execute once → bounded result → transcript persistence.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, Result};
use owo_colors::OwoColorize;
use serde::Serialize;

use crate::sandbox::Sandbox;

/// A tool definition sent to the model.
#[derive(Debug, Serialize)]
pub struct ToolDefinition {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: serde_json::Value,
}

/// A tool call parsed from the model's response.
#[derive(Clone, Debug)]
pub struct ToolCall {
    pub name: String,
    pub arguments: serde_json::Value,
}

/// Session-scoped prompt state supplied to the tool-use loop.
pub struct ToolLoopPromptContext {
    pub history: Vec<crate::providers::ChatMessage>,
    pub skill_prompt: Option<String>,
    pub effort: crate::config::Effort,
    /// Goal objective/status injected into each request while a goal runs.
    pub goal: Option<GoalPromptState>,
    /// Stable conversation id for the OpenCode Go session header.
    pub session_id: Option<String>,
    /// Optional project memory / AGENTS.md for the stable cacheable prefix.
    pub cacheable_project: Option<String>,
    /// Extra MCP tool specs selected for this session (not builtins).
    pub mcp_tools: Vec<crate::providers::ToolSpec>,
    /// Directory for full tool-result artifacts (packed stubs point here).
    pub artifacts_dir: Option<std::path::PathBuf>,
    /// Optional MCP servers to call by prefixed name `mcp__<server>__<tool>`.
    pub mcp_servers: Vec<crate::config::McpServerConfig>,
}

impl Default for ToolLoopPromptContext {
    fn default() -> Self {
        Self {
            history: Vec::new(),
            skill_prompt: None,
            effort: crate::config::Effort::Medium,
            goal: None,
            session_id: None,
            cacheable_project: None,
            mcp_tools: Vec::new(),
            artifacts_dir: None,
            mcp_servers: Vec::new(),
        }
    }
}

/// Goal information injected into tool-loop requests.
#[derive(Clone)]
pub struct GoalPromptState {
    pub objective: String,
    pub status: String,
    pub progress: String,
    pub steps_used: u32,
    pub max_steps: u32,
}

/// The result of executing a tool.
#[derive(Debug)]
pub struct ToolResult {
    pub tool_name: String,
    pub output: String,
    pub is_error: bool,
}

/// A validated `goal_update` action from the model.
#[derive(Debug, Clone)]
pub struct GoalUpdate {
    pub status: String,
    pub progress: String,
    pub evidence: String,
}

/// Host callbacks the tool loop needs. The runtime implements this so the
/// loop itself stays free of session/persistence details.
#[async_trait::async_trait]
pub trait ToolHost: Send {
    async fn prepare_request(
        &mut self,
        _endpoint: &crate::config::EndpointConfig,
        _request: &mut crate::providers::ChatRequest,
    ) -> Result<()> {
        Ok(())
    }
    /// True when mutations and commands must be blocked (dry-run).
    fn dry_run(&self) -> bool;
    /// Ask the human; execute only after explicit yes. `Always` also covers
    /// later actions of the same kind for the session.
    fn approve(&mut self, action: &str) -> crate::permissions::ApprovalChoice;
    /// Handle a validated goal_update action while a goal runs.
    fn goal_update(&mut self, _arguments: &serde_json::Value) -> Result<String, String> {
        Err("no active goal; goal_update is only valid during a goal run".to_string())
    }
    /// Persist a transcript entry (assistant tool request or tool result).
    fn record_transcript(&mut self, _role: &str, _content: &str) -> Result<()> {
        Ok(())
    }
    /// Checkpoint and count each provider turn before starting it.
    fn before_request(&mut self) -> Result<bool> {
        Ok(true)
    }
    fn stopped(&self) -> bool {
        false
    }
    /// Record provider token usage from the last stream.
    fn record_usage(&mut self, _usage: &crate::providers::TokenUsage) {}
}

/// Get the tool definitions for the model.
pub fn tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "read",
            description: "Read up to 24KB of a file. Use optional byte offset to continue reading.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "offset": {"type": "integer", "minimum": 0, "description": "Optional byte offset for large files"},
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
            description: "Edit a file by finding and replacing text. The old_string must match exactly once. Use this for targeted changes to existing files.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "The path of the file to edit (relative to project root or absolute)"
                    },
                    "old_string": {
                        "type": "string",
                        "description": "The exact text to find and replace (must match the file exactly once)"
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
            description: "Run a shell command. Use this to execute commands like git, cargo, npm, ls, etc. The command runs in the project root directory with a 60s default timeout.",
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

/// Build the slim tool-use system instruction (schemas go in request.tools).
pub fn tool_use_system_instruction(
    effort: crate::config::Effort,
    goal: Option<&GoalPromptState>,
) -> String {
    let mut instruction = format!(
        "You are Cntx Code, a coding assistant running locally in the user's terminal. \
You have direct access to the user's filesystem and can read, write, and edit files, run shell commands, \
and search the project. You are NOT a browser-based chat assistant — you run on the user's machine. \
When the user asks you to write a script, write the file to disk and run it. \
When the user asks you to edit files, use the edit tool. \
Respond in the same language the user writes in (English, Chinese, Spanish, etc.). \
Be concise and direct. Write correct, working code. Use markdown for formatting. \
Do not add unnecessary preamble or postamble.\n\n\
Effort level: {}. {}\n\n\
Use tools via the provider tool API. Fallback: <tool>{{\"name\":\"...\",\"arguments\":{{...}}}}</tool>.\n\
Prefer glob, grep, and outline-style discovery before full file reads.\n\
Available tools: read, write, edit, bash, glob, grep.\n",
        effort.as_str(),
        effort.instruction(),
    );
    if let Some(goal) = goal {
        instruction.push_str(&format!(
            "\nYou are working toward a persistent goal.\n\
Objective: {}\n\
Goal status: {} (steps used {}/{})\n\
Progress so far: {}\n\n\
Report goal state with the `goal_update` tool:\n\
<tool>{{\"name\":\"goal_update\",\"arguments\":{{\"status\":\"active|blocked|completed\",\"progress\":\"...\",\"evidence\":\"...\"}}}}</tool>\n\
- `completed` requires nonempty evidence referring to actual tool results or checks from this session.\n\
- If you cannot proceed, use status `blocked` and explain what you need.\n\
- A plain prose response does not complete the goal; only goal_update can.\n",
            goal.objective,
            goal.status,
            goal.steps_used,
            goal.max_steps,
            if goal.progress.is_empty() {
                "(none yet)"
            } else {
                &goal.progress
            },
        ));
    }
    instruction
}

/// Convert built-in tool definitions into provider ToolSpec values.
pub fn builtin_tool_specs() -> Vec<crate::providers::ToolSpec> {
    tool_definitions()
        .into_iter()
        .map(|tool| crate::providers::ToolSpec {
            name: tool.name.to_string(),
            description: tool.description.to_string(),
            input_schema: tool.input_schema,
        })
        .collect()
}

/// Parse tool calls from the model's response text.
/// Handles both the standard format `{"name":"read","arguments":{"path":"..."}}`
/// and the common variant where args are at the top level:
/// `{"name":"bash","command":"ls -la"}`.
///
/// A malformed block is an error, not a silent skip: an unclosed `<tool>`
/// tag or invalid JSON returns a descriptive error so the loop can feed a
/// correction back to the model.
pub fn parse_tool_calls(text: &str) -> Result<Vec<ToolCall>> {
    let mut calls = Vec::new();
    let mut remaining = text;

    while let Some(start) = remaining.find("<tool>") {
        let after_start = &remaining[start + 6..];
        let Some(end) = after_start.find("</tool>") else {
            return Err(anyhow!(
                "a <tool> block has no closing </tool> tag; repeat the call as one complete <tool>{{...}}</tool> block"
            ));
        };
        let json_str = &after_start[..end];
        let Ok(value) = serde_json::from_str::<serde_json::Value>(json_str) else {
            return Err(anyhow!(
                "a <tool> block contained invalid JSON; repeat the call as one complete <tool>{{...}}</tool> block with valid JSON"
            ));
        };
        let name = value
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string();
        if name.is_empty() {
            return Err(anyhow!(
                "a <tool> block is missing its \"name\" key; include both \"name\" and \"arguments\""
            ));
        }
        // Standard: arguments nested under "arguments" key.
        // Fallback: treat all top-level keys except "name" as arguments
        // (handles models that flatten the structure).
        let arguments = if let Some(args) = value.get("arguments") {
            args.clone()
        } else {
            let mut args = serde_json::Map::new();
            if let Some(obj) = value.as_object() {
                for (key, val) in obj {
                    if key != "name" {
                        args.insert(key.clone(), val.clone());
                    }
                }
            }
            serde_json::Value::Object(args)
        };
        calls.push(ToolCall { name, arguments });
        remaining = &after_start[end + 7..];
    }

    Ok(calls)
}

/// All tools pass through the same validation and permission boundary.
pub fn execute_tool(
    call: &ToolCall,
    sandbox: &Sandbox,
    project_root: &Path,
    dry_run: bool,
    approve: &mut dyn FnMut(&str) -> crate::permissions::ApprovalChoice,
) -> ToolResult {
    if let Some(result) = authorize_tool(call, sandbox, project_root, dry_run, approve) {
        return result;
    }
    execute_authorized(call, sandbox, project_root)
}

fn authorize_tool(
    call: &ToolCall,
    sandbox: &Sandbox,
    project_root: &Path,
    dry_run: bool,
    approve: &mut dyn FnMut(&str) -> crate::permissions::ApprovalChoice,
) -> Option<ToolResult> {
    use crate::permissions::{Operation, PermissionDecision};
    // 1. Validate name and arguments (C core).
    if let Err(message) = crate::core::tool_validate(&call.name, &call.arguments) {
        return Some(tool_error(call, message));
    }

    // 2. Resolve paths and containment/exclusions.
    let target = call
        .arguments
        .get("path")
        .and_then(serde_json::Value::as_str)
        .map(|path| resolve_path(path, project_root));
    let verdict = sandbox.evaluate(operation_for(&call.name), target.as_deref());

    // 3. Permission policy (C core): containment denial always wins first.
    if verdict.decision == PermissionDecision::Deny {
        return Some(tool_error(call, format!("Blocked: {}", verdict.reason)));
    }

    // 4. Dry-run blocks mutations and shell execution.
    if dry_run
        && matches!(
            operation_for(&call.name),
            Operation::WriteFile | Operation::Shell
        )
    {
        return Some(tool_error(
            call,
            "Dry run: action was NOT executed. Describe the proposed changes instead.",
        ));
    }

    // 5. Approval when the policy asks.
    if verdict.decision == PermissionDecision::Ask && !approve(&describe_action(call)).allowed() {
        return Some(tool_error(
            call,
            "Skipped: you did not approve this step. The assistant must not retry it or work around the decision.",
        ));
    }

    None
}

/// Plain-language summary of a tool call for approval prompts. File contents
/// are never included: the path (or command) is enough to decide, and keeps
/// terminal scrollback free of duplicated content.
fn describe_action(call: &ToolCall) -> String {
    let arg = |key: &str| {
        call.arguments
            .get(key)
            .and_then(|v| v.as_str())
            .unwrap_or("")
    };
    match call.name.as_str() {
        "read" => format!("read file \"{}\"", arg("path")),
        "write" => format!("write file \"{}\"", arg("path")),
        "edit" => format!("edit file \"{}\"", arg("path")),
        "bash" => {
            let short: String = arg("command").chars().take(120).collect();
            format!("run command \"{short}\"")
        }
        "glob" => format!("list files matching \"{}\"", arg("pattern")),
        "grep" => format!("search for \"{}\"", arg("pattern")),
        _ => format!("use tool \"{}\"", call.name),
    }
}

fn execute_authorized(call: &ToolCall, sandbox: &Sandbox, project_root: &Path) -> ToolResult {
    // Approval can take arbitrary time: recheck containment immediately before I/O.
    if matches!(call.name.as_str(), "write" | "edit") {
        let target = resolve_path(
            call.arguments["path"].as_str().unwrap_or_default(),
            project_root,
        );
        let verdict = sandbox.evaluate(crate::permissions::Operation::WriteFile, Some(&target));
        if verdict.decision == crate::permissions::PermissionDecision::Deny {
            return tool_error(call, format!("Blocked: {}", verdict.reason));
        }
    }
    // 6-7. Execute once and capture a bounded result.
    match call.name.as_str() {
        "read" => execute_read(call, project_root),
        "write" => execute_write(call, project_root),
        "edit" => execute_edit(call, project_root),
        "bash" => execute_bash(call, project_root),
        "glob" => execute_glob(call, project_root),
        "grep" => execute_grep(call, project_root),
        _ => tool_error(call, format!("unknown tool: {}", call.name)),
    }
}

fn operation_for(name: &str) -> crate::permissions::Operation {
    match name {
        "write" | "edit" => crate::permissions::Operation::WriteFile,
        "bash" => crate::permissions::Operation::Shell,
        _ => crate::permissions::Operation::ReadFile,
    }
}

fn tool_error(call: &ToolCall, message: impl Into<String>) -> ToolResult {
    ToolResult {
        tool_name: call.name.clone(),
        output: message.into(),
        is_error: true,
    }
}

fn execute_read(call: &ToolCall, project_root: &Path) -> ToolResult {
    let path = call
        .arguments
        .get("path")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let target = resolve_path(path, project_root);
    // Reads respect the same secret exclusions as grep/glob so the model
    // cannot pull credentials from a differently-named path either.
    if crate::blocklist::is_secret_file(&target) {
        return ToolResult {
            tool_name: "read".to_string(),
            output: format!(
                "Blocked: {} is a secret/credential file and cannot be read.",
                target.display()
            ),
            is_error: true,
        };
    }
    let offset = call
        .arguments
        .get("offset")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    match crate::core::file_read(&target, offset, crate::core::tool_read_limit()) {
        Ok((content, truncated)) => {
            let mut text = format!(
                "File: {}\nLines: {}\nSize: {} bytes\n\n{}",
                target.display(),
                content.lines().count(),
                content.len(),
                content
            );
            if truncated {
                text.push_str(
                    "\n[File continues past this window; pass a byte offset to read more.]",
                );
            }
            ToolResult {
                tool_name: "read".to_string(),
                output: text,
                is_error: false,
            }
        }
        Err(message) => ToolResult {
            tool_name: "read".to_string(),
            output: format!("Error reading {}: {}", target.display(), message),
            is_error: true,
        },
    }
}

fn execute_write(call: &ToolCall, project_root: &Path) -> ToolResult {
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
    // Revalidate the write target at execution time.
    let target = resolve_path(path, project_root);
    match crate::core::file_write(&target, content) {
        Ok(()) => ToolResult {
            tool_name: "write".to_string(),
            output: format!("Written {} bytes to {}", content.len(), target.display()),
            is_error: false,
        },
        Err(message) => ToolResult {
            tool_name: "write".to_string(),
            output: format!("Error writing {}: {}", target.display(), message),
            is_error: true,
        },
    }
}

fn execute_edit(call: &ToolCall, project_root: &Path) -> ToolResult {
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
    match crate::core::file_edit(&target, old_string, new_string) {
        Ok(()) => {
            let changes = old_string.len() as isize - new_string.len() as isize;
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
        Err(message) => ToolResult {
            tool_name: "edit".to_string(),
            output: format!("Error editing {}: {}", target.display(), message),
            is_error: true,
        },
    }
}

fn execute_bash(call: &ToolCall, project_root: &Path) -> ToolResult {
    let command = call.arguments["command"].as_str().unwrap_or_default();
    // Ctrl+C cancels the running child through the host-owned flag.
    let cancel = &crate::interactive::CANCEL;
    // Optional positive timeout_secs bounded by the C-core maximum.
    let result = crate::core::command_run(
        command,
        project_root,
        tool_timeout_from_args(&call.arguments),
        cancel,
    );
    match result {
        Ok(outcome) => {
            let (label, success) = if outcome.timed_out {
                ("Command timed out and was terminated".to_string(), false)
            } else if outcome.exit_code < 0 {
                ("Command interrupted; process terminated".to_string(), false)
            } else {
                (
                    format!("Exit code: {}", outcome.exit_code),
                    outcome.exit_code == 0,
                )
            };
            let mut output = String::new();
            if !outcome.stdout.trim().is_empty() {
                output.push_str(&outcome.stdout);
            }
            if !outcome.stderr.trim().is_empty() {
                if !output.is_empty() {
                    output.push('\n');
                }
                output.push_str(&format!("(stderr) {}", outcome.stderr.trim()));
            }
            if output.trim().is_empty() {
                output = "Command completed (no output)".to_string();
            }
            ToolResult {
                tool_name: call.name.clone(),
                output: format!("{label}\n{output}"),
                is_error: !success,
            }
        }
        Err(message) => tool_error(call, format!("Command failed: {message}")),
    }
}

/// Optional positive `timeout_secs` from the tool arguments, bounded by the
/// C-core maximum. Zero, negative, or missing values keep the default.
fn tool_timeout_from_args(arguments: &serde_json::Value) -> Duration {
    let default = crate::core::tool_timeout();
    let Some(raw) = arguments
        .get("timeout_secs")
        .and_then(serde_json::Value::as_u64)
    else {
        return default;
    };
    if raw == 0 {
        return default;
    }
    Duration::from_secs(raw.min(crate::core::tool_timeout_max().as_secs()))
}

fn execute_glob(call: &ToolCall, project_root: &Path) -> ToolResult {
    let pattern = call
        .arguments
        .get("pattern")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");

    let full_pattern = project_root.join(pattern);
    let pattern_str = full_pattern.to_string_lossy().to_string();
    let limit = crate::core::glob_result_limit() as usize;

    match glob::glob(&pattern_str) {
        Ok(entries) => {
            // Take limit+1 entries so hitting limit+1 signals truncation.
            let mut paths: Vec<String> = entries
                .filter_map(|entry| entry.ok())
                .filter(|p| {
                    // Exclude secret-named files with the same case-insensitive
                    // rule as reads and grep.
                    !crate::blocklist::is_secret_file(p.as_path())
                })
                .map(|p| {
                    p.strip_prefix(project_root)
                        .unwrap_or(&p)
                        .to_string_lossy()
                        .to_string()
                })
                .take(limit + 1)
                .collect();
            let truncated = paths.len() > limit;
            paths.truncate(limit);

            if paths.is_empty() {
                ToolResult {
                    tool_name: "glob".to_string(),
                    output: format!("No files matching pattern: {}", pattern),
                    is_error: false,
                }
            } else {
                let notice = if truncated {
                    format!(
                        "\n[Result limited to {} paths; use a more specific pattern.]",
                        limit
                    )
                } else {
                    String::new()
                };
                ToolResult {
                    tool_name: "glob".to_string(),
                    output: format!(
                        "Found {} file(s) (up to {}) matching '{}':\n{}{}",
                        paths.len(),
                        limit,
                        pattern,
                        paths.join("\n"),
                        notice
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

fn secret_excludes() -> &'static [&'static str] {
    crate::blocklist::secret_file_names()
}

/// Bounded grep through the C core's command runner: output is capped so
/// the pipe cannot fill, and exit code 1 (no matches) is distinct from real
/// execution errors. Timeout and cancellation terminate the child.
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

    let shell_quote = |value: &str| format!("'{}'", value.replace('\'', "'\\''"));
    let mut cmdline = String::from("grep -rn --with-filename -E");
    for exclude in secret_excludes() {
        cmdline.push_str(&format!(" --exclude {}", shell_quote(exclude)));
    }
    cmdline.push_str(" --exclude-dir .git --exclude-dir node_modules --exclude-dir target");
    if let Some(filter) = path_filter {
        cmdline.push_str(&format!(" --include {}", shell_quote(filter)));
    }
    cmdline.push_str(&format!(" -e {} .", shell_quote(pattern)));

    let cancel = &crate::interactive::CANCEL;
    let result =
        crate::core::command_run(&cmdline, project_root, crate::core::tool_timeout(), cancel);
    match result {
        Ok(outcome) => match outcome.exit_code {
            0 => {
                // Keep only matches outside secret-named files: grep's
                // --exclude is case-sensitive, so enforce the shared
                // case-insensitive blocklist on the reported paths too.
                let lines: Vec<&str> = outcome
                    .stdout
                    .lines()
                    .filter(|line| {
                        let path = line.split(':').next().unwrap_or("");
                        !crate::blocklist::is_secret_file(std::path::Path::new(path))
                    })
                    .collect();
                let total = lines.len();
                let display_limit = crate::core::grep_line_limit() as usize;
                let mut result = format!(
                    "Found {} match(es) for '{}':\n{}",
                    total,
                    pattern,
                    lines
                        .iter()
                        .take(display_limit)
                        .copied()
                        .collect::<Vec<_>>()
                        .join("\n")
                );
                if total > display_limit {
                    result.push_str(&format!("\n... and {} more matches", total - display_limit));
                }
                ToolResult {
                    tool_name: "grep".to_string(),
                    output: result,
                    is_error: false,
                }
            }
            1 => ToolResult {
                tool_name: "grep".to_string(),
                output: format!("No matches found for '{}'", pattern),
                is_error: false,
            },
            -1 => ToolResult {
                tool_name: "grep".to_string(),
                output: "Search interrupted or timed out; process terminated".to_string(),
                is_error: true,
            },
            _ => ToolResult {
                tool_name: "grep".to_string(),
                output: format!(
                    "Error searching: {}",
                    if outcome.stderr.trim().is_empty() {
                        format!("grep exited with code {}", outcome.exit_code)
                    } else {
                        outcome.stderr.trim().to_string()
                    }
                ),
                is_error: true,
            },
        },
        Err(message) => tool_error(call, format!("Error searching: {message}")),
    }
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

/// Run the tool-use loop: send the prompt, process tool calls, and return
/// the final response text. Tool requests and results are reported to the
/// host for persistence so the next turn keeps execution history.
pub async fn run_tool_loop(
    prompt: &str,
    sandbox: &Sandbox,
    project_root: &Path,
    endpoint: &crate::config::EndpointConfig,
    model: &str,
    prompt_context: ToolLoopPromptContext,
    host: &mut dyn ToolHost,
) -> Result<String> {
    let adapter = crate::providers::adapter_for(endpoint.provider.clone());
    let goal_state = prompt_context.goal.clone();
    let artifacts_dir = prompt_context.artifacts_dir.clone();
    let mcp_servers = prompt_context.mcp_servers.clone();

    let mut messages = vec![crate::providers::ChatMessage::cacheable(
        "system",
        tool_use_system_instruction(prompt_context.effort, goal_state.as_ref()),
    )];
    if let Some(project) = prompt_context.cacheable_project {
        messages.push(crate::providers::ChatMessage::cacheable("system", project));
    }
    if let Some(skill) = prompt_context.skill_prompt {
        messages.push(crate::providers::ChatMessage::new("system", skill));
    }
    messages.extend(prompt_context.history);
    messages.push(crate::providers::ChatMessage::new("user", prompt));
    crate::pack::dedupe_packed_reads(&mut messages);

    let mut tools = builtin_tool_specs();
    tools.extend(prompt_context.mcp_tools);

    let max_iterations = crate::core::tool_iteration_limit() as usize;
    let mut correction_attempts = 0u32;

    for iteration in 0..max_iterations {
        if crate::core::agent_next(
            crate::core::GOAL_NONE,
            iteration as u32,
            max_iterations as u32,
            crate::interactive::was_interrupted(),
            host.stopped(),
            0,
        ) != crate::core::AgentAction::Request
        {
            return Ok("Execution paused; session checkpoint retained.".into());
        }
        let mut request = crate::providers::ChatRequest {
            model: model.to_string(),
            messages: messages.clone(),
            max_tokens: Some(4096),
            session_id: prompt_context.session_id.clone(),
            tools: tools.clone(),
        };

        host.prepare_request(endpoint, &mut request).await?;
        messages = request.messages.clone();
        if !host.before_request()? {
            return Ok("Execution paused; session checkpoint retained.".into());
        }
        crate::providers::validate_chat_request(&request)?;
        let mut response = String::new();
        let preview_buf = crate::ui::preview_start();

        let streamed = crate::providers::stream_chat_with_retry(
            adapter.as_ref(),
            endpoint,
            request,
            &mut |delta| {
                if crate::interactive::was_interrupted() {
                    return;
                }
                response.push_str(&delta);
                crate::ui::preview_update(&preview_buf, &delta);
            },
        )
        .await;

        crate::ui::preview_stop();
        let outcome = match streamed {
            Ok(outcome) => outcome,
            Err(error) => {
                if !response.is_empty() {
                    host.record_transcript("assistant", &response)?;
                }
                return Err(error);
            }
        };
        host.record_usage(&outcome.usage);

        if crate::interactive::was_interrupted() {
            eprintln!("\n{}", "(interrupted)".dimmed());
            return Ok(response);
        }

        let mut tool_calls = match parse_tool_calls(&response) {
            Ok(calls) => calls,
            Err(parse_error) => {
                if correction_attempts >= 2 {
                    return Err(anyhow!(
                        "the model produced malformed tool calls three times: {parse_error}"
                    ));
                }
                correction_attempts += 1;
                host.record_transcript("assistant", &response)?;
                host.record_transcript(
                    "tool_result",
                    &format!(
                        "Tool protocol error: {parse_error}. Return one valid complete tool block."
                    ),
                )?;
                messages.push(crate::providers::ChatMessage::new(
                    "assistant",
                    response.clone(),
                ));
                messages.push(crate::providers::ChatMessage::new(
                    "user",
                    format!(
                        "Tool protocol error: {parse_error}. Fix the call and repeat it as one complete, valid <tool>{{...}}</tool> block."
                    ),
                ));
                continue;
            }
        };
        for native in outcome.tool_calls {
            tool_calls.push(ToolCall {
                name: native.name,
                arguments: native.arguments,
            });
        }
        if tool_calls.is_empty() {
            return Ok(response);
        }
        correction_attempts = 0;

        messages.push(crate::providers::ChatMessage::new(
            "assistant",
            response.clone(),
        ));
        host.record_transcript("assistant", &response)?;

        for call in &tool_calls {
            if crate::interactive::was_interrupted() {
                eprintln!("\n{}", "(interrupted)".dimmed());
                return Ok(response);
            }
            if call.name == "goal_update" {
                let result_text = match host.goal_update(&call.arguments) {
                    Ok(message) => message,
                    Err(message) => format!("Error: {message}"),
                };
                crate::ui::print_tool_done("updating goal", false);
                let packed = crate::pack::pack_tool_result(
                    "goal_update",
                    result_text.starts_with("Error:"),
                    &result_text,
                    artifacts_dir.as_deref(),
                );
                messages.push(crate::providers::ChatMessage::new(
                    "user",
                    format!("Tool result for '{}':\n{}", call.name, packed.model_visible),
                ));
                host.record_transcript(
                    "tool_result",
                    &format!("Tool result for 'goal_update':\n{}", packed.model_visible),
                )?;
                if host.stopped() {
                    return Ok(result_text);
                }
                continue;
            }
            if let Some(mcp_result) =
                execute_mcp_tool(call, &mcp_servers, sandbox, host.dry_run(), &mut |action| {
                    host.approve(action)
                })
                .await
            {
                let packed = crate::pack::pack_tool_result(
                    &call.name,
                    mcp_result.is_error,
                    &mcp_result.output,
                    artifacts_dir.as_deref(),
                );
                let progress = tool_call_progress(&call.name, &call.arguments);
                crate::ui::print_tool_done(&progress, mcp_result.is_error);
                messages.push(crate::providers::ChatMessage::new(
                    "user",
                    format!("Tool result for '{}':\n{}", call.name, packed.model_visible),
                ));
                host.record_transcript(
                    "tool_result",
                    &format!("Tool result for '{}':\n{}", call.name, packed.model_visible),
                )?;
                if host.stopped() {
                    return Ok(packed.model_visible);
                }
                continue;
            }
            let progress = tool_call_progress(&call.name, &call.arguments);
            crate::ui::print_tool_progress(&progress);
            let result = if let Some(result) =
                authorize_tool(call, sandbox, project_root, host.dry_run(), &mut |action| {
                    host.approve(action)
                }) {
                result
            } else {
                let call = call.clone();
                let sandbox = sandbox.clone();
                let root = project_root.to_path_buf();
                tokio::task::spawn_blocking(move || execute_authorized(&call, &sandbox, &root))
                    .await?
            };
            let packed = crate::pack::pack_tool_result(
                &call.name,
                result.is_error,
                &result.output,
                artifacts_dir.as_deref(),
            );
            crate::ui::print_tool_done(&progress, result.is_error);
            messages.push(crate::providers::ChatMessage::new(
                "user",
                format!("Tool result for '{}':\n{}", call.name, packed.model_visible),
            ));
            host.record_transcript(
                "tool_result",
                &format!(
                    "Tool result for '{}':\n{}\n---full---\n{}",
                    call.name, packed.model_visible, result.output
                ),
            )?;
            if host.stopped() {
                return Ok(packed.model_visible);
            }
        }
        crate::pack::dedupe_packed_reads(&mut messages);
    }
    Ok("Execution paused: 25 provider turns used. Continue with another prompt.".into())
}

/// Execute `mcp__<server>__<tool>` calls. Returns None when the name is not MCP.
async fn execute_mcp_tool(
    call: &ToolCall,
    servers: &[crate::config::McpServerConfig],
    sandbox: &Sandbox,
    dry_run: bool,
    approve: &mut dyn FnMut(&str) -> crate::permissions::ApprovalChoice,
) -> Option<ToolResult> {
    let rest = call.name.strip_prefix("mcp__")?;
    let (server_name, tool_name) = rest.split_once("__")?;
    let server = servers.iter().find(|s| s.name == server_name)?;
    let verdict = sandbox.evaluate(crate::permissions::Operation::Network, None);
    match verdict.decision {
        crate::permissions::PermissionDecision::Deny => {
            return Some(tool_error(call, format!("Denied: {}", verdict.reason)));
        }
        crate::permissions::PermissionDecision::Ask => {
            let choice = approve(&format!("call MCP tool {server_name}/{tool_name}"));
            if !choice.allowed() {
                return Some(tool_error(
                    call,
                    "Skipped: you did not approve this step. The assistant must not retry it or work around the decision.",
                ));
            }
        }
        crate::permissions::PermissionDecision::Allow => {}
    }
    if dry_run {
        return Some(ToolResult {
            tool_name: call.name.clone(),
            output: "dry-run: MCP call not executed".into(),
            is_error: false,
        });
    }
    let mut client = match crate::mcp::McpClient::spawn(server) {
        Ok(client) => client,
        Err(err) => return Some(tool_error(call, format!("MCP spawn failed: {err}"))),
    };
    if let Err(err) = client.initialize().await {
        return Some(tool_error(call, format!("MCP init failed: {err}")));
    }
    let result = match client.call_tool(tool_name, call.arguments.clone()).await {
        Ok(value) => ToolResult {
            tool_name: call.name.clone(),
            output: value.to_string(),
            is_error: false,
        },
        Err(err) => tool_error(call, format!("MCP call failed: {err}")),
    };
    client.shutdown().await;
    Some(result)
}

/// Build a human-readable progress label for a tool call.
fn tool_call_progress(name: &str, arguments: &serde_json::Value) -> String {
    match name {
        "read" => {
            let path = arguments.get("path").and_then(|v| v.as_str()).unwrap_or("");
            format!("reading {path}")
        }
        "write" => {
            let path = arguments.get("path").and_then(|v| v.as_str()).unwrap_or("");
            format!("writing {path}")
        }
        "edit" => {
            let path = arguments.get("path").and_then(|v| v.as_str()).unwrap_or("");
            format!("editing {path}")
        }
        "bash" => {
            let cmd = arguments
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let short = cmd.chars().take(60).collect::<String>();
            format!("running: {short}")
        }
        "glob" => {
            let pattern = arguments
                .get("pattern")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("globbing {pattern}")
        }
        "grep" => {
            let pattern = arguments
                .get("pattern")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("searching for '{pattern}'")
        }
        _ => format!("calling {name}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_flat_tool_arguments() {
        let calls =
            parse_tool_calls(r#"<tool>{"name":"bash","command":"python3 script.py"}</tool>"#)
                .unwrap();

        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "bash");
        assert_eq!(
            calls[0].arguments.get("command").and_then(|v| v.as_str()),
            Some("python3 script.py")
        );
    }

    #[test]
    fn parses_nested_tool_arguments() {
        let calls =
            parse_tool_calls(r#"<tool>{"name":"read","arguments":{"path":"src/main.rs"}}</tool>"#)
                .unwrap();

        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "read");
        assert_eq!(
            calls[0].arguments.get("path").and_then(|v| v.as_str()),
            Some("src/main.rs")
        );
    }

    #[test]
    fn malformed_tool_blocks_are_errors_not_silent_success() {
        let unclosed = parse_tool_calls(r#"Sure! <tool>{"name":"read","arguments":{"path":"x"}}"#);
        assert!(unclosed.is_err());

        let bad_json = parse_tool_calls("<tool>{\"name\": read}</tool>");
        assert!(bad_json.is_err());

        let no_name = parse_tool_calls("<tool>{\"arguments\":{\"path\":\"x\"}}</tool>");
        assert!(no_name.is_err());

        // No tool blocks at all is fine (final response).
        assert!(parse_tool_calls("All done.").unwrap().is_empty());
    }

    #[test]
    fn glob_excludes_secret_files_case_insensitively() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join(".env"), "x=1").unwrap();
        std::fs::write(temp.path().join("SECRETS.YAML"), "x").unwrap();
        std::fs::write(temp.path().join("keep.txt"), "x").unwrap();
        let call = ToolCall {
            name: "glob".into(),
            arguments: serde_json::json!({"pattern": "*"}),
        };
        let result = execute_glob(&call, temp.path());
        assert!(result.output.contains("keep.txt"), "{}", result.output);
        assert!(!result.output.contains(".env"));
        assert!(!result.output.contains("SECRETS.YAML"));
    }

    #[test]
    fn grep_hides_secret_files_case_insensitively() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join(".ENV"), "TOKEN=leak").unwrap();
        std::fs::write(temp.path().join("notes.md"), "TOKEN elsewhere").unwrap();
        let call = ToolCall {
            name: "grep".into(),
            arguments: serde_json::json!({"pattern": "TOKEN"}),
        };
        let result = execute_grep(&call, temp.path());
        assert!(result.output.contains("notes.md"), "{}", result.output);
        assert!(!result.output.contains(".ENV"), "{}", result.output);
    }
}
