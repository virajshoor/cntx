use serde::{Deserialize, Serialize};

use crate::config::{EndpointConfig, RouteSize, RoutingConfig};
use crate::providers::ModelInfo;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RouteDecision {
    pub endpoint: String,
    pub model: String,
    pub route_size: RouteSize,
    pub estimated_tokens: usize,
    pub reason: String,
}

#[derive(Clone, Debug)]
pub struct ModelRouter<'a> {
    config: &'a RoutingConfig,
}

impl<'a> ModelRouter<'a> {
    pub fn new(config: &'a RoutingConfig) -> Self {
        Self { config }
    }

    pub fn route<'m>(
        &self,
        endpoint: &EndpointConfig,
        models: impl IntoIterator<Item = &'m ModelInfo>,
        estimated_tokens: usize,
    ) -> Option<RouteDecision> {
        // Size-tier classification lives in the C core so routing tiers and
        // context budgets use one rule set.
        let route_size = match crate::core::route_classify(
            estimated_tokens,
            self.config.thresholds.small_prompt_tokens,
            self.config.thresholds.medium_prompt_tokens,
        ) {
            0 => RouteSize::Small,
            1 => RouteSize::Medium,
            _ => RouteSize::Large,
        };

        if let Some(model) = self
            .config
            .family_overrides
            .get(&endpoint.name)
            .and_then(|mapping| mapping.get(&route_size))
        {
            return Some(RouteDecision {
                endpoint: endpoint.name.clone(),
                model: model.clone(),
                route_size,
                estimated_tokens,
                reason: "configured route override".to_string(),
            });
        }

        let models: Vec<&ModelInfo> = models.into_iter().collect();

        let default_model = endpoint.default_model.as_ref().or_else(|| {
            self.config
                .default_models
                .get(&endpoint.name)
                .or_else(|| self.config.default_models.get(endpoint.provider.as_str()))
        });

        // Ranking and selection live in the C core (`cntx_model_rank` +
        // `cntx_model_select`) so family classification, defaults, and
        // recency tie-breaks have one implementation.
        let target_rank = match route_size {
            RouteSize::Small => 0,
            RouteSize::Medium => 1,
            RouteSize::Large => 2,
        };
        let (model, reason) = crate::core::model_select(
            &endpoint.provider,
            &models,
            target_rank,
            default_model.map(String::as_str),
        )?;

        Some(RouteDecision {
            endpoint: endpoint.name.clone(),
            model,
            route_size,
            estimated_tokens,
            reason: reason.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::*;
    use crate::config::{EndpointConfig, ProviderKind, RoutingThresholds};

    #[test]
    fn single_available_model_is_always_selected() {
        let routing = RoutingConfig::default();
        let endpoint = EndpointConfig::new("work", ProviderKind::OllamaCloud);
        let only = model("deepseek-v4-flash:cloud");

        let decision = ModelRouter::new(&routing)
            .route(&endpoint, std::iter::once(&only), 5)
            .unwrap();

        assert_eq!(decision.model, "deepseek-v4-flash:cloud");
        assert_eq!(decision.reason, "only available model");
    }

    #[test]
    fn routes_by_optimized_prompt_length() {
        let routing = RoutingConfig {
            thresholds: RoutingThresholds {
                small_prompt_tokens: 10,
                medium_prompt_tokens: 100,
            },
            ..RoutingConfig::default()
        };
        let endpoint = EndpointConfig::new("work", ProviderKind::Anthropic);
        let models = [
            model("claude-haiku-1"),
            model("claude-sonnet-2"),
            model("claude-opus-3"),
        ];

        let router = ModelRouter::new(&routing);
        assert_eq!(
            router.route(&endpoint, models.iter(), 5).unwrap().model,
            "claude-haiku-1"
        );
        assert_eq!(
            router.route(&endpoint, models.iter(), 50).unwrap().model,
            "claude-sonnet-2"
        );
        assert_eq!(
            router.route(&endpoint, models.iter(), 500).unwrap().model,
            "claude-opus-3"
        );
    }

    #[test]
    fn routing_selects_without_candidate_allocation_and_prefers_newest() {
        let routing = RoutingConfig::default();
        let endpoint = EndpointConfig::new("work", ProviderKind::OpenAi);
        let mut older = model("gpt-older-mini");
        older.created_at = Some(Utc.timestamp_opt(1, 0).unwrap());
        let mut newer = model("gpt-newer-mini");
        newer.created_at = Some(Utc.timestamp_opt(2, 0).unwrap());
        let models = [older, newer];

        let decision = ModelRouter::new(&routing)
            .route(&endpoint, models.iter(), 10)
            .unwrap();

        assert_eq!(decision.model, "gpt-newer-mini");
    }

    #[test]
    fn routes_ollama_cloud_subscription_models_as_large() {
        let routing = RoutingConfig {
            thresholds: RoutingThresholds {
                small_prompt_tokens: 10,
                medium_prompt_tokens: 100,
            },
            ..RoutingConfig::default()
        };
        let endpoint = EndpointConfig::new("ollama-pro", ProviderKind::OllamaCloud);
        let models = [model("gpt-oss:20b-cloud"), model("deepseek-v4-pro:cloud")];

        let decision = ModelRouter::new(&routing)
            .route(&endpoint, models.iter(), 500)
            .unwrap();

        assert_eq!(decision.model, "deepseek-v4-pro:cloud");
    }

    #[test]
    fn parses_trillion_parameter_ollama_sizes() {
        let routing = RoutingConfig {
            thresholds: RoutingThresholds {
                small_prompt_tokens: 10,
                medium_prompt_tokens: 100,
            },
            ..RoutingConfig::default()
        };
        let endpoint = EndpointConfig::new("ollama-pro", ProviderKind::OllamaCloud);
        let mut giant = model("custom-cloud-model");
        giant.metadata.insert(
            "details".to_string(),
            serde_json::json!({ "parameter_size": "1.6T" }),
        );
        let models = [model("gpt-oss:20b-cloud"), giant];

        let decision = ModelRouter::new(&routing)
            .route(&endpoint, models.iter(), 500)
            .unwrap();

        assert_eq!(decision.model, "custom-cloud-model");
    }

    fn model(id: &str) -> ModelInfo {
        let mut model = ModelInfo::new(id);
        model.created_at = Some(Utc::now());
        model
    }
}
