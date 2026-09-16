use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use chrono::Utc;
use clap::CommandFactory;
use clap_complete::generate;
use owo_colors::OwoColorize;
use rustyline::DefaultEditor;
use serde::Serialize;

use crate::api_keys;
use crate::cli::{
    ApiKeyCommand, Cli, Command, ConfigCommand, EndpointArgs, InitArgs, McpCommand, MemoryCommand,
    ModelCommand, ProviderCommand, SessionCommand, SkillCommand,
};
use crate::config::{
    load_custom_provider_import, load_endpoint_import, load_mcp_server_import, AppConfig,
    ConfigStore, CustomProvider, CustomProviderKind, Effort, EndpointConfig, McpServerConfig,
    ModelAlias, ProviderKind,
};
use crate::context::{build_prompt_input_with_scan, PromptContextReport};
use crate::counsel::{build_evaluation_prompt, build_worker_prompt, plan_counsel, CounselPlan};
use crate::errors::CntxError;
use crate::interactive;
use crate::mcp;
use crate::models::{refresh_models, ModelCache};
use crate::optimizer::PromptOptimizer;
use crate::permissions::Mode;
use crate::providers::{adapter_for, validate_chat_request, ChatMessage, ChatRequest};
use crate::router::ModelRouter;
use crate::sandbox::Sandbox;
use crate::sessions::{Session, SessionMessage, SessionStore};
use crate::skills::SkillStore;
use crate::ui;

/// Base system prompt injected into every request. Tells the model to match
/// the user's language, be concise, and write correct code.
const BASE_SYSTEM_PROMPT: &str = "You are Cntx Code, a coding assistant running locally in the user's terminal. \
You have direct access to the user's filesystem and can read, write, and edit files, run shell commands, \
and search the project. You are NOT a browser-based chat assistant — you run on the user's machine. \
Respond in the same language the user writes in (English, Chinese, Spanish, etc.). \
Be concise and direct. Write correct, working code. Use markdown for formatting. \
Do not add unnecessary preamble or postamble.";

fn system_prompt(effort: Effort) -> String {
    format!(
        "{BASE_SYSTEM_PROMPT}\n\nEffort level: {}. {}",
        effort.as_str(),
        effort.instruction()
    )
}

pub async fn run(cli: Cli) -> Result<()> {
    let store = ConfigStore::from_standard_locations()?;
    // First-boot auto-setup: create the secrets file so the tool is ready to
    // receive API keys on any machine after a fresh build.
    api_keys::ensure_secrets_file(&store)?;
    let mut config = store.load()?;

    if cli.docs {
        return handle_docs();
    }

    if cli.refresh_models {
        let report = refresh_with_spinner(&config, &store).await?;
        ui::print_refresh_report(&report);
        return Ok(());
    }

    match cli.command {
        Some(Command::Endpoint(args)) => handle_endpoint(args, &store, &mut config).await,
        Some(Command::Model(command)) => handle_model(command, &store, &mut config).await,
        Some(Command::Provider(command)) => handle_provider(command, &store, &mut config),
        Some(Command::ApiKey(command)) => handle_api_key(command, &store),
        Some(Command::Init(args)) => handle_init(args, &store, &mut config),
        Some(Command::Bench { json, ref prompt }) => {
            handle_bench(&cli, &store, &config, json, prompt)
        }
        Some(Command::Demo) => handle_demo(),
        Some(Command::Completions { shell }) => {
            let mut command = Cli::command();
            let bin_name = command.get_name().to_string();
            generate(shell, &mut command, bin_name, &mut io::stdout());
            Ok(())
        }
        Some(Command::Memory(command)) => handle_memory(command),
        Some(Command::Mcp(command)) => handle_mcp(command, &store, &mut config).await,
        Some(Command::Config(command)) => handle_config(command, &store, &config),
        Some(Command::Session(command)) => handle_session(command, &store).await,
        Some(Command::Skill(command)) => handle_skill(command, &store),
        Some(Command::Sandbox { yaml }) => handle_sandbox(&cli, &config, yaml),
        Some(Command::Doctor { fix, json, verify }) => {
            handle_doctor(&store, &mut config, fix, json, verify)
        }
        None => {
            let prompt = cli.prompt.join(" ");
            let interactive = prompt.trim().is_empty();
            // The mode stays exactly as configured; the default `auto-approve`
            // policy already allows reads and asks before writes/commands.
            // Interactive upgrades are never applied silently.
            let sandbox = build_sandbox_with_mode(&cli, cli.mode);
            // Tool mode is the default for interactive sessions and one-shot
            // prompts so both can create/edit/run without --tool-use.
            // `--chat-only` forces text-only operation. `--tool-use` remains a
            // compatible explicit flag. One-shot `--apply` keeps the separate
            // apply path unless tool mode is explicitly selected.
            let tool_use = if cli.chat_only {
                false
            } else if cli.tool_use {
                true
            } else {
                !cli.apply || interactive
            };
            let effort = cli.effort.unwrap_or(config.ui.effort);
            let mut runtime = Runtime::new(
                config,
                store,
                RuntimeOptions {
                    endpoint_override: cli.endpoint,
                    model_override: cli.model,
                    mode: cli.mode,
                    effort,
                    apply: cli.apply,
                    dry_run: cli.dry_run,
                    sandbox,
                    tool_use,
                },
            )?;
            if !prompt.trim().is_empty() {
                runtime.run_prompt(&prompt).await
            } else if cli.no_interactive {
                Err(anyhow!("--no-interactive requires a prompt"))
            } else {
                interactive::run(&mut runtime).await
            }
        }
    }
}

pub struct Runtime {
    pub config: AppConfig,
    pub store: ConfigStore,
    pub endpoint_override: Option<String>,
    pub model_override: Option<String>,
    pub mode: Mode,
    pub effort: Effort,
    pub apply: bool,
    pub dry_run: bool,
    pub tool_use: bool,
    pub sandbox: Sandbox,
    pub session: Session,
    pub last_apply_outcomes: Vec<crate::apply::ApplyOutcome>,
    pub active_skill: Option<crate::skills::Skill>,
    /// Running cost estimate for the current session (USD cents).
    pub cost_tracker: CostTracker,
    /// Why the active goal paused: denied approval, blocker, or stall.
    pub goal_pause_reason: Option<String>,
    /// Consecutive goal turns with no tool action or progress update.
    pub goal_stall: u32,
    /// Whether the most recent turn performed a tool action or goal update.
    pub turn_had_action: bool,
}

/// Tracks estimated token usage and cost for a session.
#[derive(Clone, Debug, Default)]
pub struct CostTracker {
    pub input_tokens: usize,
    pub output_tokens: usize,
    pub request_count: usize,
}

impl CostTracker {
    pub fn add(&mut self, input: usize, output: usize) {
        self.input_tokens += input;
        self.output_tokens += output;
        self.request_count += 1;
    }

    pub fn estimated_cost_usd(&self) -> f64 {
        // Rough blended rate: $3/M input, $15/M output (weighted average)
        let input_cost = self.input_tokens as f64 * 3.0 / 1_000_000.0;
        let output_cost = self.output_tokens as f64 * 15.0 / 1_000_000.0;
        input_cost + output_cost
    }
}

pub struct RuntimeOptions {
    pub endpoint_override: Option<String>,
    pub model_override: Option<String>,
    pub mode: Mode,
    pub effort: Effort,
    pub apply: bool,
    pub dry_run: bool,
    pub sandbox: Sandbox,
    pub tool_use: bool,
}

impl Runtime {
    pub fn new(config: AppConfig, store: ConfigStore, options: RuntimeOptions) -> Result<Self> {
        let mut session = Session::new("interactive");
        session.workspace_root = Some(options.sandbox.project_root().to_path_buf());
        Ok(Self {
            config,
            store,
            endpoint_override: options.endpoint_override,
            model_override: options.model_override,
            mode: options.mode,
            effort: options.effort,
            apply: options.apply,
            dry_run: options.dry_run,
            tool_use: options.tool_use,
            sandbox: options.sandbox,
            session,
            last_apply_outcomes: Vec::new(),
            active_skill: None,
            cost_tracker: CostTracker::default(),
            goal_pause_reason: None,
            goal_stall: 0,
            turn_had_action: false,
        })
    }

    pub async fn run_prompt(&mut self, prompt: &str) -> Result<()> {
        // Initialize theme from config
        ui::set_theme(ui::Theme::parse(&self.config.ui.theme));
        self.goal_pause_reason = None;
        // Mark that a prompt is running so Ctrl+C interrupts instead of quitting.
        crate::interactive::set_prompt_running(true);
        let result = self.run_prompt_inner(prompt).await;
        crate::interactive::set_prompt_running(false);
        result
    }

    async fn run_prompt_inner(&mut self, prompt: &str) -> Result<()> {
        // Manual-approve mode skips implicit repository scans and content
        // reads; a single explicit approval covers any requested context
        // bundle before its contents are read and sent.
        let prompt_input = if self.mode == Mode::RequestPermission {
            crate::context::build_prompt_input_with_approval(
                prompt,
                self.sandbox.project_root(),
                &crate::permissions::confirm,
            )
        } else {
            let scan_project = has_project_marker(self.sandbox.project_root());
            build_prompt_input_with_scan(prompt, self.sandbox.project_root(), scan_project)
        };
        let optimizer = PromptOptimizer;
        let optimized = optimizer.optimize(&prompt_input.text);
        let (endpoint_name, endpoint) = self.resolve_endpoint()?;
        self.compact_if_needed(&optimized).await?;
        if self.mode == Mode::Counsel {
            return self
                .run_counsel_prompt(
                    prompt,
                    &optimized,
                    &prompt_input.context,
                    &endpoint_name,
                    endpoint,
                )
                .await;
        }

        let model =
            self.resolve_model(&endpoint_name, &endpoint, optimized.report.estimated_tokens)?;
        let model = crate::providers::normalize_model_for_endpoint(&endpoint, &model);

        // Tool mode: run the tool loop. Also the default so prompts can
        // create/edit/run; chat-only paths are the explicit alternative.
        if self.tool_use {
            let history = self.session_history_messages();
            let mode_label = if self.session.goal.as_ref().is_some_and(|g| g.is_active()) {
                "tool-use goal"
            } else {
                "tool-use"
            };
            print_prompt_preview(
                &endpoint_name,
                &model,
                mode_label,
                optimized.report.estimated_tokens,
                optimized
                    .report
                    .original_chars
                    .saturating_sub(optimized.report.optimized_chars),
                prompt_input.context.included_items(),
            );

            let skill_prompt = self.active_skill.as_ref().map(|s| s.prompt.clone());
            self.session.push("user", prompt);
            self.save_session()?;
            let assistant_text = crate::tools::run_tool_loop(
                &optimized.text,
                &self.sandbox,
                self.sandbox.project_root(),
                &endpoint,
                &model,
                crate::tools::ToolLoopPromptContext {
                    history,
                    skill_prompt,
                    effort: self.effort,
                    goal: self.goal_prompt_state(),
                    session_id: Some(self.session.id.clone()),
                },
                &mut LoopHost {
                    budget: self.request_budget(&endpoint_name, &model)?,
                    goal_running: self.session.goal.as_ref().is_some_and(|g| g.is_active()),
                    store: &self.store,
                    session: &mut self.session,
                    pause_reason: &mut self.goal_pause_reason,
                    had_action: &mut self.turn_had_action,
                    dry_run: self.dry_run,
                },
            )
            .await;
            let assistant_text = match assistant_text {
                Ok(text) => text,
                Err(error) => {
                    // Failed requests preserve saved state; the transcript
                    // already captured the user turn and any tool results.
                    return Err(error);
                }
            };
            self.track_cost(optimized.report.estimated_tokens, &assistant_text);
            ui::print_markdown(&assistant_text);
            println!();

            self.session.push("assistant", assistant_text.clone());
            self.save_session()?;
            return Ok(());
        }

        let mode_label = if self.apply { "apply" } else { "auto" };
        print_prompt_preview(
            &endpoint_name,
            &model,
            mode_label,
            optimized.report.estimated_tokens,
            optimized
                .report
                .original_chars
                .saturating_sub(optimized.report.optimized_chars),
            prompt_input.context.included_items(),
        );

        let mut messages = Vec::new();
        // Always inject a base system prompt so the model matches the user's
        // language and stays concise.
        messages.push(ChatMessage {
            role: "system".to_string(),
            content: system_prompt(self.effort),
        });
        if self.apply {
            messages.push(ChatMessage {
                role: "system".to_string(),
                content: crate::apply::APPLY_SYSTEM_INSTRUCTION.to_string(),
            });
        }
        // Inject the active skill's prompt as a system message so the model
        // follows reusable instructions for this session.
        if let Some(skill) = &self.active_skill {
            messages.push(ChatMessage {
                role: "system".to_string(),
                content: skill.prompt.clone(),
            });
        }
        // Inject prior session turns so multi-turn conversation context is
        // preserved. The number of turns is bounded by config.routing.history_turns.
        for msg in self.session_history_messages() {
            messages.push(msg);
        }
        messages.push(ChatMessage {
            role: "user".to_string(),
            content: optimized.text.clone(),
        });
        let request = ChatRequest {
            model,
            messages,
            max_tokens: Some(4096),
            session_id: Some(self.session.id.clone()),
        };
        validate_chat_request(&request)?;

        self.session.push("user", prompt);
        self.save_session()?;
        if !self.count_provider_turn()? {
            return Ok(());
        }
        let assistant_text = self.generate(&endpoint, request).await?;
        self.track_cost(optimized.report.estimated_tokens, &assistant_text);
        ui::print_markdown(&assistant_text);
        println!();

        self.apply_files(&assistant_text);

        self.session.push("assistant", assistant_text);
        self.save_session()?;
        Ok(())
    }

    fn count_provider_turn(&mut self) -> Result<bool> {
        use crate::tools::ToolHost;
        LoopHost {
            budget: self.config.routing.input_token_budget,
            goal_running: self.session.goal.as_ref().is_some_and(|g| g.is_active()),
            store: &self.store,
            session: &mut self.session,
            pause_reason: &mut self.goal_pause_reason,
            had_action: &mut self.turn_had_action,
            dry_run: self.dry_run,
        }
        .before_request()
    }

    fn track_cost(&mut self, input_tokens: usize, output_text: &str) {
        self.cost_tracker
            .add(input_tokens, estimate_output_tokens(output_text));
    }

    pub fn save_session(&self) -> Result<()> {
        SessionStore::new(&self.store).save(&self.session)?;
        Ok(())
    }

    /// Estimate the full request size (optimized prompt + history) and run
    /// compaction through the provider when over budget. Compaction failure
    /// returns a clear error and preserves the session; requests are never
    /// sent oversized.
    async fn compact_if_needed(
        &mut self,
        optimized: &crate::optimizer::OptimizedPrompt,
    ) -> Result<()> {
        let (name, endpoint) = self.resolve_endpoint()?;
        let model = self.resolve_model(&name, &endpoint, optimized.report.estimated_tokens)?;
        let budget = self.request_budget(&name, &model)?;
        let instructions = crate::tools::tool_use_system_instruction(
            self.effort,
            self.goal_prompt_state().as_ref(),
        );
        let fixed = crate::optimizer::estimate_tokens(&instructions)
            + self
                .active_skill
                .as_ref()
                .map_or(0, |s| crate::optimizer::estimate_tokens(&s.prompt))
            + optimized.report.estimated_tokens;
        let estimate = |runtime: &Self| {
            fixed + crate::context::messages_tokens(&runtime.session_history_messages())
        };
        if crate::core::context_should_compact(estimate(self), budget) {
            self.compact_session().await?;
        }
        if estimate(self) > budget {
            anyhow::bail!("essential current context exceeds the {budget}-token input budget; session preserved; narrow the request or increase routing.input_token_budget");
        }
        Ok(())
    }

    /// Run a chat request, showing a live preview of streamed tokens while the
    /// model generates. Returns the full assistant text.
    pub async fn generate(
        &self,
        endpoint: &EndpointConfig,
        request: ChatRequest,
    ) -> Result<String> {
        let preview_buf = ui::preview_start();
        let mut assistant_text = String::new();
        let adapter = adapter_for(endpoint.provider.clone());
        let result = crate::providers::stream_chat_with_retry(
            adapter.as_ref(),
            endpoint,
            request,
            &mut |delta| {
                if crate::interactive::was_interrupted() {
                    return;
                }
                assistant_text.push_str(&delta);
                ui::preview_update(&preview_buf, &delta);
            },
        )
        .await;
        ui::preview_stop();
        if crate::interactive::was_interrupted() {
            eprintln!(
                "\n{}",
                "(interrupted — partial response shown above)".dimmed()
            );
            return Ok(assistant_text);
        }
        result?;
        Ok(assistant_text)
    }

    async fn run_counsel_prompt(
        &mut self,
        prompt: &str,
        optimized: &crate::optimizer::OptimizedPrompt,
        context: &PromptContextReport,
        endpoint_name: &str,
        endpoint: EndpointConfig,
    ) -> Result<()> {
        let cache = ModelCache::load(&self.store)?;
        let plan = self.resolve_counsel_plan(endpoint_name, &endpoint, &cache, &optimized.text)?;
        let worker_model = self
            .resolve_model_override()
            .unwrap_or_else(|| plan.worker_model.clone());
        let worker_model = crate::providers::normalize_model_for_endpoint(&endpoint, &worker_model);

        print_prompt_preview(
            endpoint_name,
            &worker_model,
            "counsel",
            optimized.report.estimated_tokens,
            optimized
                .report
                .original_chars
                .saturating_sub(optimized.report.optimized_chars),
            context.included_items(),
        );
        println!(
            "{} evaluator={} task={}",
            "->".cyan(),
            plan.evaluator_model,
            plan.task.as_str()
        );

        self.session.push("user", prompt);
        self.save_session()?;

        let adapter = adapter_for(endpoint.provider.clone());
        let evaluation_prompt =
            build_evaluation_prompt(&optimized.text, optimized.report.estimated_tokens);
        if !self.count_provider_turn()? {
            return Ok(());
        }
        let evaluation = collect_chat(
            adapter.as_ref(),
            &endpoint,
            ChatRequest {
                model: plan.evaluator_model.clone(),
                messages: vec![ChatMessage {
                    role: "user".to_string(),
                    content: evaluation_prompt,
                }],
                max_tokens: Some(512),
                session_id: Some(self.session.id.clone()),
            },
        )
        .await?;

        self.session
            .push("assistant", format!("Counsel evaluation:\n{evaluation}"));

        // Counsel workers run through the same tool/context/goal path as
        // other action modes, even when evaluator and worker share a model.
        let worker_prompt = build_worker_prompt(&optimized.text, &evaluation, plan.task);
        let assistant_text = self
            .run_worker_with_tools(&worker_prompt, &endpoint, &worker_model)
            .await?;
        ui::print_markdown(&assistant_text);
        println!();

        self.apply_files(&assistant_text);

        self.session.push(
            "assistant",
            format!("Counsel evaluation:\n{evaluation}\n\nResponse:\n{assistant_text}"),
        );
        self.save_session()?;
        Ok(())
    }

    /// Run a counsel worker through the shared tool loop so it can read,
    /// write, and execute like other action modes instead of returning a
    /// text-only response.
    async fn run_worker_with_tools(
        &mut self,
        worker_prompt: &str,
        endpoint: &EndpointConfig,
        worker_model: &str,
    ) -> Result<String> {
        if !self.tool_use {
            // Chat-only counsel workers still count as provider turns toward
            // the goal budget and see the same bounded session context.
            if !self.count_provider_turn()? {
                return Ok("Execution paused; session checkpoint retained.".to_string());
            }
            let mut messages = self.session_history_messages();
            messages.push(ChatMessage {
                role: "user".to_string(),
                content: worker_prompt.to_string(),
            });
            let request = ChatRequest {
                model: worker_model.to_string(),
                messages,
                max_tokens: Some(4096),
                session_id: Some(self.session.id.clone()),
            };
            validate_chat_request(&request)?;
            return self.generate(endpoint, request).await;
        }
        let history = self.session_history_messages();
        self.session.push("user", worker_prompt);
        self.save_session()?;
        let skill_prompt = self.active_skill.as_ref().map(|s| s.prompt.clone());
        let goal = self.goal_prompt_state();
        let session_id = self.session.id.clone();
        crate::tools::run_tool_loop(
            worker_prompt,
            &self.sandbox,
            self.sandbox.project_root(),
            endpoint,
            worker_model,
            crate::tools::ToolLoopPromptContext {
                history,
                skill_prompt,
                effort: self.effort,
                goal,
                session_id: Some(session_id),
            },
            &mut LoopHost {
                budget: self.request_budget(&endpoint.name, worker_model)?,
                goal_running: self.session.goal.as_ref().is_some_and(|g| g.is_active()),
                store: &self.store,
                session: &mut self.session,
                pause_reason: &mut self.goal_pause_reason,
                had_action: &mut self.turn_had_action,
                dry_run: self.dry_run,
            },
        )
        .await
    }

    /// Process apply mode for the given assistant text: extract `path=` blocks,
    /// preview, and optionally write them through the sandbox. Updates
    /// `last_apply_outcomes`. Shared by normal and counsel prompt flows.
    fn apply_files(&mut self, assistant_text: &str) {
        if !self.apply {
            return;
        }
        let files = crate::apply::extract_files(assistant_text);
        if !files.is_empty() {
            let previews =
                crate::apply::preview(&self.sandbox, &files, self.sandbox.project_root());
            crate::apply::print_preview(&previews);
            if self.dry_run {
                self.last_apply_outcomes.clear();
                println!("{}", "dry run: no files written".yellow());
            } else {
                let outcomes =
                    crate::apply::apply(&self.sandbox, &files, self.sandbox.project_root());
                crate::apply::print_checklist(&outcomes);
                self.last_apply_outcomes = outcomes;
            }
        } else {
            self.last_apply_outcomes.clear();
            println!(
                "{}",
                "no path=-annotated code blocks found to apply".dimmed()
            );
        }
    }

    /// Build a bounded list of prior session messages to inject into the next
    /// prompt. Starts after the compaction summary index, keeps system
    /// summaries visible so compaction stays continuous, and maps stored
    /// tool results onto user-role messages for the provider.
    fn session_history_messages(&self) -> Vec<ChatMessage> {
        crate::context::session_history(&self.session)
    }

    pub async fn compact_session(&mut self) -> Result<String> {
        let (name, endpoint) = self.resolve_endpoint()?;
        let model = self.resolve_model(&name, &endpoint, 0)?;
        let budget = self.request_budget(&name, &model)?;
        crate::context::compact_session(&mut self.session, &self.store, &endpoint, &model, budget)
            .await
    }

    fn request_budget(&self, endpoint: &str, model: &str) -> Result<usize> {
        let cache = ModelCache::load(&self.store)?;
        let window = cache
            .available_for(endpoint)
            .find(|m| m.id == model)
            .and_then(|m| m.context_window)
            .unwrap_or(0);
        Ok(self
            .config
            .routing
            .input_token_budget
            .max(1)
            .min(crate::core::context_budget_for_window(window)))
    }

    /// Goal status text for `/goal` with no model call.
    pub fn goal_status_text(&self) -> String {
        let Some(goal) = self.session.goal.as_ref() else {
            return "no goal; start one with /goal <objective>".to_string();
        };
        let remaining = goal.max_steps.saturating_sub(goal.steps_used);
        format!(
            "goal: {}\nstatus: {}\nprogress: {}\nevidence: {}\nsteps: {}/{} used, {} remaining\nresume: /goal resume  pause: /goal pause  cancel: /goal cancel",
            goal.objective,
            goal.status,
            if goal.progress.is_empty() { "(none yet)" } else { &goal.progress },
            if goal.evidence.is_empty() { "(none yet)" } else { &goal.evidence },
            goal.steps_used,
            goal.max_steps,
            remaining
        )
    }

    /// `/goal <objective>`: persist a new goal and start its run. Replacing
    /// an active/paused/blocked goal is rejected; cancel first.
    pub async fn start_goal(&mut self, objective: &str) -> Result<String> {
        if objective.trim().is_empty() {
            return Err(anyhow!("provide an objective: /goal <objective>"));
        }
        if let Some(goal) = self.session.goal.as_ref() {
            if !goal.is_replaceable() {
                return Err(anyhow!(
                    "a {} goal already exists; cancel it first with /goal cancel",
                    goal.status
                ));
            }
        }
        self.session.goal = Some(crate::sessions::GoalState::new(objective.trim()));
        self.save_session()?;
        println!(
            "{}",
            "goal started (Ctrl+C pauses; /goal pause also works)".cyan()
        );
        self.run_goal_loop().await
    }

    /// `/goal resume`: grant another bounded batch of steps and continue.
    pub async fn resume_goal(&mut self) -> Result<String> {
        let budget = {
            let Some(goal) = self.session.goal.as_mut() else {
                return Err(anyhow!(
                    "no goal to resume; start one with /goal <objective>"
                ));
            };
            if goal.is_active() {
                return Ok("goal is already running".to_string());
            }
            if matches!(
                goal.status_code(),
                crate::core::GOAL_COMPLETED | crate::core::GOAL_CANCELLED
            ) {
                return Err(anyhow!(
                    "goal is {}; start a new one with /goal <objective>",
                    goal.status
                ));
            }
            goal.set_status(crate::core::GOAL_ACTIVE);
            goal.steps_used = 0;
            goal.max_steps = crate::core::goal_default_max_steps();
            goal.max_steps
        };
        self.save_session()?;
        println!(
            "{}",
            format!("goal resumed with a fresh budget of up to {budget} steps").cyan()
        );
        self.run_goal_loop().await
    }

    /// `/goal pause`: mark an active goal paused; no model call.
    pub fn pause_goal(&mut self) -> Result<String> {
        let message = {
            let Some(goal) = self.session.goal.as_mut() else {
                return Err(anyhow!("no goal to pause"));
            };
            let next =
                crate::core::goal_transition(goal.status_code(), crate::core::GOAL_EVENT_PAUSE);
            goal.set_status(next);
            format!("goal status: {}", goal.status)
        };
        self.save_session()?;
        Ok(message)
    }

    /// `/goal cancel`: mark the goal cancelled; transcript retained; no model call.
    pub fn cancel_goal(&mut self) -> Result<String> {
        let message = {
            let Some(goal) = self.session.goal.as_mut() else {
                return Err(anyhow!("no goal to cancel"));
            };
            let previous = goal.status_code();
            let next = crate::core::goal_transition(previous, crate::core::GOAL_EVENT_CANCEL);
            if next == previous {
                return Err(anyhow!("goal is {}; nothing to cancel", goal.status));
            }
            goal.set_status(next);
            "goal cancelled; transcript retained".to_string()
        };
        self.save_session()?;
        Ok(message)
    }

    /// Transition the active goal through the C state machine and save.
    fn transition_goal(&mut self, event: i32) -> Result<()> {
        if let Some(goal) = self.session.goal.as_mut() {
            let next = crate::core::goal_transition(goal.status_code(), event);
            goal.set_status(next);
        }
        self.save_session()
    }

    /// The goal agent loop. Injects objective and progress into each
    /// request, counts every provider turn toward the step cap, pauses on
    /// denial/cancellation/blockers/stall, and never marks a goal complete
    /// from a plain prose response.
    async fn run_goal_loop(&mut self) -> Result<String> {
        self.goal_stall = 0;
        self.goal_pause_reason = None;
        loop {
            let Some(goal) = self.session.goal.as_ref() else {
                break;
            };
            let status = goal.status_code();
            if crate::core::agent_next(
                status,
                goal.steps_used,
                goal.max_steps,
                false,
                false,
                self.goal_stall,
            ) != crate::core::AgentAction::Request
            {
                if status == crate::core::GOAL_ACTIVE {
                    // The step cap pauses; it never marks complete.
                    self.transition_goal(crate::core::GOAL_EVENT_STEP_LIMIT)?;
                    let used = self
                        .session
                        .goal
                        .as_ref()
                        .map(|goal| goal.steps_used)
                        .unwrap_or_default();
                    let paused = format!(
                        "goal paused: step budget exhausted ({used} steps used); /goal resume grants another batch"
                    );
                    println!("{}", paused.yellow());
                    return Ok(paused);
                }
                break;
            }
            self.turn_had_action = false;
            let continuation = self.goal_continuation_prompt();
            // Interrupting a goal turn pauses instead of losing state; the
            // run_prompt path already handles Ctrl+C interruption of the
            // active provider request.
            let turn_result = self.run_prompt(&continuation).await;
            if let Err(error) = turn_result {
                // Provider failure preserves state and returns control
                // instead of terminating the shell.
                self.transition_goal(crate::core::GOAL_EVENT_PAUSE)?;
                let stopped = self
                    .session
                    .goal
                    .as_ref()
                    .map(|goal| format!("goal run stopped: {error} (status: {})", goal.status))
                    .unwrap_or_else(|| format!("goal run stopped: {error}"));
                return Ok(stopped);
            }
            if crate::interactive::was_interrupted() {
                let paused =
                    "goal paused (interrupted with Ctrl+C); /goal resume continues".to_string();
                self.transition_goal(crate::core::GOAL_EVENT_PAUSE)?;
                println!("{}", paused.yellow());
                return Ok(paused);
            }
            if let Some(reason) = self.goal_pause_reason.take() {
                // Denied approval pauses the goal and returns control.
                self.transition_goal(crate::core::GOAL_EVENT_PAUSE)?;
                let paused = format!("goal paused: {reason}");
                println!("{}", paused.yellow());
                return Ok(paused);
            }
            if self
                .session
                .goal
                .as_ref()
                .is_some_and(|goal| !goal.is_active())
            {
                // Completed/blocked/cancelled through goal_update; stop.
                break;
            }
            if self.turn_had_action {
                self.goal_stall = 0;
            } else {
                self.goal_stall += 1;
                if crate::core::agent_next(
                    crate::core::GOAL_ACTIVE,
                    0,
                    u32::MAX,
                    false,
                    false,
                    self.goal_stall,
                ) == crate::core::AgentAction::Pause
                {
                    let paused = "goal paused: two consecutive turns with no tool action or progress update; give a concrete next step with /goal resume".to_string();
                    self.transition_goal(crate::core::GOAL_EVENT_PAUSE)?;
                    println!("{}", paused.yellow());
                    return Ok(paused);
                }
            }
        }
        let goal = self
            .session
            .goal
            .as_ref()
            .map(|goal| goal.status.clone())
            .unwrap_or_default();
        Ok(format!("goal loop stopped (status: {goal})"))
    }

    fn goal_continuation_prompt(&self) -> String {
        let Some(goal) = self.session.goal.as_ref() else {
            return String::new();
        };
        format!(
            "Continue the active goal.\nObjective: {}\nStatus: {}\nProgress so far: {}\nSteps used: {}/{}\n\nTake the next concrete step toward the objective. Report state with goal_update: progress updates while working, status blocked with what you need, or completed with evidence from this session's tool results. A plain prose reply does not complete the goal.",
            goal.objective,
            goal.status,
            if goal.progress.is_empty() { "(none yet)" } else { &goal.progress },
            goal.steps_used,
            goal.max_steps
        )
    }

    /// Goal state mapped into the tool-loop prompt context.
    fn goal_prompt_state(&self) -> Option<crate::tools::GoalPromptState> {
        let goal = self.session.goal.as_ref()?;
        Some(crate::tools::GoalPromptState {
            objective: goal.objective.clone(),
            status: goal.status.clone(),
            progress: goal.progress.clone(),
            steps_used: goal.steps_used,
            max_steps: goal.max_steps,
        })
    }

    pub fn print_models(&self) -> Result<()> {
        let cache = ModelCache::load(&self.store)?;
        for (endpoint, cached) in cache.endpoints {
            println!("{}", endpoint.bold());
            for model in cached.models {
                println!("  {:?} {}", model.status, model.info.id);
            }
        }
        Ok(())
    }

    pub fn print_endpoints(&self) {
        print_endpoints(&self.config);
    }

    pub fn print_skills(&self) -> Result<()> {
        let skills = SkillStore::new(&self.store, project_root()).list()?;
        for skill in skills {
            println!("{} - {}", skill.name.bold(), skill.description);
        }
        Ok(())
    }

    pub(crate) fn resolve_endpoint(&self) -> Result<(String, EndpointConfig)> {
        let endpoint_name = self
            .endpoint_override
            .clone()
            .or_else(|| self.config.primary_endpoint.clone())
            .ok_or(CntxError::MissingPrimaryEndpoint)?;
        let mut endpoint = self
            .config
            .endpoints
            .get(&endpoint_name)
            .cloned()
            .ok_or_else(|| CntxError::EndpointNotFound(endpoint_name.clone()))?;
        // Fall back to the runtime secrets store when the endpoint has no key
        // of its own, so `cntx api-key --add anthropic` is enough to run.
        if endpoint.resolved_api_key().is_none() {
            if let Some(key) = api_keys::resolve_for_provider(&self.store, &endpoint) {
                endpoint.api_key = Some(key);
            }
        }
        Ok((endpoint_name, endpoint))
    }

    pub(crate) fn resolve_model(
        &self,
        endpoint_name: &str,
        endpoint: &EndpointConfig,
        estimated_tokens: usize,
    ) -> Result<String> {
        if let Some(model_or_alias) = &self.model_override {
            if let Some(alias) = self.config.aliases.get(model_or_alias) {
                return Ok(alias.model.clone());
            }
            return Ok(model_or_alias.clone());
        }

        let cache = ModelCache::load(&self.store)?;
        let models = cache.available_for(endpoint_name);
        if let Some(decision) =
            ModelRouter::new(&self.config.routing).route(endpoint, models, estimated_tokens)
        {
            return Ok(decision.model);
        }

        // Persisted default model (`cntx model default <name>`) is tried
        // before the endpoint's own default so users can pick once globally.
        if let Some(default) = self.config.default_model.as_ref() {
            return resolve_alias_or_model(&self.config, default);
        }

        endpoint.default_model.clone().ok_or_else(|| {
            CntxError::ModelNotFound("auto".to_string(), endpoint_name.to_string()).into()
        })
    }

    fn resolve_model_override(&self) -> Option<String> {
        self.model_override.as_ref().map(|model_or_alias| {
            self.config
                .aliases
                .get(model_or_alias)
                .map(|alias| alias.model.clone())
                .unwrap_or_else(|| model_or_alias.clone())
        })
    }

    fn resolve_counsel_plan(
        &self,
        endpoint_name: &str,
        endpoint: &EndpointConfig,
        cache: &ModelCache,
        optimized_prompt: &str,
    ) -> Result<CounselPlan> {
        plan_counsel(
            endpoint,
            cache.available_for(endpoint_name),
            optimized_prompt,
        )
        .or_else(|| {
            endpoint.default_model.as_ref().map(|model| CounselPlan {
                evaluator_model: model.clone(),
                worker_model: model.clone(),
                task: crate::counsel::classify_counsel_task(optimized_prompt),
            })
        })
        .ok_or_else(|| {
            CntxError::ModelNotFound("counsel".to_string(), endpoint_name.to_string()).into()
        })
    }
}

/// Completion evidence must correspond to this session's recorded tool
/// results/checks, or explicitly state that no executable check applies.
/// Comparing against `tool_result` messages only keeps the model from
/// satisfying the check by quoting its own tool-call text back.
fn completion_evidence_supported(evidence: &str, messages: &[SessionMessage]) -> bool {
    let lowered = evidence.to_lowercase();
    if [
        "no executable check",
        "no check",
        "cannot verify",
        "can't verify",
        "unable to verify",
        "not verifiable",
        "manual review",
    ]
    .iter()
    .any(|phrase| lowered.contains(phrase))
    {
        return true;
    }
    let tokens: std::collections::HashSet<String> = evidence
        .split(|ch: char| !ch.is_alphanumeric() && ch != '_' && ch != '.' && ch != '/')
        .filter(|token| token.len() >= 4)
        .map(|token| token.to_lowercase())
        .collect();
    if tokens.is_empty() {
        return false;
    }
    messages
        .iter()
        .filter(|message| {
            message.role == "tool_result"
                && !message.content.contains("Tool result for 'goal_update'")
        })
        .any(|message| {
            let content = message.content.to_lowercase();
            tokens.iter().any(|token| content.contains(token.as_str()))
        })
}

/// Host adapter between the tool loop and the runtime. Records the
/// transcript (with atomic saves), applies dry-run, forwards approval to the
/// human, and validates goal_update actions against the C core.
struct LoopHost<'a> {
    store: &'a ConfigStore,
    session: &'a mut Session,
    pause_reason: &'a mut Option<String>,
    had_action: &'a mut bool,
    dry_run: bool,
    goal_running: bool,
    budget: usize,
}

#[async_trait::async_trait]
impl crate::tools::ToolHost for LoopHost<'_> {
    fn dry_run(&self) -> bool {
        self.dry_run
    }

    fn approve(&mut self, action: &str) -> bool {
        let approved = crate::permissions::confirm(action);
        if !approved {
            *self.pause_reason = Some("user denied approval".to_string());
        }
        approved
    }

    /// Validate a `goal_update` action from the model. Only accepted while a
    /// goal is running; completion requires evidence from this session's
    /// tool results or a reason why no executable check applies.
    fn goal_update(&mut self, arguments: &serde_json::Value) -> Result<String, String> {
        let Some(goal) = self.session.goal.as_mut() else {
            return Err("no active goal; goal_update is only valid during a goal run".to_string());
        };
        if !goal.is_active() {
            return Err(format!(
                "goal is {}, not active; goal_update is only valid while the goal runs",
                goal.status
            ));
        }
        let status = arguments
            .get("status")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let requested = crate::core::goal_parse_status(status).ok_or_else(|| {
            format!("invalid goal status '{status}'; use active, blocked, or completed")
        })?;
        let event =
            match requested {
                crate::core::GOAL_ACTIVE => None,
                crate::core::GOAL_BLOCKED => Some(crate::core::GOAL_EVENT_BLOCK),
                crate::core::GOAL_COMPLETED => Some(crate::core::GOAL_EVENT_COMPLETE),
                _ => return Err(
                    "paused/cancelled/none are user controls; use active, blocked, or completed"
                        .to_string(),
                ),
            };
        let progress = arguments
            .get("progress")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let evidence = arguments
            .get("evidence")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        if requested == crate::core::GOAL_COMPLETED && evidence.trim().is_empty() {
            return Err(
                "completing a goal requires nonempty evidence: reference actual tool results or checks from this session, or explain why no executable check applies"
                    .to_string(),
            );
        }
        if requested == crate::core::GOAL_COMPLETED
            && !completion_evidence_supported(evidence, &self.session.messages)
        {
            return Err(
                "completion evidence must reference tool results or checks recorded in this session, or state why no executable check applies"
                    .to_string(),
            );
        }
        goal.progress = progress.to_string();
        if !evidence.is_empty() {
            goal.evidence = evidence.to_string();
        }
        let (saved_status, saved_progress, saved_evidence, saved_steps, saved_max) = (
            goal.status.clone(),
            goal.progress.clone(),
            goal.evidence.clone(),
            goal.steps_used,
            goal.max_steps,
        );
        if let Some(event) = event {
            let next = crate::core::goal_transition(goal.status_code(), event);
            goal.set_status(next);
            SessionStore::new(self.store)
                .save(self.session)
                .map_err(|error| error.to_string())?;
            match requested {
                crate::core::GOAL_COMPLETED => {
                    // The reported outcome is the model's claim backed by
                    // the evidence string; not an independent proof.
                    return Ok(format!(
                        "goal marked completed. Evidence recorded: {saved_evidence}"
                    ));
                }
                crate::core::GOAL_BLOCKED => {
                    return Ok(format!(
                        "goal marked blocked: {saved_progress}. The user will decide how to proceed."
                    ));
                }
                _ => {}
            }
        } else {
            SessionStore::new(self.store)
                .save(self.session)
                .map_err(|error| error.to_string())?;
        }
        *self.had_action = true;
        Ok(format!(
            "goal status: {saved_status} (steps {saved_steps}/{saved_max})"
        ))
    }

    async fn prepare_request(
        &mut self,
        endpoint: &EndpointConfig,
        request: &mut ChatRequest,
    ) -> Result<()> {
        if crate::context::messages_tokens(&request.messages) > self.budget {
            let old_history = crate::context::session_history(self.session).len();
            let prefix = request
                .messages
                .len()
                .checked_sub(old_history)
                .ok_or_else(|| anyhow!("context/transcript mismatch; session preserved"))?;
            let old_start = self.session.context_start_index;
            let old_summary = usize::from(self.session.summary.is_some());
            crate::context::compact_session(
                self.session,
                self.store,
                endpoint,
                &request.model,
                self.budget,
            )
            .await?;
            let covered = self.session.context_start_index - old_start;
            request
                .messages
                .drain(prefix..prefix + old_summary + covered);
            if let Some(summary) = self.session.summary.as_ref() {
                request.messages.insert(
                    prefix,
                    ChatMessage {
                        role: "system".into(),
                        content: format!("Previous conversation summary:\n{summary}"),
                    },
                );
            }
        }
        if crate::context::messages_tokens(&request.messages) > self.budget {
            anyhow::bail!(
                "essential current context exceeds the {}-token input budget; session preserved",
                self.budget
            );
        }
        Ok(())
    }

    fn before_request(&mut self) -> Result<bool> {
        if self.stopped() {
            return Ok(false);
        }
        if let Some(goal) = self.session.goal.as_mut().filter(|g| g.is_active()) {
            if crate::core::agent_next(
                goal.status_code(),
                goal.steps_used,
                goal.max_steps,
                false,
                false,
                0,
            ) != crate::core::AgentAction::Request
            {
                goal.set_status(crate::core::goal_transition(
                    goal.status_code(),
                    crate::core::GOAL_EVENT_STEP_LIMIT,
                ));
                *self.pause_reason =
                    Some("step budget exhausted; /goal resume grants another batch".into());
                SessionStore::new(self.store).save(self.session)?;
                return Ok(false);
            }
            goal.steps_used += 1;
            SessionStore::new(self.store).save(self.session)?;
        }
        Ok(true)
    }

    fn stopped(&self) -> bool {
        self.pause_reason.is_some()
            || (self.goal_running && self.session.goal.as_ref().is_some_and(|g| !g.is_active()))
    }

    fn record_transcript(&mut self, role: &str, content: &str) -> Result<()> {
        self.session.push(role, content);
        if role == "tool_result" {
            *self.had_action = true;
        }
        SessionStore::new(self.store).save(self.session)
    }
}

async fn collect_chat(
    adapter: &dyn crate::providers::ProviderAdapter,
    endpoint: &EndpointConfig,
    request: ChatRequest,
) -> Result<String> {
    validate_chat_request(&request)?;
    let mut text = String::new();
    adapter
        .stream_chat(endpoint, request, &mut |delta| text.push_str(&delta))
        .await?;
    Ok(text)
}

fn print_prompt_preview(
    endpoint_name: &str,
    model: &str,
    mode_label: &str,
    estimated_tokens: usize,
    saved_chars: usize,
    context_items: usize,
) {
    let cost = rough_cost_usd(model, estimated_tokens, 4096)
        .map(|value| format!(" cost~=${value:.4}"))
        .unwrap_or_default();
    let context = if context_items > 0 {
        format!(" context={context_items}")
    } else {
        String::new()
    };
    println!(
        "{} endpoint={} model={} mode={} tokens={} saved={} chars{}{}",
        "->".cyan(),
        endpoint_name,
        model,
        mode_label,
        estimated_tokens,
        saved_chars,
        context,
        cost
    );
}

struct DocPage {
    title: &'static str,
    body: &'static str,
}

const DOC_PAGES: &[DocPage] = &[
    DocPage {
        title: "README",
        body: include_str!("../README.md"),
    },
    DocPage {
        title: "Explanation",
        body: include_str!("../EXPLAIN.md"),
    },
    DocPage {
        title: "Commands",
        body: include_str!("../docs/commands.md"),
    },
    DocPage {
        title: "Goals",
        body: include_str!("../docs/goals.md"),
    },
    DocPage {
        title: "OpenCode Go",
        body: include_str!("../docs/opencode-go.md"),
    },
    DocPage {
        title: "Apply Mode",
        body: include_str!("../docs/apply.md"),
    },
    DocPage {
        title: "API Keys",
        body: include_str!("../docs/api-keys.md"),
    },
    DocPage {
        title: "Providers",
        body: include_str!("../docs/providers.md"),
    },
    DocPage {
        title: "Custom Providers",
        body: include_str!("../docs/custom-providers.md"),
    },
    DocPage {
        title: "Sandbox",
        body: include_str!("../docs/sandbox.md"),
    },
    DocPage {
        title: "Sessions",
        body: include_str!("../docs/sessions.md"),
    },
    DocPage {
        title: "MCP",
        body: include_str!("../docs/mcp.md"),
    },
    DocPage {
        title: "Troubleshooting",
        body: include_str!("../docs/troubleshooting.md"),
    },
];

fn handle_docs() -> Result<()> {
    let mut editor = DefaultEditor::new()?;
    loop {
        println!("{}", "Cntx docs".cyan().bold());
        for (index, page) in DOC_PAGES.iter().enumerate() {
            println!("  {}. {}", index + 1, page.title);
        }
        println!("  q. quit");
        let input = editor.readline("docs> ")?;
        let input = input.trim();
        if input.eq_ignore_ascii_case("q") || input.eq_ignore_ascii_case("quit") {
            break;
        }
        let Some(index) = input
            .parse::<usize>()
            .ok()
            .and_then(|value| value.checked_sub(1))
        else {
            println!("choose a number or q");
            continue;
        };
        let Some(page) = DOC_PAGES.get(index) else {
            println!("no doc page numbered {}", index + 1);
            continue;
        };
        println!();
        ui::print_markdown(page.body);
        println!();
        let _ = editor.readline("press Enter to return to docs...");
    }
    Ok(())
}

async fn handle_endpoint(
    args: EndpointArgs,
    store: &ConfigStore,
    config: &mut AppConfig,
) -> Result<()> {
    if args.list {
        print_endpoints(config);
        return Ok(());
    }

    if let Some(path) = args.import.as_ref() {
        let import = load_endpoint_import(path)?;
        for endpoint in import.endpoints {
            config.endpoints.insert(endpoint.name.clone(), endpoint);
        }
        if import.primary_endpoint.is_some() {
            config.primary_endpoint = import.primary_endpoint;
        }
        store.save(config)?;
        println!("imported endpoints from {}", path.display());
        return Ok(());
    }

    if let Some(name) = args.set_primary.as_ref() {
        if !config.endpoints.contains_key(name) {
            return Err(CntxError::EndpointNotFound(name.clone()).into());
        }
        config.primary_endpoint = Some(name.clone());
        store.save(config)?;
        println!("primary endpoint set to {name}");
        return Ok(());
    }

    if let Some(name) = args.remove.as_ref() {
        config.endpoints.remove(name);
        if config.primary_endpoint.as_deref() == Some(name) {
            config.primary_endpoint = config.endpoints.keys().next().cloned();
        }
        store.save(config)?;
        println!("removed endpoint {name}");
        return Ok(());
    }

    if args.new {
        let existing = if let Some(preset_name) = args.from_preset.as_ref() {
            let preset = config
                .custom_providers
                .get(preset_name)
                .cloned()
                .ok_or_else(|| anyhow!("provider preset `{preset_name}` was not found"))?;
            let endpoint_name = args.name.clone().unwrap_or_else(|| preset_name.clone());
            Some(preset.to_endpoint(endpoint_name))
        } else {
            None
        };
        let endpoint = build_endpoint_from_args(&args, existing)?;
        let name = endpoint.name.clone();
        config.endpoints.insert(name.clone(), endpoint);
        if config.primary_endpoint.is_none() {
            config.primary_endpoint = Some(name.clone());
        }
        store.save(config)?;
        println!("created endpoint {name}");
        return Ok(());
    }

    if let Some(name) = args.change.as_ref() {
        let existing = config
            .endpoints
            .get(name)
            .cloned()
            .ok_or_else(|| CntxError::EndpointNotFound(name.clone()))?;
        let endpoint = build_endpoint_from_args(&args, Some(existing))?;
        config.endpoints.insert(name.clone(), endpoint);
        store.save(config)?;
        println!("updated endpoint {name}");
        return Ok(());
    }

    Err(anyhow!(
        "choose an endpoint action: --new, --change, --remove, --list, --set-primary, or --import"
    ))
}

async fn handle_model(
    command: ModelCommand,
    store: &ConfigStore,
    config: &mut AppConfig,
) -> Result<()> {
    match command {
        ModelCommand::Add(args) => {
            if config.aliases.contains_key(&args.name) {
                return Err(CntxError::AliasExists(args.name).into());
            }
            let now = Utc::now();
            config.aliases.insert(
                args.name.clone(),
                ModelAlias {
                    alias: args.name.clone(),
                    model: args.model,
                    endpoint: args.endpoint,
                    created_at: now,
                    updated_at: now,
                },
            );
            store.save(config)?;
            println!("added model alias {}", args.name);
            Ok(())
        }
        ModelCommand::Remove(args) => {
            config.aliases.remove(&args.name);
            store.save(config)?;
            println!("removed model alias {}", args.name);
            Ok(())
        }
        ModelCommand::Default { model, unset } => {
            if unset {
                config.default_model = None;
                store.save(config)?;
                println!("cleared default model");
                return Ok(());
            }
            let model = model.ok_or_else(|| {
                anyhow!(
                    "provide a model or alias, e.g. `cntx model default gpt-5.5`, or use --unset"
                )
            })?;
            config.default_model = Some(model.clone());
            store.save(config)?;
            println!("default model set to {model}");
            Ok(())
        }
        ModelCommand::List => {
            println!("{}", "Aliases".bold());
            for alias in config.aliases.values() {
                println!(
                    "  {} -> {}{}",
                    alias.alias,
                    alias.model,
                    alias
                        .endpoint
                        .as_ref()
                        .map(|endpoint| format!(" ({endpoint})"))
                        .unwrap_or_default()
                );
            }
            if let Some(default) = config.default_model.as_ref() {
                println!("{} default model: {}", "*".cyan(), default);
            }
            let sandbox = Sandbox::new(Mode::Auto, project_root(), Vec::new());
            Runtime::new(
                config.clone(),
                store.clone(),
                RuntimeOptions {
                    endpoint_override: None,
                    model_override: None,
                    mode: Mode::Auto,
                    effort: config.ui.effort,
                    apply: false,
                    dry_run: false,
                    sandbox,
                    tool_use: false,
                },
            )?
            .print_models()
        }
        ModelCommand::Refresh => {
            let report = refresh_with_spinner(config, store).await?;
            ui::print_refresh_report(&report);
            Ok(())
        }
    }
}

fn handle_config(command: ConfigCommand, store: &ConfigStore, config: &AppConfig) -> Result<()> {
    match command {
        ConfigCommand::Init => {
            store.save(config)?;
            println!("initialized config at {}", store.config_path().display());
        }
        ConfigCommand::Path => println!("{}", store.config_path().display()),
        ConfigCommand::Show => println!("{}", serde_yaml::to_string(config)?),
    }
    Ok(())
}

async fn handle_session(command: SessionCommand, store: &ConfigStore) -> Result<()> {
    let sessions = SessionStore::new(store);
    match command {
        SessionCommand::List => {
            for session in sessions.list()? {
                println!(
                    "{} {} {} messages  {}",
                    session.id,
                    session.updated_at,
                    session.messages.len(),
                    session.title
                );
            }
        }
        SessionCommand::Resume { id } => {
            let session = if let Some(id) = id {
                sessions.load(&id)?
            } else {
                // Latest for the current workspace, like `/resume`.
                let current = project_root();
                sessions
                    .list()?
                    .into_iter()
                    .find(|session| {
                        session.workspace_root.as_deref() == Some(current.as_path())
                            || session.workspace_root.is_none()
                    })
                    .ok_or_else(|| anyhow!("no saved sessions in this workspace yet"))?
            };
            // Actual resume: enter the interactive loop with the loaded
            // session instead of printing YAML and exiting.
            let config = store.load()?;
            let sandbox = Sandbox::new(config.ui.mode, project_root(), Vec::new());
            let effort = config.ui.effort;
            let mode = config.ui.mode;
            let mut runtime = Runtime::new(
                config,
                store.clone(),
                RuntimeOptions {
                    endpoint_override: None,
                    model_override: None,
                    mode,
                    effort,
                    apply: false,
                    dry_run: false,
                    sandbox,
                    tool_use: true,
                },
            )?;
            if let Some(session_root) = session.workspace_root.as_ref() {
                if session_root != &project_root() {
                    println!(
                        "{}",
                        format!(
                            "session {} belongs to {}; launch cntx in that directory to resume it",
                            session.id,
                            session_root.display()
                        )
                        .yellow()
                    );
                    return Ok(());
                }
            } else {
                println!(
                    "{}",
                    "note: this session predates workspace tracking; resuming in the current workspace"
                        .yellow()
                );
            }
            runtime.session = session;
            interactive::run(&mut runtime).await?;
        }
        SessionCommand::Export { id, output } => {
            sessions.export(&id, &output)?;
            println!("exported session {id} to {}", output.display());
        }
        SessionCommand::Import { input } => {
            let session = sessions.import(&input)?;
            println!("imported session {}", session.id);
        }
    }
    Ok(())
}

fn handle_skill(command: SkillCommand, store: &ConfigStore) -> Result<()> {
    let skills = SkillStore::new(store, project_root());
    match command {
        SkillCommand::List => {
            for skill in skills.list()? {
                println!("{} - {}", skill.name.bold(), skill.description);
            }
        }
        SkillCommand::New { name, description } => {
            skills.create(&name, &description)?;
            println!("created skill {name}");
        }
        SkillCommand::Show { name } => {
            let skill = skills
                .get(&name)?
                .ok_or_else(|| anyhow!("skill `{name}` was not found"))?;
            println!("{}", serde_yaml::to_string(&skill)?);
        }
    }
    Ok(())
}

fn handle_doctor(
    store: &ConfigStore,
    config: &mut AppConfig,
    fix: bool,
    json: bool,
    verify: bool,
) -> Result<()> {
    let mut fixes = Vec::new();
    if fix {
        store.ensure_dirs()?;
        api_keys::ensure_secrets_file(store)?;
        install_missing_builtin_presets(config);
        fixes.push("ensured config directories, secrets file, and built-in presets".to_string());
        if config.primary_endpoint.is_none() && config.endpoints.len() == 1 {
            config.primary_endpoint = config.endpoints.keys().next().cloned();
            fixes.push("selected the only configured endpoint as primary".to_string());
        }
        if config.default_model.is_none() {
            config.default_model = config
                .primary_endpoint
                .as_ref()
                .and_then(|name| config.endpoints.get(name))
                .and_then(|endpoint| endpoint.default_model.clone());
            if config.default_model.is_some() {
                fixes.push(
                    "copied the primary endpoint default model to the global default".to_string(),
                );
            }
        }
        store.save(config)?;
    }

    let report = build_doctor_report(store, config, fixes)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_doctor_report(&report);
    }

    if verify {
        println!("\n{}", "Verification checks".cyan().bold());
        let checks = [
            ("cargo fmt --check", "fmt", "fmt", &[] as &[&str]),
            (
                "cargo clippy --all-targets -- -D warnings",
                "clippy",
                "clippy",
                &["--all-targets", "--", "-D", "warnings"] as &[&str],
            ),
            ("cargo test", "test", "test", &[] as &[&str]),
            ("cargo build", "build", "build", &[] as &[&str]),
        ];
        let mut all_passed = true;
        for (label, _sub, sub_args, extra_args) in checks {
            let mut cmd = std::process::Command::new("cargo");
            cmd.arg(sub_args);
            for a in extra_args {
                cmd.arg(a);
            }
            let status = cmd.stdout(std::process::Stdio::null()).status();
            let passed = status.map(|s| s.success()).unwrap_or(false);
            if !passed {
                all_passed = false;
            }
            let mark = if passed {
                "PASS".green().to_string()
            } else {
                "FAIL".red().to_string()
            };
            println!("  [{mark}] {label}");
        }
        if !all_passed {
            println!("{}", "one or more verification checks failed".yellow());
        }
    }

    Ok(())
}

#[derive(Debug, Serialize)]
struct DoctorReport {
    paths: DoctorPaths,
    endpoint_count: usize,
    primary_endpoint: Option<String>,
    default_model: Option<String>,
    custom_provider_count: usize,
    mcp_configured: usize,
    mcp_enabled: usize,
    model_cache: DoctorModelCache,
    endpoints: Vec<DoctorEndpoint>,
    mcp_servers: Vec<DoctorMcpServer>,
    sandbox_default: String,
    fixes: Vec<String>,
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
struct DoctorPaths {
    config: PathBuf,
    secrets: PathBuf,
    models: PathBuf,
    sessions: PathBuf,
    skills: PathBuf,
    config_root_writable: bool,
    secrets_exists: bool,
}

#[derive(Debug, Serialize)]
struct DoctorModelCache {
    exists: bool,
    refreshed_at: Option<String>,
    cached_endpoints: usize,
}

#[derive(Debug, Serialize)]
struct DoctorEndpoint {
    name: String,
    provider: String,
    primary: bool,
    base_url: String,
    default_model: Option<String>,
    key_source: String,
    has_key: bool,
    cached_models: usize,
}

#[derive(Debug, Serialize)]
struct DoctorMcpServer {
    name: String,
    enabled: bool,
    built_in: bool,
    command: String,
    command_available: bool,
}

fn build_doctor_report(
    store: &ConfigStore,
    config: &AppConfig,
    fixes: Vec<String>,
) -> Result<DoctorReport> {
    let model_cache = ModelCache::load(store).unwrap_or_default();
    let mut warnings = Vec::new();
    if config.primary_endpoint.is_none() {
        warnings.push("no primary endpoint; run `cntx init`".to_string());
    } else if let Some(primary) = config.primary_endpoint.as_ref() {
        if !config.endpoints.contains_key(primary) {
            warnings.push(format!("primary endpoint `{primary}` does not exist"));
        }
    }
    if config.endpoints.is_empty() {
        warnings.push("no endpoints configured".to_string());
    }

    let endpoints = config
        .endpoints
        .values()
        .map(|endpoint| {
            let (has_key, key_source) = endpoint_key_status(store, endpoint);
            if endpoint.provider.requires_key_by_default() && !has_key {
                warnings.push(format!(
                    "endpoint `{}` has no resolved API key",
                    endpoint.name
                ));
            }
            let cached_models = model_cache
                .endpoints
                .get(&endpoint.name)
                .map(|cached| {
                    cached
                        .models
                        .iter()
                        .filter(|model| model.status == crate::models::ModelStatus::Available)
                        .count()
                })
                .unwrap_or(0);
            if cached_models == 0 {
                warnings.push(format!(
                    "endpoint `{}` has no cached models; run `cntx --refresh-models`",
                    endpoint.name
                ));
            }
            DoctorEndpoint {
                name: endpoint.name.clone(),
                provider: endpoint.provider.as_str().to_string(),
                primary: config.primary_endpoint.as_deref() == Some(endpoint.name.as_str()),
                base_url: endpoint.base_url.clone(),
                default_model: endpoint.default_model.clone(),
                key_source,
                has_key,
                cached_models,
            }
        })
        .collect::<Vec<_>>();

    let mcp_servers = config
        .mcp
        .servers
        .values()
        .map(|server| {
            let command_available = executable_available(&server.command);
            if server.enabled && !command_available {
                warnings.push(format!(
                    "MCP server `{}` command `{}` was not found on PATH",
                    server.name, server.command
                ));
            }
            DoctorMcpServer {
                name: server.name.clone(),
                enabled: server.enabled,
                built_in: server.built_in,
                command: server.command.clone(),
                command_available,
            }
        })
        .collect::<Vec<_>>();

    Ok(DoctorReport {
        paths: DoctorPaths {
            config: store.config_path(),
            secrets: store.secrets_path(),
            models: store.model_cache_path(),
            sessions: store.sessions_dir(),
            skills: store.skills_dir(),
            config_root_writable: is_writable_dir(store.root()),
            secrets_exists: store.secrets_path().exists(),
        },
        endpoint_count: config.endpoints.len(),
        primary_endpoint: config.primary_endpoint.clone(),
        default_model: config.default_model.clone(),
        custom_provider_count: config.custom_providers.len(),
        mcp_configured: config.mcp.servers.len(),
        mcp_enabled: mcp::enabled_servers(config).len(),
        model_cache: DoctorModelCache {
            exists: store.model_cache_path().exists(),
            refreshed_at: model_cache.refreshed_at.map(|value| value.to_rfc3339()),
            cached_endpoints: model_cache.endpoints.len(),
        },
        endpoints,
        mcp_servers,
        sandbox_default: "enabled; writes stay inside the project root unless widened".to_string(),
        fixes,
        warnings,
    })
}

fn print_doctor_report(report: &DoctorReport) {
    println!("{}", "diagnostics".bold());
    for fix in &report.fixes {
        println!("  {} {fix}", "[fixed]".green());
    }
    println!("  config: {}", report.paths.config.display());
    println!("  secrets: {}", report.paths.secrets.display());
    println!("  models: {}", report.paths.models.display());
    println!("  sessions: {}", report.paths.sessions.display());
    println!("  skills: {}", report.paths.skills.display());
    println!(
        "  config root writable: {}",
        yes_no(report.paths.config_root_writable)
    );
    println!("  secrets file: {}", yes_no(report.paths.secrets_exists));
    println!("  endpoints: {}", report.endpoint_count);
    println!(
        "  primary endpoint: {}",
        report.primary_endpoint.as_deref().unwrap_or("<none>")
    );
    println!(
        "  default model: {}",
        report.default_model.as_deref().unwrap_or("<none>")
    );
    println!("  custom providers: {}", report.custom_provider_count);
    println!(
        "  model cache: {} cached endpoints, refreshed {}",
        report.model_cache.cached_endpoints,
        report
            .model_cache
            .refreshed_at
            .as_deref()
            .unwrap_or("<never>")
    );
    println!(
        "  mcp servers: {} configured, {} enabled",
        report.mcp_configured, report.mcp_enabled
    );
    println!("  sandbox: {}", report.sandbox_default);

    if !report.endpoints.is_empty() {
        println!("{}", "endpoints".bold());
        for endpoint in &report.endpoints {
            let marker = if endpoint.primary { "*" } else { " " };
            println!(
                "  {marker} {} [{}] key={} models={} default={}",
                endpoint.name,
                endpoint.provider,
                endpoint.key_source,
                endpoint.cached_models,
                endpoint.default_model.as_deref().unwrap_or("<auto>")
            );
        }
    }

    if !report.mcp_servers.is_empty() {
        println!("{}", "mcp".bold());
        for server in &report.mcp_servers {
            let marker = if server.enabled { "*" } else { " " };
            let available = if server.command_available {
                "ok".green().to_string()
            } else {
                "missing".yellow().to_string()
            };
            println!(
                "  {marker} {} command={} {}",
                server.name, server.command, available
            );
        }
    }

    for warning in &report.warnings {
        println!("  {} {warning}", "[warn]".yellow());
    }
}

fn endpoint_key_status(store: &ConfigStore, endpoint: &EndpointConfig) -> (bool, String) {
    if endpoint
        .api_key
        .as_ref()
        .is_some_and(|key| !key.trim().is_empty())
    {
        return (true, "inline config key".to_string());
    }
    if let Some(env_name) = endpoint.api_key_env.as_ref() {
        if env::var(env_name).is_ok() {
            return (true, format!("env:{env_name}"));
        }
        if api_keys::resolve_for_provider(store, endpoint).is_some() {
            return (true, "runtime secrets store".to_string());
        }
        return (false, format!("missing env:{env_name}"));
    }
    if api_keys::resolve_for_provider(store, endpoint).is_some() {
        return (true, "runtime secrets store".to_string());
    }
    if endpoint.provider.requires_key_by_default() {
        (false, "missing".to_string())
    } else {
        (true, "not required".to_string())
    }
}

fn executable_available(command: &str) -> bool {
    let command_path = PathBuf::from(command);
    if command_path.components().count() > 1 {
        return command_path.exists();
    }
    let Some(paths) = env::var_os("PATH") else {
        return false;
    };
    env::split_paths(&paths).any(|path| path.join(command).exists())
}

fn is_writable_dir(path: &std::path::Path) -> bool {
    fs::metadata(path)
        .map(|metadata| metadata.is_dir() && !metadata.permissions().readonly())
        .unwrap_or(false)
}

fn yes_no(value: bool) -> String {
    if value {
        "yes".green().to_string()
    } else {
        "no".yellow().to_string()
    }
}

fn handle_init(args: InitArgs, store: &ConfigStore, config: &mut AppConfig) -> Result<()> {
    let mut editor = if args.yes {
        None
    } else {
        Some(DefaultEditor::new()?)
    };
    let provider = match args.provider {
        Some(provider) => provider,
        None if args.yes => ProviderKind::Anthropic,
        None => prompt_provider(editor.as_mut())?.unwrap_or(ProviderKind::Anthropic),
    };
    let api_key_env = args
        .api_key_env
        .or_else(|| default_api_key_env(&provider).map(str::to_string));
    let default_model = match args.default_model {
        Some(model) => Some(model),
        None if args.yes => None,
        None => prompt_optional(editor.as_mut(), "Default model (optional)")?,
    };

    let mut endpoint = EndpointConfig::new(args.name.clone(), provider.clone());
    endpoint.api_key_env = api_key_env;
    endpoint.default_model = default_model.clone();
    if provider == ProviderKind::OllamaCloud {
        endpoint.ollama_cloud = Some(Default::default());
    }
    config.endpoints.insert(args.name.clone(), endpoint);
    config.primary_endpoint = Some(args.name.clone());
    if let Some(model) = default_model {
        config.default_model = Some(model);
    }
    install_missing_builtin_presets(config);
    store.save(config)?;

    if let Some(key) = args.api_key {
        api_keys::add(store, provider.as_str(), key.trim())?;
    }

    println!(
        "initialized endpoint `{}` for {}",
        args.name,
        provider.as_str()
    );
    println!("next: cntx --refresh-models");
    println!("then: cntx \"explain this project\"");
    Ok(())
}

fn prompt_provider(editor: Option<&mut DefaultEditor>) -> Result<Option<ProviderKind>> {
    let Some(editor) = editor else {
        return Ok(None);
    };
    println!("Provider [anthropic/openai/ollama-cloud/ollama-local/openai-compatible]");
    let value = editor.readline("> ")?;
    let provider = match value.trim() {
        "" | "anthropic" => ProviderKind::Anthropic,
        "openai" | "open-ai" => ProviderKind::OpenAi,
        "ollama-cloud" => ProviderKind::OllamaCloud,
        "ollama-local" => ProviderKind::OllamaLocal,
        "openai-compatible" | "open-ai-compatible" => ProviderKind::OpenAiCompatible,
        other => return Err(anyhow!("unknown provider `{other}`")),
    };
    Ok(Some(provider))
}

fn prompt_optional(editor: Option<&mut DefaultEditor>, label: &str) -> Result<Option<String>> {
    let Some(editor) = editor else {
        return Ok(None);
    };
    println!("{label}");
    let value = editor.readline("> ")?;
    let value = value.trim();
    Ok((!value.is_empty()).then(|| value.to_string()))
}

fn default_api_key_env(provider: &ProviderKind) -> Option<&'static str> {
    match provider {
        ProviderKind::OpenAi => Some("OPENAI_API_KEY"),
        ProviderKind::Anthropic => Some("ANTHROPIC_API_KEY"),
        ProviderKind::OpenAiCompatible => Some("OPENAI_API_KEY"),
        ProviderKind::OllamaLocal => None,
        ProviderKind::OllamaCloud => Some("OLLAMA_API_KEY"),
    }
}

fn handle_bench(
    cli: &Cli,
    store: &ConfigStore,
    config: &AppConfig,
    json: bool,
    prompt: &[String],
) -> Result<()> {
    let prompt = prompt.join(" ");
    if prompt.trim().is_empty() {
        return Err(anyhow!("provide a prompt to benchmark"));
    }
    let prompt_input = build_prompt_input_with_scan(
        &prompt,
        &project_root(),
        has_project_marker(&project_root()),
    );
    let optimizer = PromptOptimizer;
    let optimized = optimizer.optimize(&prompt_input.text);
    let endpoint_name = cli
        .endpoint
        .clone()
        .or_else(|| config.primary_endpoint.clone())
        .unwrap_or_else(|| "<none>".to_string());
    // Validate that an explicitly requested endpoint exists in config.
    if cli.endpoint.is_some()
        && endpoint_name != "<none>"
        && !config.endpoints.contains_key(&endpoint_name)
    {
        return Err(anyhow!(CntxError::EndpointNotFound(endpoint_name)));
    }
    let endpoint = endpoint_name
        .as_str()
        .ne("<none>")
        .then(|| config.endpoints.get(&endpoint_name))
        .flatten();
    let mut route_reason = None;
    let model = if let Some(model) = cli.model.clone() {
        route_reason = Some("CLI model override".to_string());
        model
    } else {
        endpoint
            .and_then(|endpoint| {
                let cache = ModelCache::load(store).ok()?;
                ModelRouter::new(&config.routing)
                    .route(
                        endpoint,
                        cache.available_for(&endpoint_name),
                        optimized.report.estimated_tokens,
                    )
                    .map(|decision| {
                        route_reason = Some(decision.reason);
                        decision.model
                    })
                    .or_else(|| {
                        config.default_model.clone().inspect(|_| {
                            route_reason = Some("configured default model".to_string())
                        })
                    })
                    .or_else(|| {
                        endpoint
                            .default_model
                            .clone()
                            .inspect(|_| route_reason = Some("endpoint default model".to_string()))
                    })
            })
            .unwrap_or_else(|| "<auto>".to_string())
    };
    let estimate = rough_cost_usd(&model, optimized.report.estimated_tokens, 4096);
    let report = BenchReport {
        user_prompt_chars: prompt.len(),
        original_chars: optimized.report.original_chars,
        optimized_chars: optimized.report.optimized_chars,
        estimated_input_tokens: optimized.report.estimated_tokens,
        duplicate_lines_removed: optimized.report.duplicate_lines_removed,
        saved_chars: optimized
            .report
            .original_chars
            .saturating_sub(optimized.report.optimized_chars),
        endpoint: endpoint_name,
        routed_model: model,
        route_reason,
        rough_request_cost_usd: estimate,
        context: prompt_input.context,
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    println!("{}", "benchmark".bold());
    println!("  user prompt chars: {}", report.user_prompt_chars);
    println!(
        "  original chars sent to optimizer: {}",
        report.original_chars
    );
    println!("  optimized chars: {}", report.optimized_chars);
    println!(
        "  estimated input tokens: {}",
        report.estimated_input_tokens
    );
    println!(
        "  duplicate lines removed: {}",
        report.duplicate_lines_removed
    );
    println!("  context items: {}", report.context.included_items());
    println!("  endpoint: {}", report.endpoint);
    println!("  routed model: {}", report.routed_model);
    if let Some(reason) = report.route_reason.as_ref() {
        println!("  route reason: {reason}");
    }
    if let Some(cost) = report.rough_request_cost_usd {
        println!("  rough request cost: ${cost:.4}");
    } else {
        println!("  rough request cost: unknown for this model");
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct BenchReport {
    user_prompt_chars: usize,
    original_chars: usize,
    optimized_chars: usize,
    estimated_input_tokens: usize,
    duplicate_lines_removed: usize,
    saved_chars: usize,
    endpoint: String,
    routed_model: String,
    route_reason: Option<String>,
    rough_request_cost_usd: Option<f64>,
    context: PromptContextReport,
}

fn rough_cost_usd(model: &str, input_tokens: usize, output_tokens: usize) -> Option<f64> {
    let lower = model.to_lowercase();
    let (input_per_million, output_per_million) =
        if lower.contains("haiku") || lower.contains("mini") || lower.contains("nano") {
            (0.25, 1.25)
        } else if lower.contains("sonnet") || lower.contains("gpt-4") || lower.contains("gpt-5") {
            (3.0, 15.0)
        } else if lower.contains("opus") || lower.contains("pro") || lower.contains("max") {
            (15.0, 75.0)
        } else if lower.contains("ollama") || lower.contains("local") {
            (0.0, 0.0)
        } else {
            return None;
        };
    Some(
        (input_tokens as f64 / 1_000_000.0 * input_per_million)
            + (output_tokens as f64 / 1_000_000.0 * output_per_million),
    )
}

fn handle_demo() -> Result<()> {
    ui::print_markdown(
        "# Cntx demo\n\n\
**Markdown renders** instead of showing raw punctuation.\n\n\
```rust path=src/lib.rs\n\
pub fn answer() -> i32 { 42 }\n\
```\n\n\
- apply mode parses `path=` blocks\n\
- sandboxed writes stay in the project\n\
- `/checklist` shows the last apply result\n",
    );
    println!("{}", "working... done".cyan());
    println!("{}", "file checklist".bold());
    println!("  {} README.md write within sandbox", "[written]".green());
    println!(
        "  {} /tmp/outside.rs path is outside the sandbox",
        "[outside sandbox]".red()
    );
    Ok(())
}

fn handle_memory(command: MemoryCommand) -> Result<()> {
    let path = project_root().join(".cntx").join("memory.md");
    match command {
        MemoryCommand::Add { text } => {
            let note = text.join(" ");
            if note.trim().is_empty() {
                return Err(anyhow!("provide memory text to add"));
            }
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let line = format!("- {}\n", note.trim());
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)?
                .write_all(line.as_bytes())?;
            println!("added memory to {}", path.display());
        }
        MemoryCommand::Show => {
            if path.exists() {
                ui::print_markdown(&std::fs::read_to_string(&path)?);
            } else {
                println!("no project memory yet; add one with `cntx memory add ...`");
            }
        }
        MemoryCommand::Path => println!("{}", path.display()),
    }
    Ok(())
}

async fn refresh_with_spinner(
    config: &AppConfig,
    store: &ConfigStore,
) -> Result<crate::models::RefreshReport> {
    let spinner = ui::spinner("refreshing models from provider APIs");
    let result = refresh_models(config, store).await;
    spinner.finish_and_clear();
    result
}

/// Resolve a model-or-alias string to a concrete model id.
fn resolve_alias_or_model(config: &AppConfig, value: &str) -> Result<String> {
    if let Some(alias) = config.aliases.get(value) {
        Ok(alias.model.clone())
    } else {
        Ok(value.to_string())
    }
}

/// Build the active edit sandbox from CLI safety flags and the project root.
fn build_sandbox(cli: &Cli) -> Sandbox {
    build_sandbox_with_mode(cli, cli.mode)
}

fn build_sandbox_with_mode(cli: &Cli, mode: Mode) -> Sandbox {
    let root = project_root();
    if cli.dangerously_disable_sandbox {
        return Sandbox::disabled(mode, root);
    }
    Sandbox::new(mode, root, cli.allow_write.clone())
}

fn handle_sandbox(cli: &Cli, config: &AppConfig, yaml: bool) -> Result<()> {
    let sandbox = build_sandbox(cli);
    let summary = sandbox.summary();
    if yaml {
        println!("{}", serde_yaml::to_string(&summary)?);
        return Ok(());
    }
    println!(
        "sandbox: {}",
        if summary.enabled {
            "enabled".green().to_string()
        } else {
            "DISABLED".red().to_string()
        }
    );
    println!("mode: {:?}", summary.mode);
    println!("project root: {}", summary.project_root.display());
    println!("writable roots:");
    for root in &summary.allow_write_roots {
        println!("  - {}", root.display());
    }
    println!(
        "custom providers: {} configured",
        config.custom_providers.len()
    );
    Ok(())
}

fn handle_provider(
    command: ProviderCommand,
    store: &ConfigStore,
    config: &mut AppConfig,
) -> Result<()> {
    match command {
        ProviderCommand::Add {
            file,
            name,
            kind,
            base_url,
            api_key_env,
            default_model,
            headers,
        } => {
            if let Some(path) = file {
                let import = load_custom_provider_import(&path)?;
                for provider in import.providers {
                    config
                        .custom_providers
                        .insert(provider.name.clone(), provider);
                }
                store.save(config)?;
                println!("imported providers from {}", path.display());
                return Ok(());
            }

            let name = name.ok_or_else(|| anyhow!("--name is required to add a provider"))?;
            let provider = CustomProvider {
                name: name.clone(),
                kind: kind.unwrap_or_default(),
                base_url,
                api_key_env,
                default_model,
                headers: parse_headers(&headers)?,
                models_path: None,
                chat_path: None,
            };
            config.custom_providers.insert(name.clone(), provider);
            store.save(config)?;
            println!("added provider preset {name}");
            println!(
                "create an endpoint with: cntx endpoint --new --name <name> --from-preset {name}"
            );
            Ok(())
        }
        ProviderCommand::List => {
            for provider in config.custom_providers.values() {
                println!(
                    "{} [{}] {} default={}",
                    provider.name.bold(),
                    provider.kind.as_str(),
                    provider.base_url.as_deref().unwrap_or("<provider default>"),
                    provider.default_model.as_deref().unwrap_or("<auto>"),
                );
            }
            if config.custom_providers.is_empty() {
                println!("no custom provider presets; add one with `cntx provider add`");
            }
            Ok(())
        }
        ProviderCommand::Gallery => {
            for preset in builtin_provider_presets() {
                println!(
                    "{} [{}] {} default={}",
                    preset.name.bold(),
                    preset.kind.as_str(),
                    preset.base_url.as_deref().unwrap_or("<provider default>"),
                    preset.default_model.as_deref().unwrap_or("<auto>")
                );
            }
            Ok(())
        }
        ProviderCommand::InstallPreset { name } => {
            let preset = builtin_provider_presets()
                .into_iter()
                .find(|preset| preset.name == name)
                .ok_or_else(|| anyhow!("unknown built-in provider preset `{name}`"))?;
            config.custom_providers.insert(preset.name.clone(), preset);
            store.save(config)?;
            println!("installed provider preset {name}");
            println!("next: cntx provider use {name}");
            Ok(())
        }
        ProviderCommand::Remove { name } => {
            if config.custom_providers.remove(&name).is_some() {
                store.save(config)?;
                println!("removed provider preset {name}");
            } else {
                println!("no provider preset named {name}");
            }
            Ok(())
        }
        ProviderCommand::Use { name } => {
            let provider = config
                .custom_providers
                .get(&name)
                .cloned()
                .ok_or_else(|| anyhow!("provider preset `{name}` was not found"))?;
            let endpoint = provider.to_endpoint(name.clone());
            config.endpoints.insert(name.clone(), endpoint);
            config.primary_endpoint = Some(name.clone());
            store.save(config)?;
            println!("created endpoint {name} from preset and set it primary");
            println!("add a key with: cntx api-key add --provider {name}");
            println!("then refresh models with: cntx --refresh-models");
            Ok(())
        }
    }
}

fn install_missing_builtin_presets(config: &mut AppConfig) {
    for preset in builtin_provider_presets() {
        config
            .custom_providers
            .entry(preset.name.clone())
            .or_insert(preset);
    }
}

fn builtin_provider_presets() -> Vec<CustomProvider> {
    vec![
        provider_preset(
            "openrouter",
            CustomProviderKind::OpenAiCompatible,
            "https://openrouter.ai/api/v1",
            "OPENROUTER_API_KEY",
            "openrouter/auto",
        ),
        provider_preset(
            "groq",
            CustomProviderKind::OpenAiCompatible,
            "https://api.groq.com/openai/v1",
            "GROQ_API_KEY",
            "llama-3.3-70b-versatile",
        ),
        provider_preset(
            "together",
            CustomProviderKind::OpenAiCompatible,
            "https://api.together.xyz/v1",
            "TOGETHER_API_KEY",
            "meta-llama/Llama-3.3-70B-Instruct-Turbo",
        ),
        provider_preset(
            "fireworks",
            CustomProviderKind::OpenAiCompatible,
            "https://api.fireworks.ai/inference/v1",
            "FIREWORKS_API_KEY",
            "accounts/fireworks/models/llama-v3p1-70b-instruct",
        ),
        // OpenCode Go subscription (https://opencode.ai/docs/go/). Protocol
        // routing per model family lives in the C core; glm-5.3-flash is
        // confirmed in the fetched model list. Never falls back to another
        // provider's key.
        provider_preset(
            "opencode-go",
            CustomProviderKind::OpenAiCompatible,
            "https://opencode.ai/zen/go/v1",
            "OPENCODE_GO_API_KEY",
            "glm-5.3-flash",
        ),
    ]
}

fn provider_preset(
    name: &str,
    kind: CustomProviderKind,
    base_url: &str,
    api_key_env: &str,
    default_model: &str,
) -> CustomProvider {
    CustomProvider {
        name: name.to_string(),
        kind,
        base_url: Some(base_url.to_string()),
        api_key_env: Some(api_key_env.to_string()),
        default_model: Some(default_model.to_string()),
        headers: BTreeMap::new(),
        models_path: None,
        chat_path: None,
    }
}

fn handle_api_key(command: ApiKeyCommand, store: &ConfigStore) -> Result<()> {
    match command {
        ApiKeyCommand::Add { provider, value } => add_or_change_key(store, &provider, value),
        ApiKeyCommand::Change { provider, value } => add_or_change_key(store, &provider, value),
        ApiKeyCommand::Delete { provider } => {
            let label = api_keys::canonical_provider_label(&provider);
            let removed = api_keys::remove(store, &label)?;
            if removed {
                println!("removed key for {label}");
            } else {
                println!("no key stored for {label}");
            }
            Ok(())
        }
        ApiKeyCommand::List => {
            let secrets = api_keys::load(store)?;
            if secrets.keys.is_empty() {
                println!("no keys stored; add one with `cntx api-key --add --provider anthropic`");
                return Ok(());
            }
            for provider in secrets.keys.keys() {
                let key = secrets.get(provider).unwrap_or_default();
                println!("{}", api_keys::ApiSecrets::masked(provider, key));
            }
            Ok(())
        }
    }
}

fn add_or_change_key(store: &ConfigStore, provider: &str, value: Option<String>) -> Result<()> {
    let label = api_keys::canonical_provider_label(provider);
    let key = match value {
        Some(value) => value,
        None => prompt_secret(&label)?,
    };
    if key.trim().is_empty() {
        return Err(anyhow!("key cannot be empty"));
    }
    api_keys::add(store, &label, key.trim())?;
    println!("stored key for {label}");
    Ok(())
}

fn prompt_secret(provider: &str) -> Result<String> {
    #[cfg(unix)]
    {
        println!("Paste the API key for {provider} and press Enter:");
        print!("> ");
        io::stdout().flush()?;
        let _ = std::process::Command::new("stty").arg("-echo").status();
        let mut line = String::new();
        let read_result = io::stdin().read_line(&mut line);
        let _ = std::process::Command::new("stty").arg("echo").status();
        println!();
        read_result?;
        Ok(line)
    }

    #[cfg(not(unix))]
    {
        let mut editor = DefaultEditor::new()?;
        // rustyline echoes input; warn the user before they paste a secret.
        println!("Paste the API key for {provider} and press Enter:");
        let line = editor.readline("> ")?;
        Ok(line)
    }
}

async fn handle_mcp(
    command: McpCommand,
    store: &ConfigStore,
    config: &mut AppConfig,
) -> Result<()> {
    match command {
        McpCommand::List => {
            mcp::print_servers(config);
            Ok(())
        }
        McpCommand::Tools { name } => mcp::print_tools(config, &name).await,
        McpCommand::Add {
            name,
            command,
            args,
            env,
            disabled,
            file,
        } => {
            if let Some(path) = file {
                let import = load_mcp_server_import(&path)?;
                for server in import.servers {
                    config.mcp.servers.insert(server.name.clone(), server);
                }
                store.save(config)?;
                println!("imported MCP servers from {}", path.display());
                return Ok(());
            }
            let server = McpServerConfig {
                name: name.clone(),
                command,
                args,
                env: parse_env(&env)?,
                enabled: !disabled,
                url: None,
                built_in: false,
                description: None,
            };
            config.mcp.servers.insert(name.clone(), server);
            store.save(config)?;
            println!("added MCP server {name}");
            Ok(())
        }
        McpCommand::Remove { name } => {
            if let Some(server) = config.mcp.servers.get(&name) {
                if server.built_in {
                    return Err(anyhow!(
                        "cannot remove built-in server `{name}`; use `cntx mcp disable {name}`"
                    ));
                }
            }
            if config.mcp.servers.remove(&name).is_some() {
                store.save(config)?;
                println!("removed MCP server {name}");
            } else {
                println!("no MCP server named {name}");
            }
            Ok(())
        }
        McpCommand::Enable { name } => {
            toggle_mcp(config, &name, true)?;
            store.save(config)?;
            println!("enabled MCP server {name}");
            Ok(())
        }
        McpCommand::Disable { name } => {
            toggle_mcp(config, &name, false)?;
            store.save(config)?;
            println!("disabled MCP server {name}");
            Ok(())
        }
    }
}

fn toggle_mcp(config: &mut AppConfig, name: &str, enabled: bool) -> Result<()> {
    let server = config
        .mcp
        .servers
        .get_mut(name)
        .ok_or_else(|| anyhow!("MCP server `{name}` is not configured"))?;
    server.enabled = enabled;
    Ok(())
}

fn parse_env(values: &[String]) -> Result<BTreeMap<String, String>> {
    let mut env = BTreeMap::new();
    for value in values {
        let (key, val) = value
            .split_once('=')
            .ok_or_else(|| anyhow!("env must use KEY=VALUE format: {value}"))?;
        env.insert(key.trim().to_string(), val.trim().to_string());
    }
    Ok(env)
}

fn build_endpoint_from_args(
    args: &EndpointArgs,
    existing: Option<EndpointConfig>,
) -> Result<EndpointConfig> {
    let mut endpoint = existing.unwrap_or_else(|| {
        let provider = args.provider.clone().unwrap_or(ProviderKind::OpenAi);
        EndpointConfig::new(
            args.name.clone().unwrap_or_else(|| "default".to_string()),
            provider,
        )
    });

    if let Some(name) = args.name.as_ref() {
        endpoint.name = name.clone();
    }
    if let Some(provider) = args.provider.as_ref() {
        endpoint.provider = provider.clone();
        if args.base_url.is_none() {
            endpoint.base_url = provider.default_base_url().to_string();
        }
    }
    if let Some(api_key) = args.api_key.as_ref() {
        endpoint.api_key = Some(api_key.clone());
    }
    if let Some(api_key_env) = args.api_key_env.as_ref() {
        endpoint.api_key_env = Some(api_key_env.clone());
        endpoint.api_key = None;
    }
    if endpoint.provider == ProviderKind::OllamaCloud
        && endpoint.api_key.is_none()
        && endpoint.api_key_env.is_none()
    {
        endpoint.api_key_env = Some("OLLAMA_API_KEY".to_string());
    }
    if let Some(base_url) = args.base_url.as_ref() {
        endpoint.base_url = base_url.clone();
    }
    if let Some(default_model) = args.default_model.as_ref() {
        endpoint.default_model = Some(default_model.clone());
    }
    endpoint.timeout_secs = args.timeout_secs;
    endpoint
        .custom_headers
        .extend(parse_headers(&args.headers)?);
    apply_ollama_cloud_options(args, &mut endpoint)?;
    Ok(endpoint)
}

fn apply_ollama_cloud_options(args: &EndpointArgs, endpoint: &mut EndpointConfig) -> Result<()> {
    if (args.ollama_cloud_plan.is_some() || args.ollama_subscription_models)
        && endpoint.provider != ProviderKind::OllamaCloud
    {
        return Err(anyhow!(
            "--ollama-cloud-plan and --ollama-subscription-models require --provider ollama-cloud"
        ));
    }

    if endpoint.provider != ProviderKind::OllamaCloud {
        endpoint.ollama_cloud = None;
        return Ok(());
    }

    let mut options = endpoint.ollama_cloud.take().unwrap_or_default();
    if let Some(plan) = args.ollama_cloud_plan {
        options.plan = Some(plan);
        if plan.includes_subscription_models() {
            options.subscription_models = true;
        }
    }
    if args.ollama_subscription_models {
        options.subscription_models = true;
    }
    endpoint.ollama_cloud = Some(options);
    Ok(())
}

fn parse_headers(values: &[String]) -> Result<BTreeMap<String, String>> {
    let mut headers = BTreeMap::new();
    for value in values {
        let (key, header_value) = value
            .split_once('=')
            .ok_or_else(|| anyhow!("header must use KEY=VALUE format: {value}"))?;
        headers.insert(key.trim().to_string(), header_value.trim().to_string());
    }
    Ok(headers)
}

fn print_endpoints(config: &AppConfig) {
    for endpoint in config.endpoints.values() {
        let primary = if config.primary_endpoint.as_deref() == Some(endpoint.name.as_str()) {
            "*"
        } else {
            " "
        };
        println!(
            "{} {} [{}] {} default={}{}",
            primary,
            endpoint.name.bold(),
            endpoint.provider.as_str(),
            endpoint.base_url,
            endpoint.default_model.as_deref().unwrap_or("<auto>"),
            endpoint
                .ollama_cloud
                .as_ref()
                .and_then(|options| options.plan)
                .map(|plan| format!(" plan={}", plan.as_str()))
                .unwrap_or_default()
        );
    }
}

/// Resolve the project root by searching upward for a `.git` or `.cntx`
/// marker directory. Falls back to the current working directory when no
/// marker is found, but the caller can check for a marker to decide whether
/// automatic context scanning is safe.
fn project_root() -> PathBuf {
    if let Ok(cwd) = std::env::current_dir() {
        if let Some(root) = find_project_marker(&cwd) {
            return root;
        }
        return cwd;
    }
    PathBuf::from(".")
}

/// Walk upward from `start` looking for a directory containing `.git` or
/// `.cntx`. Returns the first ancestor that contains one, or `None`.
fn find_project_marker(start: &Path) -> Option<PathBuf> {
    let mut current = Some(start);
    while let Some(dir) = current {
        if dir.join(".git").exists() || dir.join(".cntx").exists() {
            return Some(dir.to_path_buf());
        }
        current = dir.parent();
    }
    None
}

/// Returns true when the project root contains a `.git` or `.cntx` marker,
/// indicating it is a real project workspace where automatic context scanning
/// is safe and useful. When false, the caller should skip the recursive scan
/// to avoid walking an entire home directory or other large tree.
fn has_project_marker(root: &Path) -> bool {
    root.join(".git").exists() || root.join(".cntx").exists()
}

/// Rough token estimate for output text (chars/4 or words, whichever is more).
fn estimate_output_tokens(text: &str) -> usize {
    let chars = text.chars().count();
    let words = text.split_whitespace().count();
    chars.div_ceil(4).max(words)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ollama_cloud_endpoint_defaults_to_api_key_env_and_records_plan() {
        let args = EndpointArgs {
            new: true,
            change: None,
            remove: None,
            list: false,
            set_primary: None,
            import: None,
            name: Some("ollama-pro".to_string()),
            provider: Some(ProviderKind::OllamaCloud),
            from_preset: None,
            api_key: None,
            api_key_env: None,
            base_url: None,
            default_model: Some("deepseek-v4-pro:cloud".to_string()),
            timeout_secs: 120,
            headers: Vec::new(),
            ollama_cloud_plan: Some(crate::config::OllamaCloudPlan::Pro),
            ollama_subscription_models: false,
        };

        let endpoint = build_endpoint_from_args(&args, None).unwrap();
        let options = endpoint.ollama_cloud.unwrap();

        assert_eq!(endpoint.api_key_env.as_deref(), Some("OLLAMA_API_KEY"));
        assert_eq!(
            endpoint.default_model.as_deref(),
            Some("deepseek-v4-pro:cloud")
        );
        assert_eq!(options.plan, Some(crate::config::OllamaCloudPlan::Pro));
        assert!(options.subscription_models);
    }

    #[test]
    fn ollama_cloud_options_are_rejected_for_other_providers() {
        let args = EndpointArgs {
            new: true,
            change: None,
            remove: None,
            list: false,
            set_primary: None,
            import: None,
            name: Some("openai".to_string()),
            provider: Some(ProviderKind::OpenAi),
            from_preset: None,
            api_key: None,
            api_key_env: None,
            base_url: None,
            default_model: None,
            timeout_secs: 120,
            headers: Vec::new(),
            ollama_cloud_plan: Some(crate::config::OllamaCloudPlan::Pro),
            ollama_subscription_models: false,
        };

        let error = build_endpoint_from_args(&args, None).unwrap_err();

        assert!(error
            .to_string()
            .contains("require --provider ollama-cloud"));
    }
}
