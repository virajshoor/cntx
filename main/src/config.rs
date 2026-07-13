use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::permissions::Mode;

pub const CONFIG_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AppConfig {
    pub version: u32,
    pub primary_endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
    pub endpoints: BTreeMap<String, EndpointConfig>,
    pub aliases: BTreeMap<String, ModelAlias>,
    #[serde(default)]
    pub custom_providers: BTreeMap<String, CustomProvider>,
    #[serde(default)]
    pub mcp: McpConfig,
    pub routing: RoutingConfig,
    pub ui: UiConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            primary_endpoint: None,
            default_model: None,
            endpoints: BTreeMap::new(),
            aliases: BTreeMap::new(),
            custom_providers: BTreeMap::new(),
            mcp: McpConfig::default().with_builtins(),
            routing: RoutingConfig::default(),
            ui: UiConfig::default(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderKind {
    OpenAi,
    Anthropic,
    OpenAiCompatible,
    OllamaLocal,
    OllamaCloud,
}

impl ProviderKind {
    pub fn default_base_url(&self) -> &'static str {
        match self {
            Self::OpenAi => "https://api.openai.com/v1",
            Self::Anthropic => "https://api.anthropic.com/v1",
            Self::OpenAiCompatible => "https://api.openai.com/v1",
            Self::OllamaLocal => "http://localhost:11434",
            Self::OllamaCloud => "https://ollama.com",
        }
    }

    pub fn requires_key_by_default(&self) -> bool {
        !matches!(self, Self::OllamaLocal)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::OpenAiCompatible => "openai-compatible",
            Self::OllamaLocal => "ollama-local",
            Self::OllamaCloud => "ollama-cloud",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum OllamaCloudPlan {
    Free,
    Pro,
    Max,
    Team,
}

impl OllamaCloudPlan {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Free => "free",
            Self::Pro => "pro",
            Self::Max => "max",
            Self::Team => "team",
        }
    }

    pub fn includes_subscription_models(self) -> bool {
        matches!(self, Self::Pro | Self::Max | Self::Team)
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct OllamaCloudOptions {
    pub plan: Option<OllamaCloudPlan>,
    pub subscription_models: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EndpointConfig {
    pub name: String,
    pub provider: ProviderKind,
    pub api_key: Option<String>,
    pub api_key_env: Option<String>,
    pub base_url: String,
    pub default_model: Option<String>,
    pub custom_headers: BTreeMap<String, String>,
    pub timeout_secs: u64,
    pub metadata: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ollama_cloud: Option<OllamaCloudOptions>,
}

impl EndpointConfig {
    pub fn new(name: impl Into<String>, provider: ProviderKind) -> Self {
        let base_url = provider.default_base_url().to_string();
        Self {
            name: name.into(),
            provider,
            api_key: None,
            api_key_env: None,
            base_url,
            default_model: None,
            custom_headers: BTreeMap::new(),
            timeout_secs: 120,
            metadata: BTreeMap::new(),
            ollama_cloud: None,
        }
    }

    pub fn resolved_api_key(&self) -> Option<String> {
        self.api_key
            .clone()
            .or_else(|| self.api_key_env.as_ref().and_then(|key| env::var(key).ok()))
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ModelAlias {
    pub alias: String,
    pub model: String,
    pub endpoint: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RoutingConfig {
    pub thresholds: RoutingThresholds,
    pub family_overrides: BTreeMap<String, BTreeMap<RouteSize, String>>,
    pub default_models: BTreeMap<String, String>,
    /// Number of prior user/assistant turns from the current session to inject
    /// into each new prompt. 0 disables history. Defaults to 10.
    #[serde(default = "default_history_turns")]
    pub history_turns: usize,
}

fn default_history_turns() -> usize {
    10
}

impl Default for RoutingConfig {
    fn default() -> Self {
        Self {
            thresholds: RoutingThresholds::default(),
            family_overrides: BTreeMap::new(),
            default_models: BTreeMap::new(),
            history_turns: 10,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RoutingThresholds {
    pub small_prompt_tokens: usize,
    pub medium_prompt_tokens: usize,
}

impl Default for RoutingThresholds {
    fn default() -> Self {
        Self {
            small_prompt_tokens: 2_000,
            medium_prompt_tokens: 12_000,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum RouteSize {
    Small,
    Medium,
    Large,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct UiConfig {
    pub theme: String,
    pub markdown: bool,
    pub syntax_highlighting: bool,
    pub vim_keys: bool,
    pub mode: Mode,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            theme: "system".to_string(),
            markdown: true,
            syntax_highlighting: true,
            vim_keys: false,
            mode: Mode::Auto,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ConfigStore {
    root: PathBuf,
}

impl ConfigStore {
    pub fn from_standard_locations() -> Result<Self> {
        let root = if let Ok(path) = env::var("CNTX_CONFIG_DIR") {
            PathBuf::from(path)
        } else {
            dirs::config_dir()
                .context("could not determine user config directory")?
                .join("cntxcode")
        };
        Ok(Self { root })
    }

    pub fn from_root(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn config_path(&self) -> PathBuf {
        self.root.join("config.yaml")
    }

    pub fn model_cache_path(&self) -> PathBuf {
        self.root.join("models.yaml")
    }

    /// Path to the gitignored runtime secrets file. This file stores API keys
    /// added with `cntx api-key --add` and is created automatically on first
    /// boot. It is never part of the source tree or published crate.
    pub fn secrets_path(&self) -> PathBuf {
        self.root.join("secrets.yaml")
    }

    pub fn sessions_dir(&self) -> PathBuf {
        self.root.join("sessions")
    }

    pub fn skills_dir(&self) -> PathBuf {
        self.root.join("skills")
    }

    /// Path to the persistent rustyline command history file.
    pub fn history_path(&self) -> PathBuf {
        self.root.join("history.txt")
    }

    pub fn ensure_dirs(&self) -> Result<()> {
        fs::create_dir_all(&self.root)?;
        fs::create_dir_all(self.sessions_dir())?;
        fs::create_dir_all(self.skills_dir())?;
        Ok(())
    }

    pub fn load(&self) -> Result<AppConfig> {
        self.ensure_dirs()?;
        let path = self.config_path();
        if !path.exists() {
            return Ok(AppConfig::default());
        }

        let raw = fs::read_to_string(&path)
            .with_context(|| format!("failed to read config at {}", path.display()))?;
        let mut config: AppConfig = serde_yaml::from_str(&raw)
            .with_context(|| format!("failed to parse config at {}", path.display()))?;
        // Built-in MCP servers (Context7 doc search, Headroom token saving)
        // are always present, but never overwrite a user's edits to those names.
        config.mcp = config.mcp.with_builtins();
        Ok(config)
    }

    pub fn save(&self, config: &AppConfig) -> Result<()> {
        self.ensure_dirs()?;
        let raw = serde_yaml::to_string(config)?;
        fs::write(self.config_path(), raw)?;
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct EndpointImportFile {
    pub endpoints: Vec<EndpointConfig>,
    pub primary_endpoint: Option<String>,
}

/// A reusable provider preset defined in YAML. Custom providers are not new
/// adapter implementations; they describe how to configure an existing adapter
/// kind (OpenAI-compatible, Anthropic, or Ollama) for a specific gateway so
/// endpoints can be created from them with a single command.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct CustomProvider {
    pub name: String,
    /// Adapter family to reuse: `openai-compatible`, `anthropic`, or `ollama`.
    pub kind: CustomProviderKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// Optional override for the model listing path, e.g. `v1/models`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub models_path: Option<String>,
    /// Optional override for the chat path, e.g. `v1/chat/completions`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_path: Option<String>,
}

impl CustomProvider {
    /// Resolve this preset into the provider kind an endpoint should use.
    pub fn provider_kind(&self) -> ProviderKind {
        match self.kind {
            CustomProviderKind::Anthropic => ProviderKind::Anthropic,
            CustomProviderKind::Ollama => ProviderKind::OllamaLocal,
            CustomProviderKind::OpenAiCompatible => ProviderKind::OpenAiCompatible,
        }
    }

    /// Build an endpoint configuration from this preset.
    pub fn to_endpoint(&self, endpoint_name: impl Into<String>) -> EndpointConfig {
        let provider = self.provider_kind();
        let mut endpoint = EndpointConfig::new(endpoint_name, provider);
        if let Some(base_url) = self.base_url.as_ref() {
            endpoint.base_url = base_url.clone();
        }
        if let Some(api_key_env) = self.api_key_env.as_ref() {
            endpoint.api_key_env = Some(api_key_env.clone());
        }
        if let Some(default_model) = self.default_model.as_ref() {
            endpoint.default_model = Some(default_model.clone());
        }
        endpoint.custom_headers = self.headers.clone();
        if let Some(path) = self.models_path.as_ref() {
            endpoint
                .metadata
                .insert("models_path".to_string(), Value::from(path.clone()));
        }
        if let Some(path) = self.chat_path.as_ref() {
            endpoint
                .metadata
                .insert("chat_path".to_string(), Value::from(path.clone()));
        }
        endpoint
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum CustomProviderKind {
    #[default]
    OpenAiCompatible,
    Anthropic,
    Ollama,
}

impl CustomProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenAiCompatible => "open-ai-compatible",
            Self::Anthropic => "anthropic",
            Self::Ollama => "ollama",
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct McpConfig {
    #[serde(default)]
    pub servers: BTreeMap<String, McpServerConfig>,
}

impl McpConfig {
    /// Merge built-in server definitions (Context7 doc search and Headroom
    /// token saving) into the configured set without overwriting a user's
    /// customizations for those names.
    pub fn with_builtins(mut self) -> Self {
        for (name, server) in McpServerConfig::builtins() {
            self.servers.entry(name).or_insert(server);
        }
        self
    }
}

/// Configuration for a single Model Context Protocol server. Built-in entries
/// ship with Cntx Code so doc search and token saving work without manual
/// setup; users can add custom servers from YAML or the CLI.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct McpServerConfig {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub built_in: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl McpServerConfig {
    /// Built-in server definitions. These are present by default but are only
    /// spawned on demand (for `cntx mcp tools` or the future agent loop), so
    /// they never slow down normal prompts.
    pub fn builtins() -> Vec<(String, Self)> {
        let context7 = Self {
            name: "context7".to_string(),
            command: "npx".to_string(),
            args: vec!["-y".to_string(), "@upstash/context7-mcp@3.2.3".to_string()],
            env: BTreeMap::new(),
            enabled: true,
            url: None,
            built_in: true,
            description: Some("Built-in doc search (Context7)".to_string()),
        };
        let headroom = Self {
            name: "headroom".to_string(),
            command: "headroom".to_string(),
            args: vec!["mcp".to_string(), "serve".to_string()],
            env: BTreeMap::new(),
            enabled: true,
            url: None,
            built_in: true,
            description: Some("Built-in token saving (Headroom)".to_string()),
        };
        vec![
            ("context7".to_string(), context7),
            ("headroom".to_string(), headroom),
        ]
    }
}

fn default_true() -> bool {
    true
}

fn is_false(value: &bool) -> bool {
    !value
}

pub fn load_endpoint_import(path: &Path) -> Result<EndpointImportFile> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read endpoint import {}", path.display()))?;
    let import = serde_yaml::from_str(&raw)
        .with_context(|| format!("failed to parse endpoint import {}", path.display()))?;
    Ok(import)
}

#[derive(Clone, Debug, Deserialize)]
pub struct CustomProviderImportFile {
    #[serde(default)]
    pub providers: Vec<CustomProvider>,
}

pub fn load_custom_provider_import(path: &Path) -> Result<CustomProviderImportFile> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read provider import {}", path.display()))?;
    let import = serde_yaml::from_str(&raw)
        .with_context(|| format!("failed to parse provider import {}", path.display()))?;
    Ok(import)
}

/// A YAML file describing custom MCP servers to register.
#[derive(Clone, Debug, Deserialize)]
pub struct McpServerImportFile {
    pub servers: Vec<McpServerConfig>,
}

pub fn load_mcp_server_import(path: &Path) -> Result<McpServerImportFile> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read MCP import {}", path.display()))?;
    let import = serde_yaml::from_str(&raw)
        .with_context(|| format!("failed to parse MCP import {}", path.display()))?;
    Ok(import)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_round_trips() {
        let temp = tempfile::tempdir().unwrap();
        let store = ConfigStore::from_root(temp.path());
        let mut config = AppConfig {
            primary_endpoint: Some("work".to_string()),
            ..AppConfig::default()
        };
        config.endpoints.insert(
            "work".to_string(),
            EndpointConfig::new("work", ProviderKind::OpenAi),
        );

        store.save(&config).unwrap();
        let loaded = store.load().unwrap();

        assert_eq!(loaded.primary_endpoint.as_deref(), Some("work"));
        assert!(loaded.endpoints.contains_key("work"));
    }

    #[test]
    fn ollama_cloud_options_round_trip_when_present() {
        let temp = tempfile::tempdir().unwrap();
        let store = ConfigStore::from_root(temp.path());
        let mut config = AppConfig::default();
        let mut endpoint = EndpointConfig::new("cloud", ProviderKind::OllamaCloud);
        endpoint.ollama_cloud = Some(OllamaCloudOptions {
            plan: Some(OllamaCloudPlan::Pro),
            subscription_models: true,
        });
        config.endpoints.insert("cloud".to_string(), endpoint);

        store.save(&config).unwrap();
        let loaded = store.load().unwrap();
        let options = loaded
            .endpoints
            .get("cloud")
            .and_then(|endpoint| endpoint.ollama_cloud.as_ref())
            .unwrap();

        assert_eq!(options.plan, Some(OllamaCloudPlan::Pro));
        assert!(options.subscription_models);
    }
}
