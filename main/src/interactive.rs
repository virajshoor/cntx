use anyhow::Result;
use owo_colors::OwoColorize;
use rustyline::{Cmd, DefaultEditor, Event, EventHandler, KeyCode, KeyEvent, Modifiers, Movement};

use crate::app::Runtime;
use crate::permissions::Operation;
use crate::sandbox::SandboxVerdict;

pub async fn run(runtime: &mut Runtime) -> Result<()> {
    let mut editor = DefaultEditor::new()?;
    // Bind Shift+Tab to dedent (remove one level of indentation)
    editor.bind_sequence(
        Event::from(KeyEvent(KeyCode::BackTab, Modifiers::NONE)),
        EventHandler::from(Cmd::Dedent(Movement::WholeLine)),
    );
    // Initialize theme from config
    crate::ui::set_theme(crate::ui::Theme::parse(&runtime.config.ui.theme));
    // Load persistent command history so prior prompts are recallable across
    // shell restarts. Load errors are non-fatal (first run has no history file).
    let history_path = runtime.store.history_path();
    let _ = editor.load_history(&history_path);
    print_greeting(runtime);
    ui_line("Type `/help` for commands, `/status` for the current workspace, `/exit` to quit.");

    while let Ok(line) = editor.readline(&prompt(runtime)) {
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

        runtime.run_prompt(input).await?;
    }

    // Persist command history for the next session.
    let _ = editor.save_history(&history_path);
    Ok(())
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
- `/models` - list cached models and aliases\n\
- `/endpoints` - list endpoints\n\
- `/skills` - list skills\n\
- `/skill <name>` - activate a skill so its prompt is injected into each request\n\
- `/session` - show the current session id\n\
- `/sandbox` - show the edit sandbox policy\n\
- `/mcp` - list MCP servers\n\
- `/api-keys` - list stored API keys, masked\n\
- `/default <model-or-alias>` - set the default model for this session\n\
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
        "  endpoint: {}   model: {}   mode: {:?}",
        endpoint.green(),
        model.green(),
        runtime.mode
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
