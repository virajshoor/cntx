use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

use crate::config::{EndpointConfig, ProviderKind, RouteSize, RoutingConfig};
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
        let route_size = if estimated_tokens <= self.config.thresholds.small_prompt_tokens {
            RouteSize::Small
        } else if estimated_tokens <= self.config.thresholds.medium_prompt_tokens {
            RouteSize::Medium
        } else {
            RouteSize::Large
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

        let models: Vec<&'m ModelInfo> = models.into_iter().collect();

        // If only one model is available, auto mode always uses it. This keeps
        // a fresh setup with a single cached model working without hitting the
        // size-based fallback or a "model not found" error.
        if models.len() == 1 {
            let model = models[0];
            return Some(RouteDecision {
                endpoint: endpoint.name.clone(),
                model: model.id.clone(),
                route_size,
                estimated_tokens,
                reason: "only available model".to_string(),
            });
        }

        let default_model = endpoint.default_model.as_ref().or_else(|| {
            self.config
                .default_models
                .get(&endpoint.name)
                .or_else(|| self.config.default_models.get(endpoint.provider.as_str()))
        });

        let target_rank = family_rank(route_size);
        let mut seen_any = false;
        let mut default_available = false;
        let mut best_target: Option<&'m ModelInfo> = None;
        let mut best_any: Option<&'m ModelInfo> = None;

        for model in &models {
            seen_any = true;
            if default_model.is_some_and(|default| model.id == default.as_str()) {
                default_available = true;
            }
            if classify_model(&endpoint.provider, model) == target_rank {
                best_target = newer_model(best_target, model);
            }
            best_any = newer_model(best_any, model);
        }

        if let Some(default) = default_model {
            if !seen_any || default_available {
                return Some(RouteDecision {
                    endpoint: endpoint.name.clone(),
                    model: default.clone(),
                    route_size,
                    estimated_tokens,
                    reason: "endpoint default model".to_string(),
                });
            }
        }

        best_target.or(best_any).map(|model| RouteDecision {
            endpoint: endpoint.name.clone(),
            model: model.id.clone(),
            route_size,
            estimated_tokens,
            reason: "prompt length after optimization".to_string(),
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ModelRank {
    Small,
    Medium,
    Large,
    Unknown,
}

fn family_rank(route_size: RouteSize) -> ModelRank {
    match route_size {
        RouteSize::Small => ModelRank::Small,
        RouteSize::Medium => ModelRank::Medium,
        RouteSize::Large => ModelRank::Large,
    }
}

fn classify_model(provider: &ProviderKind, model: &ModelInfo) -> ModelRank {
    match provider {
        ProviderKind::Anthropic => classify_anthropic(&model.id),
        ProviderKind::OpenAi | ProviderKind::OpenAiCompatible => classify_openai_like(&model.id),
        ProviderKind::OllamaLocal => classify_ollama(model, false),
        ProviderKind::OllamaCloud => classify_ollama(model, true),
    }
}

fn classify_anthropic(id: &str) -> ModelRank {
    let id = id.to_lowercase();
    if id.contains("haiku") {
        ModelRank::Small
    } else if id.contains("sonnet") {
        ModelRank::Medium
    } else if id.contains("opus") {
        ModelRank::Large
    } else {
        ModelRank::Unknown
    }
}

fn classify_openai_like(id: &str) -> ModelRank {
    let id = id.to_lowercase();
    if id.contains("nano") || id.contains("mini") || id.contains("small") {
        ModelRank::Small
    } else if id.contains("pro") || id.contains("large") || id.contains("max") {
        ModelRank::Large
    } else {
        ModelRank::Medium
    }
}

fn classify_ollama(model: &ModelInfo, cloud: bool) -> ModelRank {
    if let Some(rank) = ollama_usage_rank(model) {
        return rank;
    }

    let id = model.id.to_lowercase();
    if id.contains("pro")
        || id.contains("ultra")
        || id.contains("max")
        || id.contains("extra-high")
        || id.contains("extra_high")
    {
        return ModelRank::Large;
    }
    if cloud && (id.contains("flash") || id.contains("mini") || id.contains("light")) {
        return ModelRank::Small;
    }

    if let Some(size) = model
        .metadata
        .get("details")
        .and_then(|value| value.get("parameter_size"))
        .and_then(|value| value.as_str())
        .and_then(parse_size_billions)
        .or_else(|| parse_size_billions(&model.id))
    {
        let small_cutoff = if cloud { 25.0 } else { 10.0 };
        if size <= small_cutoff {
            ModelRank::Small
        } else if size < 40.0 {
            ModelRank::Medium
        } else {
            ModelRank::Large
        }
    } else {
        ModelRank::Unknown
    }
}

fn ollama_usage_rank(model: &ModelInfo) -> Option<ModelRank> {
    ["usage", "usage_level", "usageLevel"]
        .iter()
        .find_map(|key| model.metadata.get(*key).and_then(|value| value.as_str()))
        .map(|usage| usage.to_lowercase())
        .and_then(|usage| {
            if usage.contains("extra") || usage.contains("high") {
                Some(ModelRank::Large)
            } else if usage.contains("medium") {
                Some(ModelRank::Medium)
            } else if usage.contains("low") || usage.contains("light") || usage.contains("small") {
                Some(ModelRank::Small)
            } else {
                None
            }
        })
}

fn parse_size_billions(value: &str) -> Option<f32> {
    let lower = value.to_lowercase();
    let (marker, multiplier) = lower
        .find('b')
        .map(|index| (index, 1.0))
        .or_else(|| lower.find('t').map(|index| (index, 1000.0)))?;
    let mut start = marker;
    for (index, ch) in lower[..marker].char_indices().rev() {
        if ch.is_ascii_digit() || ch == '.' {
            start = index;
        } else {
            break;
        }
    }
    lower[start..marker]
        .parse::<f32>()
        .ok()
        .map(|value| value * multiplier)
}

fn compare_models(a: &ModelInfo, b: &ModelInfo) -> Ordering {
    a.created_at
        .cmp(&b.created_at)
        .then_with(|| a.id.cmp(&b.id))
}

fn newer_model<'a>(
    current: Option<&'a ModelInfo>,
    candidate: &'a ModelInfo,
) -> Option<&'a ModelInfo> {
    match current {
        Some(current) if compare_models(current, candidate) != Ordering::Less => Some(current),
        _ => Some(candidate),
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
