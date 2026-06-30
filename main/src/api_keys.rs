//! Runtime API key storage.
//!
//! API keys never live in source. They are stored in a gitignored runtime
//! secrets file inside the user config directory (`secrets.yaml`) that is
//! created automatically on first boot. Keys can be managed with
//! `cntx api-key --add/--change/--delete/--list` and are resolved at request
//! time for any endpoint whose own `api_key`/`api_key_env` is not set.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::{ConfigStore, ProviderKind};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ApiSecrets {
    #[serde(default)]
    pub keys: BTreeMap<String, String>,
}

impl ApiSecrets {
    pub fn get(&self, provider: &str) -> Option<&str> {
        self.keys.get(provider).map(String::as_str)
    }

    pub fn providers(&self) -> Vec<String> {
        self.keys.keys().cloned().collect()
    }

    /// Returns the last four characters of the key for safe display, or
    /// `<empty>` when the key is too short to mask meaningfully.
    pub fn masked(provider: &str, key: &str) -> String {
        let tail = key.chars().rev().take(4).collect::<Vec<_>>();
        if tail.len() < 4 || key.len() < 8 {
            format!("{provider}: <hidden>")
        } else {
            let tail: String = tail.into_iter().rev().collect();
            format!("{provider}: ****{tail}")
        }
    }
}

/// Read the runtime secrets file, returning an empty store when it does not
/// exist yet. The caller is responsible for creating it on first boot via
/// [`ensure_secrets_file`].
pub fn load(store: &ConfigStore) -> Result<ApiSecrets> {
    let path = store.secrets_path();
    if !path.exists() {
        return Ok(ApiSecrets::default());
    }
    let raw = fs::read_to_string(&path)
        .with_context(|| format!("failed to read secrets file {}", path.display()))?;
    let secrets = serde_yaml::from_str(&raw)
        .with_context(|| format!("failed to parse secrets file {}", path.display()))?;
    Ok(secrets)
}

/// Create an empty secrets file with restrictive permissions when missing.
/// This runs on first boot so the tool is ready to receive keys without any
/// manual setup, and on any machine after a fresh build.
pub fn ensure_secrets_file(store: &ConfigStore) -> Result<()> {
    store.ensure_dirs()?;
    let path = store.secrets_path();
    if path.exists() {
        return Ok(());
    }
    fs::write(&path, "keys: {}\n")?;
    restrict_permissions(&path)
}

pub fn save(store: &ConfigStore, secrets: &ApiSecrets) -> Result<()> {
    store.ensure_dirs()?;
    let path = store.secrets_path();
    let raw = serde_yaml::to_string(secrets)?;
    fs::write(&path, raw)?;
    restrict_permissions(&path)
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let metadata = fs::metadata(path)?;
    let mut permissions = metadata.permissions();
    permissions.set_mode(0o600);
    fs::set_permissions(path, permissions)?;
    Ok(())
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

/// Add or replace a key for a provider label.
pub fn add(store: &ConfigStore, provider: &str, key: &str) -> Result<ApiSecrets> {
    let mut secrets = load(store)?;
    secrets.keys.insert(provider.to_string(), key.to_string());
    save(store, &secrets)?;
    Ok(secrets)
}

/// Remove a provider key, returning whether one was present.
pub fn remove(store: &ConfigStore, provider: &str) -> Result<bool> {
    let mut secrets = load(store)?;
    let existed = secrets.keys.remove(provider).is_some();
    if existed {
        save(store, &secrets)?;
    }
    Ok(existed)
}

/// Resolve a key for a provider kind, falling back through the endpoint's
/// configured env var. Returns `None` when no key is available anywhere.
pub fn resolve_for_provider(
    store: &ConfigStore,
    endpoint: &crate::config::EndpointConfig,
) -> Option<String> {
    if let Some(key) = endpoint.resolved_api_key() {
        return Some(key);
    }
    let secrets = load(store).ok()?;
    secrets
        .get(endpoint.provider.as_str())
        .map(ToOwned::to_owned)
}

/// Normalize a provider label accepted on the CLI into the canonical key used
/// in the secrets store. Accepts both kebab-case provider kinds and custom
/// provider names.
pub fn canonical_provider_label(label: &str) -> String {
    label.trim().to_lowercase()
}

/// Provider labels a user is likely to add keys for, for help text.
pub fn well_known_providers() -> Vec<&'static str> {
    vec![
        ProviderKind::OpenAi.as_str(),
        ProviderKind::Anthropic.as_str(),
        ProviderKind::OpenAiCompatible.as_str(),
        ProviderKind::OllamaCloud.as_str(),
        ProviderKind::OllamaLocal.as_str(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ConfigStore, EndpointConfig, ProviderKind};

    #[test]
    fn secrets_round_trip_and_mask() {
        let temp = tempfile::tempdir().unwrap();
        let store = ConfigStore::from_root(temp.path());
        ensure_secrets_file(&store).unwrap();
        assert!(store.secrets_path().exists());

        add(&store, "anthropic", "sk-ant-1234567890").unwrap();
        let secrets = load(&store).unwrap();
        assert_eq!(secrets.get("anthropic"), Some("sk-ant-1234567890"));

        let display = ApiSecrets::masked("anthropic", "sk-ant-1234567890");
        assert!(display.contains("****7890"));
        assert!(!display.contains("1234567890"));
    }

    #[test]
    fn remove_returns_existence() {
        let temp = tempfile::tempdir().unwrap();
        let store = ConfigStore::from_root(temp.path());
        ensure_secrets_file(&store).unwrap();

        add(&store, "openai", "sk-test").unwrap();
        assert!(remove(&store, "openai").unwrap());
        assert!(!remove(&store, "openai").unwrap());
    }

    #[test]
    fn resolve_falls_back_to_secrets_when_endpoint_has_no_key() {
        let temp = tempfile::tempdir().unwrap();
        let store = ConfigStore::from_root(temp.path());
        ensure_secrets_file(&store).unwrap();
        add(&store, "anthropic", "sk-from-secrets").unwrap();

        let endpoint = EndpointConfig::new("work", ProviderKind::Anthropic);
        let key = resolve_for_provider(&store, &endpoint).unwrap();
        assert_eq!(key, "sk-from-secrets");
    }

    #[test]
    fn short_keys_are_not_partially_revealed() {
        let display = ApiSecrets::masked("ollama-local", "abc");
        assert_eq!(display, "ollama-local: <hidden>");
    }
}
