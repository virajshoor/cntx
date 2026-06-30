use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{anyhow, Result};
use chrono::Utc;
use owo_colors::OwoColorize;
use rustyline::DefaultEditor;

use crate::api_keys;
use crate::cli::{
    ApiKeyCommand, Cli, Command, ConfigCommand, EndpointArgs, McpCommand, ModelCommand,
    ProviderCommand, SessionCommand, SkillCommand,
};
use crate::config::{
    load_custom_provider_import, load_endpoint_import, load_mcp_server_import, AppConfig,
    ConfigStore, CustomProvider, EndpointConfig, McpServerConfig, ModelAlias, ProviderKind,
};
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
use crate::sessions::{Session, SessionStore};
use crate::skills::SkillStore;
use crate::ui;

pub async fn run(cli: Cli) -> Result<()> {
    let store = ConfigStore::from_standard_locations()?;
    // First-boot auto-setup: create the secrets file so the tool is ready to
    // receive API keys on any machine after a fresh build.
    api_keys::ensure_secrets_file(&store)?;
    let mut config = store.load()?;

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
        Some(Command::Mcp(command)) => handle_mcp(command, &store, &mut config).await,
        Some(Command::Config(command)) => handle_config(command, &store, &config),
        Some(Command::Session(command)) => handle_session(command, &store),
        Some(Command::Skill(command)) => handle_skill(command, &store),
        Some(Command::Sandbox { yaml }) => handle_sandbox(&cli, &config, yaml),
        Some(Command::Doctor) => handle_doctor(&store, &config),
        None => {
            let prompt = cli.prompt.join(" ");
            let sandbox = build_sandbox(&cli);
            let mut runtime = Runtime::new(
                config,
                store,
                cli.endpoint,
                cli.model,
                cli.mode,
                cli.apply,
                sandbox,
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
    pub apply: bool,
    pub sandbox: Sandbox,
    pub session: Session,
    pub last_apply_outcomes: Vec<crate::apply::ApplyOutcome>,
}

impl Runtime {
    pub fn new(
        config: AppConfig,
        store: ConfigStore,
        endpoint_override: Option<String>,
        model_override: Option<String>,
        mode: Mode,
        apply: bool,
        sandbox: Sandbox,
    ) -> Result<Self> {
        Ok(Self {
            config,
            store,
            endpoint_override,
            model_override,
            mode,
            apply,
            sandbox,
            session: Session::new("interactive"),
            last_apply_outcomes: Vec::new(),
        })
    }

    pub async fn run_prompt(&mut self, prompt: &str) -> Result<()> {
        let optimizer = PromptOptimizer;
        let optimized = optimizer.optimize(prompt);
        let (endpoint_name, endpoint) = self.resolve_endpoint()?;
        if self.mode == Mode::Counsel {
            return self
                .run_counsel_prompt(prompt, &optimized, &endpoint_name, endpoint)
                .await;
        }

        let model =
            self.resolve_model(&endpoint_name, &endpoint, optimized.report.estimated_tokens)?;

        let mode_label = if self.apply { "apply" } else { "auto" };
        println!(
            "{} endpoint={} model={} mode={} tokens={} saved={} chars",
            "->".cyan(),
            endpoint_name,
            model,
            mode_label,
            optimized.report.estimated_tokens,
            optimized
                .report
                .original_chars
                .saturating_sub(optimized.report.optimized_chars)
        );

        let mut messages = Vec::new();
        if self.apply {
            messages.push(ChatMessage {
                role: "system".to_string(),
                content: crate::apply::APPLY_SYSTEM_INSTRUCTION.to_string(),
            });
        }
        messages.push(ChatMessage {
            role: "user".to_string(),
            content: optimized.text.clone(),
        });
        let request = ChatRequest {
            model,
            messages,
            max_tokens: Some(4096),
        };
        validate_chat_request(&request)?;

        self.session.push("user", prompt);
        let assistant_text = self.generate(&endpoint, request).await?;
        ui::print_markdown(&assistant_text);
        println!();

        if self.apply {
            let files = crate::apply::extract_files(&assistant_text);
            if !files.is_empty() {
                let outcomes =
                    crate::apply::apply(&self.sandbox, &files, self.sandbox.project_root());
                crate::apply::print_checklist(&outcomes);
                self.last_apply_outcomes = outcomes;
            } else {
                self.last_apply_outcomes.clear();
                println!(
                    "{}",
                    "no path=-annotated code blocks found to apply".dimmed()
                );
            }
        }

        self.session.push("assistant", assistant_text);
        SessionStore::new(&self.store).save(&self.session)?;
        Ok(())
    }

    /// Run a chat request, buffering the stream behind a working indicator so
    /// the terminal does not look frozen. Returns the full assistant text.
    async fn generate(&self, endpoint: &EndpointConfig, request: ChatRequest) -> Result<String> {
        let spinner = ui::working();
        let mut assistant_text = String::new();
        let adapter = adapter_for(endpoint.provider.clone());
        let result = adapter
            .stream_chat(endpoint, request, &mut |delta| {
                assistant_text.push_str(&delta);
            })
            .await;
        spinner.finish_and_clear();
        result?;
        Ok(assistant_text)
    }

    async fn run_counsel_prompt(
        &mut self,
        prompt: &str,
        optimized: &crate::optimizer::OptimizedPrompt,
        endpoint_name: &str,
        endpoint: EndpointConfig,
    ) -> Result<()> {
        let cache = ModelCache::load(&self.store)?;
        let plan = self.resolve_counsel_plan(endpoint_name, &endpoint, &cache, &optimized.text)?;
        let worker_model = self
            .resolve_model_override()
            .unwrap_or_else(|| plan.worker_model.clone());

        println!(
            "{} mode=counsel endpoint={} evaluator={} worker={} task={} tokens={} saved={} chars",
            "->".cyan(),
            endpoint_name,
            plan.evaluator_model,
            worker_model,
            plan.task.as_str(),
            optimized.report.estimated_tokens,
            optimized
                .report
                .original_chars
                .saturating_sub(optimized.report.optimized_chars)
        );

        self.session.push("user", prompt);

        let adapter = adapter_for(endpoint.provider.clone());
        let evaluation_prompt =
            build_evaluation_prompt(&optimized.text, optimized.report.estimated_tokens);
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
            },
        )
        .await?;

        if worker_model == plan.evaluator_model {
            ui::print_markdown(&evaluation);
            println!();
            self.session
                .push("assistant", format!("Counsel evaluation:\n{evaluation}"));
            SessionStore::new(&self.store).save(&self.session)?;
            return Ok(());
        }

        let worker_prompt = build_worker_prompt(&optimized.text, &evaluation, plan.task);
        let request = ChatRequest {
            model: worker_model,
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: worker_prompt,
            }],
            max_tokens: Some(4096),
        };
        validate_chat_request(&request)?;

        let assistant_text = self.generate(&endpoint, request).await?;
        ui::print_markdown(&assistant_text);
        println!();

        if self.apply {
            let files = crate::apply::extract_files(&assistant_text);
            if !files.is_empty() {
                let outcomes =
                    crate::apply::apply(&self.sandbox, &files, self.sandbox.project_root());
                crate::apply::print_checklist(&outcomes);
                self.last_apply_outcomes = outcomes;
            } else {
                self.last_apply_outcomes.clear();
                println!(
                    "{}",
                    "no path=-annotated code blocks found to apply".dimmed()
                );
            }
        }

        self.session.push(
            "assistant",
            format!("Counsel evaluation:\n{evaluation}\n\nResponse:\n{assistant_text}"),
        );
        SessionStore::new(&self.store).save(&self.session)?;
        Ok(())
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

    fn resolve_endpoint(&self) -> Result<(String, EndpointConfig)> {
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

    fn resolve_model(
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
                None,
                None,
                Mode::Auto,
                false,
                sandbox,
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

fn handle_session(command: SessionCommand, store: &ConfigStore) -> Result<()> {
    let sessions = SessionStore::new(store);
    match command {
        SessionCommand::List => {
            for session in sessions.list()? {
                println!(
                    "{} {} {} messages",
                    session.id,
                    session.updated_at,
                    session.messages.len()
                );
            }
        }
        SessionCommand::Resume { id } => {
            let session = sessions.load(&id)?;
            println!("{}", serde_yaml::to_string(&session)?);
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

fn handle_doctor(store: &ConfigStore, config: &AppConfig) -> Result<()> {
    println!("config: {}", store.config_path().display());
    println!("secrets: {}", store.secrets_path().display());
    println!("models: {}", store.model_cache_path().display());
    println!("sessions: {}", store.sessions_dir().display());
    println!("skills: {}", store.skills_dir().display());
    println!("endpoints: {}", config.endpoints.len());
    println!(
        "primary endpoint: {}",
        config.primary_endpoint.as_deref().unwrap_or("<none>")
    );
    println!(
        "default model: {}",
        config.default_model.as_deref().unwrap_or("<none>")
    );
    println!("custom providers: {}", config.custom_providers.len());
    let mcp_enabled = mcp::enabled_servers(config).len();
    println!(
        "mcp servers: {} configured, {} enabled",
        config.mcp.servers.len(),
        mcp_enabled
    );
    println!("sandbox: enabled by default; see `cntx sandbox`");
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
    let root = project_root();
    if cli.dangerously_disable_sandbox {
        return Sandbox::disabled(cli.mode, root);
    }
    Sandbox::new(cli.mode, root, cli.allow_write.clone())
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
            println!(
                "add a key with: cntx api-key --add --provider {}",
                provider.provider_kind().as_str()
            );
            Ok(())
        }
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
    let mut editor = DefaultEditor::new()?;
    // rustyline echoes input; warn the user before they paste a secret.
    println!("Paste the API key for {provider} and press Enter:");
    let line = editor.readline("> ")?;
    Ok(line)
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

fn project_root() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
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
