use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config::{AppConfig, ConfigStore};
use crate::providers::{adapter_for, ModelInfo};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ModelCache {
    pub refreshed_at: Option<DateTime<Utc>>,
    pub endpoints: BTreeMap<String, CachedEndpointModels>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct CachedEndpointModels {
    pub provider: String,
    pub models: Vec<CachedModel>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CachedModel {
    pub info: ModelInfo,
    pub status: ModelStatus,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ModelStatus {
    Available,
    Deprecated,
}

#[derive(Clone, Debug, Default)]
pub struct RefreshReport {
    pub endpoints: Vec<EndpointRefreshReport>,
}

#[derive(Clone, Debug)]
pub struct EndpointRefreshReport {
    pub endpoint: String,
    pub added: Vec<String>,
    pub deprecated: Vec<String>,
    pub total_available: usize,
}

impl ModelCache {
    pub fn load(store: &ConfigStore) -> Result<Self> {
        let path = store.model_cache_path();
        if !path.exists() {
            return Ok(Self::default());
        }

        let raw = fs::read_to_string(&path)
            .with_context(|| format!("failed to read model cache {}", path.display()))?;
        let cache = serde_yaml::from_str(&raw)
            .with_context(|| format!("failed to parse model cache {}", path.display()))?;
        Ok(cache)
    }

    pub fn save(&self, store: &ConfigStore) -> Result<()> {
        store.ensure_dirs()?;
        fs::write(store.model_cache_path(), serde_yaml::to_string(self)?)?;
        Ok(())
    }

    pub fn available_for<'a>(&'a self, endpoint: &str) -> impl Iterator<Item = &'a ModelInfo> + 'a {
        self.endpoints
            .get(endpoint)
            .into_iter()
            .flat_map(|cached| cached.models.iter())
            .filter(|model| model.status == ModelStatus::Available)
            .map(|model| &model.info)
    }
}

pub async fn refresh_models(config: &AppConfig, store: &ConfigStore) -> Result<RefreshReport> {
    let mut cache = ModelCache::load(store)?;
    let mut report = RefreshReport::default();

    for endpoint in config.endpoints.values() {
        let previous = cache
            .endpoints
            .get(&endpoint.name)
            .map(|cached| {
                cached
                    .models
                    .iter()
                    .filter(|model| model.status == ModelStatus::Available)
                    .map(|model| model.info.id.clone())
                    .collect::<BTreeSet<_>>()
            })
            .unwrap_or_default();

        // Resolve the key from the endpoint, its env var, or the runtime
        // secrets store so `cntx api-key add` is enough for refresh too.
        let mut endpoint = endpoint.clone();
        if endpoint.resolved_api_key().is_none() {
            if let Some(key) = crate::api_keys::resolve_for_provider(store, &endpoint) {
                endpoint.api_key = Some(key);
            }
        }

        let adapter = adapter_for(endpoint.provider.clone());
        let fresh = adapter.list_models(&endpoint).await?;
        let fresh_ids = fresh
            .iter()
            .map(|model| model.id.clone())
            .collect::<BTreeSet<_>>();

        let added = fresh_ids
            .difference(&previous)
            .cloned()
            .collect::<Vec<String>>();
        let deprecated = previous
            .difference(&fresh_ids)
            .cloned()
            .collect::<Vec<String>>();

        let mut models = fresh
            .into_iter()
            .map(|info| CachedModel {
                info,
                status: ModelStatus::Available,
            })
            .collect::<Vec<_>>();

        for deprecated_id in &deprecated {
            models.push(CachedModel {
                info: ModelInfo::new(deprecated_id),
                status: ModelStatus::Deprecated,
            });
        }
        models.sort_by(|a, b| a.info.id.cmp(&b.info.id));

        let total_available = fresh_ids.len();
        cache.endpoints.insert(
            endpoint.name.clone(),
            CachedEndpointModels {
                provider: endpoint.provider.as_str().to_string(),
                models,
            },
        );
        report.endpoints.push(EndpointRefreshReport {
            endpoint: endpoint.name.clone(),
            added,
            deprecated,
            total_available,
        });
    }

    cache.refreshed_at = Some(Utc::now());
    cache.save(store)?;
    Ok(report)
}
