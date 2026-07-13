use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use clap_complete::Shell;

use crate::config::{CustomProviderKind, OllamaCloudPlan, ProviderKind};
use crate::permissions::Mode;

#[derive(Debug, Parser)]
#[command(name = "cntx")]
#[command(
    author,
    version,
    about = "Cntx Code: BYOK, token-efficient AI coding assistant"
)]
pub struct Cli {
    #[arg(long, global = true, help = "Model id or alias to use for this prompt")]
    pub model: Option<String>,

    #[arg(long, global = true, help = "Endpoint name to use")]
    pub endpoint: Option<String>,

    #[arg(long, global = true, value_enum, default_value_t = Mode::Auto)]
    pub mode: Mode,

    #[arg(
        long,
        global = true,
        help = "Refresh model lists from configured provider APIs"
    )]
    pub refresh_models: bool,

    #[arg(
        long,
        global = true,
        help = "Open the packaged interactive docs browser"
    )]
    pub docs: bool,

    #[arg(long, global = true, help = "Run a single prompt and exit")]
    pub no_interactive: bool,

    /// Extend the edit sandbox with an additional writable directory.
    /// Repeatable. The sandbox is on by default and confines edits to the
    /// project root.
    #[arg(long, global = true, value_name = "PATH")]
    pub allow_write: Vec<PathBuf>,

    /// Disable the edit sandbox entirely. Dangerous: the assistant may then
    /// edit files anywhere on the machine.
    #[arg(long, global = true, help_heading = "Safety")]
    pub dangerously_disable_sandbox: bool,

    /// Write the model's code to files. The model is asked to emit each file as
    /// a fenced block annotated with `path=<relative path>`; Cntx writes them
    /// through the sandbox and prints a checklist.
    #[arg(long, global = true, help_heading = "Safety")]
    pub apply: bool,

    /// In apply mode, show file previews without writing anything.
    #[arg(long, global = true, help_heading = "Safety")]
    pub dry_run: bool,

    /// Enable the tool-use loop so the model can read, write, edit files, and
    /// run shell commands through tool calls.
    #[arg(long, global = true, help_heading = "Tools")]
    pub tool_use: bool,

    #[command(subcommand)]
    pub command: Option<Command>,

    #[arg(value_name = "PROMPT", trailing_var_arg = true)]
    pub prompt: Vec<String>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Manage provider endpoints (create, list, set primary, import).
    Endpoint(EndpointArgs),
    /// Manage model aliases, the default model, and the model cache.
    #[command(subcommand)]
    Model(ModelCommand),
    /// Manage custom provider presets defined in YAML.
    #[command(subcommand)]
    Provider(ProviderCommand),
    /// Manage runtime API keys stored in the gitignored secrets file.
    #[command(subcommand)]
    ApiKey(ApiKeyCommand),
    /// First-run setup for provider, endpoint, key, and default model.
    Init(InitArgs),
    /// Estimate prompt optimization, routing, and rough cost without calling a model.
    Bench {
        #[arg(long, help = "Print benchmark data as JSON")]
        json: bool,
        #[arg(value_name = "PROMPT", trailing_var_arg = true)]
        prompt: Vec<String>,
    },
    /// Show a canned local demo of markdown chat, apply mode, and the checklist.
    Demo,
    /// Generate shell completions.
    Completions { shell: Shell },
    /// Manage project memory stored in .cntx/memory.md.
    #[command(subcommand)]
    Memory(MemoryCommand),
    /// Manage built-in and custom MCP servers (doc search, token saving, tools).
    #[command(subcommand)]
    Mcp(McpCommand),
    /// Inspect or initialize the configuration file.
    #[command(subcommand)]
    Config(ConfigCommand),
    /// List, resume, export, or import sessions.
    #[command(subcommand)]
    Session(SessionCommand),
    /// Manage reusable skills.
    #[command(subcommand)]
    Skill(SkillCommand),
    /// Show the active edit sandbox and permission configuration.
    Sandbox {
        #[arg(long, help = "Print the sandbox summary as YAML")]
        yaml: bool,
    },
    /// Show configuration paths and a diagnostics summary.
    Doctor {
        #[arg(
            long,
            help = "Create missing local config files and apply safe defaults"
        )]
        fix: bool,
        #[arg(long, help = "Print diagnostics as JSON")]
        json: bool,
        #[arg(
            long,
            help = "Run cargo fmt, clippy, test, and build, then report pass/fail"
        )]
        verify: bool,
    },
}

#[derive(Debug, Args)]
pub struct InitArgs {
    #[arg(long, value_enum, help = "Provider to configure")]
    pub provider: Option<ProviderKind>,

    #[arg(
        long,
        default_value = "work",
        help = "Endpoint name to create or update"
    )]
    pub name: String,

    #[arg(long, help = "Default model for the endpoint and global default")]
    pub default_model: Option<String>,

    #[arg(long, help = "Environment variable that stores the API key")]
    pub api_key_env: Option<String>,

    #[arg(long, help = "Store an API key value in the runtime key store")]
    pub api_key: Option<String>,

    #[arg(long, help = "Skip interactive prompts and use defaults/flags only")]
    pub yes: bool,
}

#[derive(Debug, Args)]
pub struct EndpointArgs {
    #[arg(long = "new", help = "Create a new endpoint")]
    pub new: bool,

    #[arg(long, value_name = "NAME", help = "Change/update an endpoint")]
    pub change: Option<String>,

    #[arg(long, value_name = "NAME", help = "Remove an endpoint")]
    pub remove: Option<String>,

    #[arg(long, help = "List endpoints")]
    pub list: bool,

    #[arg(long, value_name = "NAME", help = "Set the primary endpoint")]
    pub set_primary: Option<String>,

    #[arg(long, value_name = "FILE", help = "Import endpoints from YAML")]
    pub import: Option<PathBuf>,

    #[arg(long, value_name = "NAME", help = "Endpoint name")]
    pub name: Option<String>,

    #[arg(long, value_enum, help = "Provider kind")]
    pub provider: Option<ProviderKind>,

    /// Create this endpoint from a custom provider preset
    #[arg(
        long,
        value_name = "PRESET",
        help = "Create endpoint from a custom provider preset"
    )]
    pub from_preset: Option<String>,

    #[arg(
        long,
        help = "API key value; prefer --api-key-env or `cntx api-key --add`"
    )]
    pub api_key: Option<String>,

    #[arg(long, help = "Environment variable that stores the API key")]
    pub api_key_env: Option<String>,

    #[arg(long, help = "Provider base URL")]
    pub base_url: Option<String>,

    #[arg(long, help = "Default model for this endpoint")]
    pub default_model: Option<String>,

    #[arg(long, default_value_t = 120, help = "Request timeout in seconds")]
    pub timeout_secs: u64,

    #[arg(long = "header", value_name = "KEY=VALUE", help = "Custom header")]
    pub headers: Vec<String>,

    #[arg(long, value_enum, help = "Ollama Cloud plan for this endpoint")]
    pub ollama_cloud_plan: Option<OllamaCloudPlan>,

    #[arg(
        long,
        help = "Mark this Ollama Cloud endpoint as intended for Pro/Max subscription models"
    )]
    pub ollama_subscription_models: bool,
}

#[derive(Debug, Subcommand)]
pub enum ModelCommand {
    /// Add a model alias.
    Add(ModelAddArgs),
    /// Remove a model alias.
    Remove(ModelRemoveArgs),
    /// Set or clear the default model used when no --model override applies.
    Default {
        /// Model id or alias to make the default, or omit with --unset to clear.
        model: Option<String>,
        #[arg(long, help = "Clear the configured default model")]
        unset: bool,
    },
    /// List aliases, the default model, and cached models.
    List,
    /// Refresh the model cache from provider APIs.
    Refresh,
}

#[derive(Debug, Args)]
pub struct ModelAddArgs {
    pub model: String,

    #[arg(long, help = "Alias name")]
    pub name: String,

    #[arg(long, help = "Optional endpoint this alias belongs to")]
    pub endpoint: Option<String>,
}

#[derive(Debug, Args)]
pub struct ModelRemoveArgs {
    pub name: String,
}

#[derive(Debug, Subcommand)]
pub enum ProviderCommand {
    /// Add a custom provider preset from flags or YAML.
    Add {
        #[arg(long, value_name = "FILE", help = "Import provider presets from YAML")]
        file: Option<PathBuf>,
        #[arg(long, value_name = "NAME", help = "Name for the new provider preset")]
        name: Option<String>,
        #[arg(long, value_enum, help = "Adapter family to reuse")]
        kind: Option<CustomProviderKind>,
        #[arg(long, value_name = "URL", help = "Base URL")]
        base_url: Option<String>,
        #[arg(
            long,
            value_name = "ENV",
            help = "Environment variable holding the API key"
        )]
        api_key_env: Option<String>,
        #[arg(long, value_name = "MODEL", help = "Default model id")]
        default_model: Option<String>,
        #[arg(long = "header", value_name = "KEY=VALUE", help = "Custom header")]
        headers: Vec<String>,
    },
    /// List configured custom provider presets.
    List,
    /// List built-in provider presets that can be installed.
    Gallery,
    /// Install a built-in provider preset by name.
    InstallPreset { name: String },
    /// Remove a custom provider preset.
    Remove { name: String },
    /// Create an endpoint from a preset and set it as primary.
    Use { name: String },
}

#[derive(Debug, Subcommand)]
pub enum MemoryCommand {
    /// Append a note to .cntx/memory.md.
    Add {
        #[arg(value_name = "TEXT", trailing_var_arg = true)]
        text: Vec<String>,
    },
    /// Print project memory.
    Show,
    /// Print the project memory path.
    Path,
}

#[derive(Debug, Subcommand)]
pub enum ApiKeyCommand {
    /// Add or replace an API key for a provider.
    Add {
        #[arg(long, help = "Provider label, e.g. anthropic, openai, ollama-cloud")]
        provider: String,
        #[arg(long, help = "Key value; if omitted you are prompted securely")]
        value: Option<String>,
    },
    /// Replace an existing key (alias for add).
    Change {
        #[arg(long)]
        provider: String,
        #[arg(long)]
        value: Option<String>,
    },
    /// Remove a stored key.
    Delete {
        #[arg(long)]
        provider: String,
    },
    /// List providers with a stored key.
    List,
}

#[derive(Debug, Subcommand)]
pub enum McpCommand {
    /// List configured MCP servers.
    List,
    /// Show the tools a server exposes by connecting to it on demand.
    Tools { name: String },
    /// Add a custom MCP server.
    Add {
        name: String,
        command: String,
        #[arg(long = "arg", value_name = "ARG", help = "Repeatable argument")]
        args: Vec<String>,
        #[arg(
            long = "env",
            value_name = "KEY=VALUE",
            help = "Repeatable environment variable"
        )]
        env: Vec<String>,
        #[arg(long, help = "Disable the server after adding")]
        disabled: bool,
        #[arg(long, value_name = "FILE", help = "Import MCP servers from YAML")]
        file: Option<PathBuf>,
    },
    /// Remove a custom MCP server. Built-in servers cannot be removed.
    Remove { name: String },
    /// Enable a server.
    Enable { name: String },
    /// Disable a server.
    Disable { name: String },
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Write the current configuration to disk.
    Init,
    /// Print the configuration file path.
    Path,
    /// Print the current configuration as YAML.
    Show,
}

#[derive(Debug, Subcommand)]
pub enum SessionCommand {
    /// List saved sessions.
    List,
    /// Resume and print a saved session.
    Resume { id: Option<String> },
    /// Export a session to JSON.
    Export { id: String, output: PathBuf },
    /// Import a session from JSON or YAML.
    Import { input: PathBuf },
}

#[derive(Debug, Subcommand)]
pub enum SkillCommand {
    /// List available skills.
    List,
    /// Create a new skill.
    New { name: String, description: String },
    /// Show a skill's contents.
    Show { name: String },
}
