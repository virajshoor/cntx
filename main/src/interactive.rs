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

/// Shift+Tab cycles the mode without submitting or discarding the current
/// draft: the handler captures the draft from the line buffer, the loop
/// cycles the mode, and the next readline starts pre-filled with the draft.
struct ShiftTabHandler;

impl ConditionalEventHandler for ShiftTabHandler {
    fn handle(&self, _evt: &Event, _n: u16, _positive: bool, ctx: &EventContext) -> Option<Cmd> {
        let draft = ctx.line().to_string();
        *PENDING_DRAFT.lock().unwrap() = Some(draft);
        SHIFT_TAB_PRESSED.store(true, Ordering::SeqCst);
        Some(Cmd::AcceptLine)
    }
}

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

static SHIFT_TAB_PRESSED: AtomicBool = AtomicBool::new(false);
static PENDING_DRAFT: Mutex<Option<String>> = Mutex::new(None);

/// Set when Ctrl+C is pressed during generation. The streaming loop checks
/// this and breaks out early, returning whatever text was generated so far.
pub static INTERRUPTED: AtomicBool = AtomicBool::new(false);

/// Host-owned cancellation flag for the C core's command runner. The C wait
/// loop polls this int; non-zero terminates the child process group.
pub static CANCEL: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

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
        CANCEL.store(0, Ordering::SeqCst);
    }
}

/// Returns true if the current generation was interrupted by Ctrl+C.
pub fn was_interrupted() -> bool {
    INTERRUPTED.load(Ordering::SeqCst)
}

/// Spawn a background task that listens for Ctrl+C. When a prompt is running,
/// the first Ctrl+C sets the interrupt flag. When idle, Ctrl+C is handled by
/// rustyline (which returns an Interrupted error).
pub async fn wait_for_interrupt() {
    while !was_interrupted() {
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
}

pub fn spawn_ctrl_c_handler() {
    tokio::spawn(async {
        loop {
            tokio::signal::ctrl_c().await.ok();
            if PROMPT_RUNNING.load(Ordering::SeqCst) {
                INTERRUPTED.store(true, Ordering::SeqCst);
                CANCEL.store(1, Ordering::SeqCst);
            }
            // If not running, rustyline's readline will get the SIGINT and
            // return an error, which our loop handles by quitting.
        }
    });
}

/// Shift+Tab press result consumed by the readline loop: cycle the mode and
/// restore the captured draft on the next prompt.
fn take_shift_tab() -> Option<String> {
    if SHIFT_TAB_PRESSED.swap(false, Ordering::SeqCst) {
        let draft = PENDING_DRAFT.lock().unwrap().take().unwrap_or_default();
        Some(draft)
    } else {
        None
    }
}

pub async fn run(runtime: &mut Runtime) -> Result<()> {
    // Configure the editor: emacs mode.
    let config = ConfigBuilder::new().edit_mode(EditMode::Emacs).build();
    let mut editor = DefaultEditor::with_config(config)?;
    // Shift+Tab => accept the line immediately; the loop detects the flag,
    // cycles the permission mode, and restores the draft (like Claude Code).
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
    ui_line(
        "Press Shift+Tab to cycle permission modes (auto-approve, all-approve, manual-approve).",
    );

    // A draft captured by Shift+Tab is restored here so the user can keep
    // editing; pressing Shift+Tab alone starts with an empty draft.
    let mut pending_draft: Option<String> = None;

    loop {
        let readline_result = match pending_draft.take() {
            Some(draft) => editor.readline_with_initial(&prompt(runtime), (&draft, "")),
            None => editor.readline(&prompt(runtime)),
        };
        let line = match readline_result {
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

        // Shift+Tab: cycle the mode; keep the draft for the next prompt.
        if let Some(draft) = take_shift_tab() {
            cycle_mode(runtime);
            pending_draft = Some(draft);
            continue;
        }

        let input = line.trim().to_string();
        if input.is_empty() {
            continue;
        }
        let _ = editor.add_history_entry(&input);

        if input.starts_with('/') {
            match handle_slash(runtime, &input).await {
                Ok(quit) => {
                    if quit {
                        break;
                    }
                }
                Err(error) => {
                    // Slash command errors leave the shell usable.
                    println!("{} {error}", "error:".red());
                }
            }
            continue;
        }

        // The entire input is one prompt; ordinary text containing shell
        // syntax like ` && ` is never split into a queue.
        match runtime.run_prompt(&input).await {
            Ok(()) => {}
            Err(error) => {
                // Prompt failures keep the shell usable and the saved state.
                println!("{} {error}", "error:".red());
            }
        }
        if was_interrupted() {
            println!("{}", "(interrupted)".dimmed());
        }
    }

    // Persist command history for the next session.
    let _ = editor.save_history(&history_path);
    Ok(())
}

/// Cycle through the canonical modes from the C core:
/// auto-approve -> all-approve -> manual-approve -> auto-approve.
/// Legacy extra modes (counsel, file-only) return to auto-approve.
fn cycle_mode(runtime: &mut Runtime) {
    runtime.mode = runtime.mode.next();
    // Update runtime and sandbox policy together so displayed state and
    // enforced policy never disagree.
    runtime.sandbox.set_mode(runtime.mode);
    println!(
        "mode: {} - {}",
        runtime.mode.as_str(),
        runtime.mode.description()
    );
}

fn prompt(runtime: &Runtime) -> String {
    let endpoint = runtime.config.primary_endpoint.as_deref().unwrap_or("none");
    let model = runtime
        .model_override
        .as_deref()
        .or(runtime.config.default_model.as_deref())
        .unwrap_or("auto");
    let mode = runtime.mode.as_str();
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
- `/mode` - show the current approval mode\n\
- `/mode <name>` - switch modes for this session: auto-approve, all-approve, manual-approve, counsel, file-only, plan\n\
- `/model` - show the effective endpoint and model\n\
- `/model <id-or-alias>` - switch the model for this session (`/model auto` restores automatic selection)\n\
- `/models` - list cached models grouped by endpoint\n\
- `/effort [low|medium|high]` - show or set investigation and verification depth\n\
- `/goal` - show goal status\n\
- `/goal <objective>` - start a goal-oriented run\n\
- `/goal resume|pause|cancel` - control the active goal\n\
- `/goal new <objective>` - start a goal whose text begins with a reserved word\n\
- `/resume [session-id]` - resume the latest or given session in this workspace\n\
- `/clear` - save the old session and start a fresh one\n\
- `/compact` - summarize the conversation so far; the session id stays the same\n\
- `/cost` - show estimated token usage and cost for this session\n\
- `/usage` - show last-turn and session provider usage (input/output/cache)\n\
- `/endpoints` - list endpoints\n\
- `/skills` - list skills\n\
- `/skill <name>` - activate a skill so its prompt is injected into each request\n\
- `/session` - show the current session id\n\
- `/sandbox` - show the edit sandbox policy\n\
- `/mcp` - list MCP servers\n\
- `/api-keys` - list stored API keys, masked\n\
- `/default <model-or-alias>` - set the persistent default model\n\
- `/apply` - toggle apply mode and write `path=` fenced blocks through the sandbox\n\
- `/dry-run` - toggle dry-run: no file writes or shell execution\n\
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
            if let Some(name) = parts.get(1).copied() {
                // Validate first: invalid input changes nothing.
                match Mode::parse(name) {
                    Some(mode) => {
                        runtime.mode = mode;
                        runtime.sandbox.set_mode(mode);
                        println!(
                            "mode: {} - {}",
                            runtime.mode.as_str(),
                            runtime.mode.description()
                        );
                    }
                    None => println!(
                        "invalid mode '{name}'; use auto-approve, all-approve, manual-approve, counsel, file-only, or plan"
                    ),
                }
            } else {
                println!(
                    "mode: {} - {}",
                    runtime.mode.as_str(),
                    runtime.mode.description()
                );
                println!(
                    "modes: auto-approve, all-approve, manual-approve, counsel, file-only, plan"
                );
            }
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
        Some("/goal") => {
            let rest = input
                .split_once("/goal")
                .map(|(_, rest)| rest.trim())
                .unwrap_or("");
            if rest.is_empty() {
                println!("{}", runtime.goal_status_text());
                return Ok(false);
            }
            if rest == "pause" {
                let message = runtime.pause_goal()?;
                println!("{message}");
                return Ok(false);
            }
            if rest == "cancel" {
                let message = runtime.cancel_goal()?;
                println!("{message}");
                return Ok(false);
            }
            if rest == "resume" {
                let message = runtime.resume_goal().await?;
                println!("{message}");
                return Ok(false);
            }
            let objective = if let Some(new_rest) = rest.strip_prefix("new ") {
                // Explicit form for objectives that begin with reserved words.
                new_rest.trim().to_string()
            } else {
                rest.to_string()
            };
            let message = runtime.start_goal(&objective).await?;
            println!("{message}");
            Ok(false)
        }
        Some("/resume") => {
            let session_id = parts.get(1).copied();
            let store = crate::sessions::SessionStore::new(&runtime.store);
            let loaded = match session_id {
                Some(id) => store.load(id)?,
                None => {
                    // Latest for the current workspace: sessions from other
                    // directories are never silently resumed here. Legacy
                    // sessions without a root are treated as this
                    // workspace's with a visible notice below.
                    let current = runtime.sandbox.project_root().to_path_buf();
                    store
                        .list()?
                        .into_iter()
                        .find(|session| {
                            session.workspace_root.as_deref() == Some(current.as_path())
                                || session.workspace_root.is_none()
                        })
                        .ok_or_else(|| anyhow::anyhow!("no saved sessions in this workspace yet"))?
                }
            };
            // Never silently switch workspace roots.
            let current = runtime.sandbox.project_root().to_path_buf();
            if let Some(session_root) = loaded.workspace_root.as_ref() {
                if session_root != &current {
                    println!(
                        "{}",
                        format!(
                            "session {} belongs to {}; launch cntx in that directory to resume it",
                            loaded.id,
                            session_root.display()
                        )
                        .yellow()
                    );
                    return Ok(false);
                }
            } else {
                println!(
                    "{}",
                    "note: this session predates workspace tracking; resuming in the current workspace".yellow()
                );
            }
            println!("resumed session {}", loaded.id);
            runtime.session = loaded;
            if runtime.session.goal.is_some() {
                println!("{}", runtime.goal_status_text());
            }
            Ok(false)
        }
        Some("/clear") => {
            // Save the old session, then start fresh. Selected endpoint,
            // model, mode, and effort are kept.
            runtime.save_session()?;
            let mut fresh = crate::sessions::Session::new("interactive");
            fresh.workspace_root = Some(runtime.sandbox.project_root().to_path_buf());
            runtime.session = fresh;
            runtime.last_apply_outcomes.clear();
            println!("started a fresh session: {}", runtime.session.id);
            Ok(false)
        }
        Some("/compact") => {
            let message = runtime.compact_session().await?;
            println!("{message}");
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
        Some("/usage") => {
            let ct = &runtime.cost_tracker;
            let source = if ct.provider_reported {
                "provider"
            } else {
                "estimate"
            };
            println!("source: {source}");
            println!(
                "last turn: in={} out={} cache_read={} cache_write={}",
                ct.last_turn.input_tokens,
                ct.last_turn.output_tokens,
                ct.last_turn.cache_read_tokens,
                ct.last_turn.cache_write_tokens
            );
            println!(
                "session:   in={} out={} cache_read={} cache_write={} requests={}",
                ct.input_tokens,
                ct.output_tokens,
                ct.cache_read_tokens,
                ct.cache_write_tokens,
                ct.request_count
            );
            println!("est. cost: ${:.4}", ct.estimated_cost_usd());
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
                if value == "auto" {
                    runtime.model_override = None;
                    println!("model: automatic selection (config default or routing)");
                    return Ok(false);
                }
                // An endpoint-bound alias selects its endpoint too. Only an
                // explicit CLI endpoint override conflicts; the configured
                // primary is not a conflict, the alias simply switches the
                // session's endpoint.
                if let Some(alias) = runtime.config.aliases.get(value) {
                    if let Some(alias_endpoint) = alias.endpoint.as_ref() {
                        if runtime.endpoint_override.is_some()
                            && runtime.endpoint_override.as_deref() != Some(alias_endpoint.as_str())
                        {
                            println!(
                                "{}",
                                format!(
                                    "alias '{value}' belongs to endpoint '{alias_endpoint}' but --endpoint {} was passed; restart without --endpoint or use /model auto",
                                    runtime.endpoint_override.as_deref().unwrap_or_default()
                                )
                                .yellow()
                            );
                            return Ok(false);
                        }
                        if runtime.endpoint_override.is_none() {
                            runtime.endpoint_override = Some(alias_endpoint.clone());
                        }
                    }
                    runtime.model_override = Some(alias.model.clone());
                    println!(
                        "model set to {} for this session{}",
                        alias.model,
                        runtime
                            .endpoint_override
                            .as_deref()
                            .map(|endpoint| format!(" on endpoint {endpoint}"))
                            .unwrap_or_default()
                    );
                } else {
                    runtime.model_override = Some(value.to_string());
                    println!("model set to {value} for this session");
                }
            } else {
                // Show the effective endpoint/model resolved the same way a
                // real request resolves it, not stale config values.
                match runtime.model_override.as_deref() {
                    Some(model) => {
                        let resolved = runtime.resolve_endpoint().and_then(|(name, endpoint)| {
                            runtime
                                .resolve_model(&name, &endpoint, 0)
                                .map(|model| (name, model))
                        });
                        match resolved {
                            Ok((name, resolved)) => println!(
                                "model: {resolved} on endpoint {name} (session override: {model})"
                            ),
                            Err(_) => println!("model: {model} (session override)"),
                        }
                    }
                    None => {
                        let resolved = runtime.resolve_endpoint().and_then(|(name, endpoint)| {
                            runtime
                                .resolve_model(&name, &endpoint, 0)
                                .map(|model| (name, model))
                        });
                        match resolved {
                            Ok((name, resolved)) => println!(
                                "model: {resolved} on endpoint {name} (automatic selection)"
                            ),
                            Err(_) => println!("model: auto (automatic selection: routing)"),
                        }
                    }
                }
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
                    "off (chat-only)".dimmed().to_string()
                }
            );
            Ok(false)
        }
        Some("/dry-run") => {
            runtime.dry_run = !runtime.dry_run;
            println!(
                "dry run: {}",
                if runtime.dry_run {
                    "on (mutations and shell execution are blocked; changes are described only)"
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
        "  endpoint: {}   model: {}   mode: {}   effort: {}",
        endpoint.green(),
        model.green(),
        runtime.mode.as_str(),
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
    if let Some(goal) = runtime.session.goal.as_ref() {
        println!("  goal: {} ({})", goal.objective, goal.status);
    }
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
    println!("mode: {}", summary.mode.as_str());
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
