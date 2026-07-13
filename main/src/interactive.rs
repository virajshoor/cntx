use anyhow::Result;
use owo_colors::OwoColorize;
use rustyline::config::{Builder as ConfigBuilder, EditMode};
use rustyline::{
    Cmd, ConditionalEventHandler, DefaultEditor, Event, EventContext, EventHandler, KeyCode,
    KeyEvent, Modifiers,
};

use crate::app::Runtime;
use crate::permissions::Mode;
use crate::permissions::Operation;
use crate::sandbox::SandboxVerdict;

struct ShiftTabHandler;

impl ConditionalEventHandler for ShiftTabHandler {
    fn handle(&self, _evt: &Event, _n: u16, _positive: bool, _ctx: &EventContext) -> Option<Cmd> {
        SHIFT_TAB_PRESSED.store(true, Ordering::SeqCst);
        Some(Cmd::AcceptLine)
    }
}

use std::sync::atomic::{AtomicBool, Ordering};

static SHIFT_TAB_PRESSED: AtomicBool = AtomicBool::new(false);

/// Set when Ctrl+C is pressed during generation. The streaming loop checks
/// this and breaks out early, returning whatever text was generated so far.
pub static INTERRUPTED: AtomicBool = AtomicBool::new(false);

/// True when a prompt is currently running. Used to distinguish "interrupt"
/// (first Ctrl+C while generating) from "quit" (Ctrl+C while idle).
static PROMPT_RUNNING: AtomicBool = AtomicBool::new(false);

/// Returns true if a prompt is currently running.
pub fn is_prompt_running() -> bool {
    PROMPT_RUNNING.load(Ordering::SeqCst)
}

/// Set the prompt-running flag. Called by `run_prompt` before generation starts.
pub fn set_prompt_running(running: bool) {
    PROMPT_RUNNING.store(running, Ordering::SeqCst);
    if running {
        INTERRUPTED.store(false, Ordering::SeqCst);
    }
}

/// Returns true if the current generation was interrupted by Ctrl+C.
pub fn was_interrupted() -> bool {
    INTERRUPTED.load(Ordering::SeqCst)
}

/// Spawn a background task that listens for Ctrl+C. When a prompt is running,
/// the first Ctrl+C sets the interrupt flag. When idle, Ctrl+C is handled by
/// rustyline (which returns an Interrupted error).
pub fn spawn_ctrl_c_handler() {
    tokio::spawn(async {
        loop {
            tokio::signal::ctrl_c().await.ok();
            if PROMPT_RUNNING.load(Ordering::SeqCst) {
                INTERRUPTED.store(true, Ordering::SeqCst);
            }
            // If not running, rustyline's readline will get the SIGINT and
            // return an error, which our loop handles by quitting.
        }
    });
}

pub async fn run(runtime: &mut Runtime) -> Result<()> {
    // Spawn background Ctrl+C handler for interrupting generation.
    spawn_ctrl_c_handler();
    // Configure the editor: emacs mode.
    let config = ConfigBuilder::new().edit_mode(EditMode::Emacs).build();
    let mut editor = DefaultEditor::with_config(config)?;
    // Shift+Tab => accept the line immediately; the loop detects the flag
    // and cycles the permission mode (like Claude Code).
    editor.bind_sequence(
        Event::from(KeyEvent(KeyCode::BackTab, Modifiers::NONE)),
        EventHandler::Conditional(Box::new(ShiftTabHandler)),
    );
    // Initialize theme from config
    crate::ui::set_theme(crate::ui::Theme::parse(&runtime.config.ui.theme));
    // Load persistent command history so prior prompts are recallable across
    // shell restarts. Load errors are non-fatal (first run has no history file).
    let history_path = runtime.store.history_path();
    let _ = editor.load_history(&history_path);
    print_greeting(runtime);
    ui_line("Type `/help` for commands, `/status` for the current workspace, `/exit` to quit.");
    ui_line("Press Shift+Tab to cycle permission modes.");

    loop {
        let line = match editor.readline(&prompt(runtime)) {
            Ok(line) => line,
            Err(rustyline::error::ReadlineError::Interrupted) => {
                // Ctrl+C while idle at the prompt: quit.
                println!();
                break;
            }
            Err(rustyline::error::ReadlineError::Eof) => {
                // Ctrl+D: quit.
                break;
            }
            Err(e) => return Err(e.into()),
        };

        // Check if Shift+Tab was pressed (the handler accepted the line).
        if SHIFT_TAB_PRESSED.swap(false, Ordering::SeqCst) {
            cycle_mode(runtime);
            continue;
        }

        let input = line.trim();
        if input.is_empty() {
            continue;
        }
        let _ = editor.add_history_entry(input);

        if input.starts_with('/') {
            if handle_slash(runtime, input).await? {
                break;
            }
            continue;
        }

        // Support message queueing: prompts separated by ` && ` are processed
        // in sequence. Each is treated as a separate conversation turn.
        let prompts: Vec<&str> = input.split(" && ").collect();
        for queued in prompts {
            let queued = queued.trim();
            if queued.is_empty() {
                continue;
            }
            if queued.starts_with('/') {
                if handle_slash(runtime, queued).await? {
                    break;
                }
                continue;
            }
            runtime.run_prompt(queued).await?;
            // If the prompt was interrupted, don't process the rest of the queue.
            if was_interrupted() {
                println!("{}", "(interrupted)".dimmed());
                break;
            }
        }
    }

    // Persist command history for the next session.
    let _ = editor.save_history(&history_path);
    Ok(())
}

/// Cycle through permission modes: Auto -> Allow -> Counsel -> FileOnly -> Auto.
fn cycle_mode(runtime: &mut Runtime) {
    runtime.mode = match runtime.mode {
        Mode::Auto => Mode::Allow,
        Mode::Allow => Mode::Counsel,
        Mode::Counsel => Mode::FileOnly,
        Mode::FileOnly => Mode::RequestPermission,
        Mode::RequestPermission => Mode::Auto,
    };
    runtime.sandbox.set_mode(runtime.mode);
    println!("mode: {:?} - {}", runtime.mode, runtime.mode.description());
}

fn prompt(runtime: &Runtime) -> String {
    let endpoint = runtime.config.primary_endpoint.as_deref().unwrap_or("none");
    let model = runtime
        .model_override
        .as_deref()
        .or(runtime.config.default_model.as_deref())
        .unwrap_or("auto");
    let mode = match runtime.mode {
        crate::permissions::Mode::Auto => "auto",
        crate::permissions::Mode::Counsel => "counsel",
        crate::permissions::Mode::Allow => "allow",
        crate::permissions::Mode::RequestPermission => "ask",
        crate::permissions::Mode::FileOnly => "files",
    };
    let apply = if runtime.apply { "+apply" } else { "" };
    let dry_run = if runtime.dry_run { "+dry-run" } else { "" };
    let safety = if runtime.sandbox.enabled() {
        "sandbox"
    } else {
        "unsafe"
    };
    format!(
        "{} {} {}/{} {} {}{}{} ",
        "cntx".cyan().bold(),
        "›".dimmed(),
        endpoint,
        model,
        mode.dimmed(),
        safety.dimmed(),
        if apply.is_empty() {
            String::new()
        } else {
            format!(" {}", apply.green())
        },
        if dry_run.is_empty() {
            String::new()
        } else {
            format!(" {}", dry_run.yellow())
        }
    )
}

fn print_greeting(runtime: &Runtime) {
    println!("{}", "Cntx Code".cyan().bold());
    print_status(runtime);
}

async fn handle_slash(runtime: &mut Runtime, input: &str) -> Result<bool> {
    let parts = input.split_whitespace().collect::<Vec<_>>();
    match parts.first().copied() {
        Some("/exit") | Some("/quit") => Ok(true),
        Some("/help") => {
            crate::ui::print_markdown(
                "**Commands**\n\n\
- `/help` - show this help\n\
- `/status` - show endpoint, model, mode, sandbox, and apply state\n\
- `/mode` - show the active permission mode\n\
- `/model <model>` - switch the model for this session (e.g. `/model gpt-4o`)\n\
- `/model` - show the current model\n\
- `/effort [low|medium|high]` - show or set investigation and verification depth\n\
- `/clear` - start a fresh conversation session\n\
- `/compact` - summarize the conversation so far and start a fresh context\n\
- `/cost` - show estimated token usage and cost for this session\n\
- `/models` - list cached models and aliases\n\
- `/endpoints` - list endpoints\n\
- `/skills` - list skills\n\
- `/skill <name>` - activate a skill so its prompt is injected into each request\n\
- `/session` - show the current session id\n\
- `/sandbox` - show the edit sandbox policy\n\
- `/mcp` - list MCP servers\n\
- `/api-keys` - list stored API keys, masked\n\
- `/default <model-or-alias>` - set the persistent default model\n\
- `/apply` - toggle apply mode and write `path=` fenced blocks through the sandbox\n\
- `/dry-run` - toggle apply previews without file writes\n\
- `/checklist` - show the files from the last apply run\n\
- `/theme` - toggle between dark and light mode\n\
- `/exit` - quit\n",
            );
            Ok(false)
        }
        Some("/status") => {
            print_status(runtime);
            Ok(false)
        }
        Some("/mode") => {
            println!("mode: {:?} - {}", runtime.mode, runtime.mode.description());
            Ok(false)
        }
        Some("/effort") => {
            if let Some(value) = parts.get(1).copied() {
                match crate::config::Effort::parse(value) {
                    Some(effort) => {
                        runtime.effort = effort;
                        runtime.config.ui.effort = effort;
                        runtime.store.save(&runtime.config)?;
                        println!("effort: {} - {}", effort.as_str(), effort.instruction());
                    }
                    None => println!("invalid effort '{value}'; use low, medium, or high"),
                }
            } else {
                println!(
                    "effort: {} - {}",
                    runtime.effort.as_str(),
                    runtime.effort.instruction()
                );
            }
            Ok(false)
        }
        Some("/clear") => {
            runtime.session = crate::sessions::Session::new("interactive");
            runtime.last_apply_outcomes.clear();
            println!("started a fresh session: {}", runtime.session.id);
            Ok(false)
        }
        Some("/compact") => {
            let msg_count = runtime.session.messages.len();
            if msg_count <= 4 {
                println!("nothing to compact; only {msg_count} messages in this session");
                return Ok(false);
            }
            // Keep the last 2 turns (4 messages) and summarize the rest.
            let to_summarize: Vec<_> = runtime.session.messages[..msg_count - 4]
                .iter()
                .map(|m| {
                    format!(
                        "{}: {}",
                        m.role,
                        m.content.chars().take(500).collect::<String>()
                    )
                })
                .collect();
            let summary_prompt = format!(
                "Summarize the following conversation in 3-5 bullet points. Keep key decisions, file names, and context:\n\n{}",
                to_summarize.join("\n\n")
            );
            // Do a quick model call to summarize.
            let endpoint_name = runtime.config.primary_endpoint.clone().unwrap_or_default();
            let endpoint = runtime.config.endpoints.get(&endpoint_name).cloned();
            if let Some(endpoint) = endpoint {
                let model = runtime
                    .model_override
                    .clone()
                    .or_else(|| runtime.config.default_model.clone())
                    .unwrap_or_else(|| endpoint.default_model.clone().unwrap_or_default());
                let request = crate::providers::ChatRequest {
                    model,
                    messages: vec![crate::providers::ChatMessage {
                        role: "user".to_string(),
                        content: summary_prompt,
                    }],
                    max_tokens: Some(512),
                };
                println!("compacting {} messages...", msg_count - 4);
                let summary = runtime.generate(&endpoint, request).await?;
                let last_messages = runtime.session.messages[msg_count - 4..].to_vec();
                runtime.session = crate::sessions::Session::new("interactive");
                runtime.session.push(
                    "system",
                    format!("Previous conversation summary:\n{summary}"),
                );
                for msg in last_messages {
                    runtime.session.push(&msg.role, &msg.content);
                }
                crate::sessions::SessionStore::new(&runtime.store).save(&runtime.session)?;
                println!("compacted to {} messages", runtime.session.messages.len());
            } else {
                println!("no endpoint configured; cannot compact");
            }
            Ok(false)
        }
        Some("/cost") => {
            let ct = &runtime.cost_tracker;
            println!("requests:    {}", ct.request_count);
            println!("input tokens:  {}", ct.input_tokens);
            println!("output tokens: {}", ct.output_tokens);
            println!("total tokens:  {}", ct.input_tokens + ct.output_tokens);
            println!("est. cost:    ${:.4}", ct.estimated_cost_usd());
            Ok(false)
        }
        Some("/models") => {
            runtime.print_models()?;
            Ok(false)
        }
        Some("/endpoints") => {
            runtime.print_endpoints();
            Ok(false)
        }
        Some("/skills") => {
            runtime.print_skills()?;
            Ok(false)
        }
        Some("/skill") => {
            if let Some(name) = parts.get(1).copied() {
                let store =
                    crate::skills::SkillStore::new(&runtime.store, runtime.sandbox.project_root());
                match store.get(name) {
                    Ok(Some(skill)) => {
                        runtime.active_skill = Some(skill.clone());
                        println!("active skill: {} - {}", skill.name, skill.description);
                    }
                    Ok(None) => {
                        println!("no skill named '{name}'; use /skills to list");
                    }
                    Err(e) => {
                        println!("error loading skill: {e}");
                    }
                }
            } else if let Some(skill) = runtime.active_skill.as_ref() {
                println!("active skill: {} - {}", skill.name, skill.description);
            } else {
                println!("no active skill; set one with /skill <name>");
            }
            Ok(false)
        }
        Some("/session") => {
            println!("session: {}", runtime.session.id);
            Ok(false)
        }
        Some("/sandbox") => {
            print_sandbox(&runtime.sandbox);
            Ok(false)
        }
        Some("/mcp") => {
            println!("configured MCP servers:");
            for server in runtime.config.mcp.servers.values() {
                let marker = if server.enabled { "*" } else { " " };
                let built_in = if server.built_in { " (built-in)" } else { "" };
                println!("{marker} {}{built_in}", server.name);
            }
            println!("use `cntx mcp tools <name>` to connect and list exposed tools");
            Ok(false)
        }
        Some("/api-keys") => {
            let secrets = crate::api_keys::load(&runtime.store)?;
            if secrets.keys.is_empty() {
                println!("no keys stored; add one with `cntx api-key add --provider anthropic`");
            } else {
                for provider in secrets.keys.keys() {
                    let key = secrets.get(provider).unwrap_or_default();
                    println!("{}", crate::api_keys::ApiSecrets::masked(provider, key));
                }
            }
            Ok(false)
        }
        Some("/model") => {
            if let Some(value) = parts.get(1).copied() {
                runtime.model_override = Some(value.to_string());
                println!("model set to {value} for this session");
            } else {
                let current = runtime
                    .model_override
                    .as_deref()
                    .or(runtime.config.default_model.as_deref())
                    .unwrap_or("auto");
                println!("model: {current}");
            }
            Ok(false)
        }
        Some("/default") => {
            if let Some(value) = parts.get(1).copied() {
                runtime.config.default_model = Some(value.to_string());
                runtime.store.save(&runtime.config)?;
                println!("default model set to {value} for this session");
            } else if let Some(default) = runtime.config.default_model.as_deref() {
                println!("default model: {default}");
            } else {
                println!("no default model; set one with /default <model-or-alias>");
            }
            Ok(false)
        }
        Some("/apply") => {
            runtime.apply = !runtime.apply;
            println!(
                "apply mode: {}",
                if runtime.apply {
                    "on (files the model emits with path= are written through the sandbox)"
                        .green()
                        .to_string()
                } else {
                    "off".dimmed().to_string()
                }
            );
            Ok(false)
        }
        Some("/tools") => {
            runtime.tool_use = !runtime.tool_use;
            println!(
                "tool-use: {}",
                if runtime.tool_use {
                    "on (the model can read, write, edit files and run shell commands)"
                        .green()
                        .to_string()
                } else {
                    "off".dimmed().to_string()
                }
            );
            Ok(false)
        }
        Some("/dry-run") => {
            runtime.dry_run = !runtime.dry_run;
            println!(
                "dry run: {}",
                if runtime.dry_run {
                    "on (apply previews are shown but files are not written)"
                        .yellow()
                        .to_string()
                } else {
                    "off".dimmed().to_string()
                }
            );
            Ok(false)
        }
        Some("/checklist") => {
            if runtime.last_apply_outcomes.is_empty() {
                println!("no files applied yet; enable /apply and run a prompt");
            } else {
                crate::apply::print_checklist(&runtime.last_apply_outcomes);
            }
            Ok(false)
        }
        Some("/theme") => {
            let new_theme = crate::ui::current_theme().toggle();
            crate::ui::set_theme(new_theme);
            runtime.config.ui.theme = new_theme.as_str().to_string();
            runtime.store.save(&runtime.config)?;
            println!("theme set to {}", new_theme.as_str());
            Ok(false)
        }
        Some(command) => {
            println!("unknown slash command: {command}");
            Ok(false)
        }
        None => Ok(false),
    }
}

fn print_status(runtime: &Runtime) {
    let endpoint = runtime
        .config
        .primary_endpoint
        .as_deref()
        .unwrap_or("<none>");
    let model = runtime
        .model_override
        .as_deref()
        .or(runtime.config.default_model.as_deref())
        .unwrap_or("<auto>");
    println!(
        "  endpoint: {}   model: {}   mode: {:?}   effort: {}",
        endpoint.green(),
        model.green(),
        runtime.mode,
        runtime.effort.as_str()
    );
    println!(
        "  sandbox: {}   apply: {}   dry-run: {}   session: {}",
        if runtime.sandbox.enabled() {
            "on".green().to_string()
        } else {
            "off".red().to_string()
        },
        if runtime.apply {
            "on".green().to_string()
        } else {
            "off".dimmed().to_string()
        },
        if runtime.dry_run {
            "on".yellow().to_string()
        } else {
            "off".dimmed().to_string()
        },
        runtime.session.id
    );
}

fn ui_line(text: &str) {
    crate::ui::print_markdown(text);
}

fn print_sandbox(sandbox: &crate::sandbox::Sandbox) {
    let summary = sandbox.summary();
    println!(
        "sandbox: {}",
        if summary.enabled {
            "enabled"
        } else {
            "DISABLED (dangerous)"
        }
    );
    println!("mode: {:?}", summary.mode);
    println!("project root: {}", summary.project_root.display());
    println!("writable roots:");
    for root in &summary.allow_write_roots {
        println!("  - {}", root.display());
    }
    let demo_write = summary.project_root.join("cntx-sandbox-check.txt");
    let write_verdict = sandbox.evaluate(Operation::WriteFile, Some(&demo_write));
    print_verdict("write project file", &write_verdict);
    let shell_verdict = sandbox.evaluate(Operation::Shell, None);
    print_verdict("shell", &shell_verdict);
    let network_verdict = sandbox.evaluate(Operation::Network, None);
    print_verdict("network", &network_verdict);
}

fn print_verdict(label: &str, verdict: &SandboxVerdict) {
    println!("  {label}: {:?} ({})", verdict.decision, verdict.reason);
}
