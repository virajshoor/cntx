use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

use crate::config::{EndpointConfig, ProviderKind};
use crate::providers::ModelInfo;

const COUNSEL_EVALUATION_CHAR_BUDGET: usize = 6 * 1024;
const COUNSEL_NOTE_CHAR_BUDGET: usize = 2 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CounselTask {
    Evaluate,
    SmallChange,
    Refactor,
}

impl CounselTask {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Evaluate => "evaluate",
            Self::SmallChange => "small-change",
            Self::Refactor => "refactor",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CounselPlan {
    pub evaluator_model: String,
    pub worker_model: String,
    pub task: CounselTask,
}

pub fn classify_counsel_task(prompt: &str) -> CounselTask {
    let prompt = prompt.to_lowercase();
    if contains_any(
        &prompt,
        &[
            "refactor",
            "restructure",
            "rewrite",
            "architecture",
            "extract",
            "split",
            "modularize",
            "redesign",
        ],
    ) {
        CounselTask::Refactor
    } else if contains_any(
        &prompt,
        &[
            "fix",
            "change",
            "add",
            "implement",
            "update",
            "modify",
            "patch",
            "build",
        ],
    ) {
        CounselTask::SmallChange
    } else {
        CounselTask::Evaluate
    }
}

pub fn plan_counsel<'a>(
    endpoint: &EndpointConfig,
    models: impl IntoIterator<Item = &'a ModelInfo>,
    prompt: &str,
) -> Option<CounselPlan> {
    let task = classify_counsel_task(prompt);
    let mut candidates = CounselCandidates::default();

    for model in models {
        candidates.consider(endpoint.provider.clone(), model);
    }

    let evaluator_model = model_id(
        candidates
            .evaluator
            .as_ref()
            .or(candidates.small_change.as_ref())
            .or(candidates.any.as_ref())?,
    );
    let worker_model = match task {
        CounselTask::Evaluate => evaluator_model.clone(),
        CounselTask::SmallChange => model_id(
            candidates
                .small_change
                .as_ref()
                .or(candidates.evaluator.as_ref())
                .or(candidates.any.as_ref())?,
        ),
        CounselTask::Refactor => model_id(
            candidates
                .refactor
                .as_ref()
                .or(candidates.small_change.as_ref())
                .or(candidates.any.as_ref())?,
        ),
    };

    Some(CounselPlan {
        evaluator_model,
        worker_model,
        task,
    })
}

pub fn build_evaluation_prompt(optimized_prompt: &str, estimated_tokens: usize) -> String {
    format!(
        "You are the Counsel evaluator. Be concise and token efficient.\n\
         Classify the request as evaluate, small-change, or refactor. Then list only:\n\
         - risk\n\
         - likely files or concepts\n\
         - smallest useful next action\n\
         Do not solve the whole task.\n\
         Optimized prompt estimate: {estimated_tokens} tokens.\n\n\
         Prompt preview:\n{}",
        bounded_prompt_preview(optimized_prompt, COUNSEL_EVALUATION_CHAR_BUDGET)
    )
}

pub fn build_worker_prompt(optimized_prompt: &str, evaluation: &str, task: CounselTask) -> String {
    format!(
        "Counsel mode selected task: {}.\n\
         Use the evaluator note only as guidance. Keep the answer focused and avoid repeating context.\n\
         Evaluator note:\n{}\n\n\
         User request:\n{}",
        task.as_str(),
        bounded_prompt_preview(evaluation, COUNSEL_NOTE_CHAR_BUDGET),
        optimized_prompt
    )
}

fn bounded_prompt_preview(prompt: &str, budget: usize) -> String {
    let char_count = prompt.chars().count();
    if char_count <= budget {
        return prompt.to_string();
    }

    let half = budget.saturating_sub(32) / 2;
    let head = prompt.chars().take(half).collect::<String>();
    let tail = prompt
        .chars()
        .rev()
        .take(half)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    format!("{head}\n...[truncated for token efficiency]...\n{tail}")
}

#[derive(Clone, Default)]
struct CounselCandidates {
    evaluator: Option<ModelInfo>,
    small_change: Option<ModelInfo>,
    refactor: Option<ModelInfo>,
    any: Option<ModelInfo>,
}

impl CounselCandidates {
    fn consider(&mut self, provider: ProviderKind, model: &ModelInfo) {
        self.any = newer_model(self.any.take(), model);
        match counsel_role(provider, model) {
            CounselRole::Evaluator => self.evaluator = newer_model(self.evaluator.take(), model),
            CounselRole::SmallChange => {
                self.small_change = newer_model(self.small_change.take(), model)
            }
            CounselRole::Refactor => self.refactor = newer_model(self.refactor.take(), model),
            CounselRole::Unknown => {}
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CounselRole {
    Evaluator,
    SmallChange,
    Refactor,
    Unknown,
}

fn counsel_role(provider: ProviderKind, model: &ModelInfo) -> CounselRole {
    match provider {
        ProviderKind::Anthropic => anthropic_role(&model.id),
        ProviderKind::OpenAi | ProviderKind::OpenAiCompatible => openai_like_role(&model.id),
        ProviderKind::OllamaLocal | ProviderKind::OllamaCloud => ollama_role(&model.id, model),
    }
}

fn anthropic_role(id: &str) -> CounselRole {
    let id = id.to_lowercase();
    if id.contains("haiku") {
        CounselRole::Evaluator
    } else if id.contains("sonnet") {
        CounselRole::SmallChange
    } else if id.contains("opus") {
        CounselRole::Refactor
    } else {
        CounselRole::Unknown
    }
}

fn openai_like_role(id: &str) -> CounselRole {
    let id = id.to_lowercase();
    if contains_any(&id, &["nano", "mini", "small"]) {
        CounselRole::Evaluator
    } else if contains_any(&id, &["pro", "large", "max"]) {
        CounselRole::Refactor
    } else {
        CounselRole::SmallChange
    }
}

fn ollama_role(id: &str, model: &ModelInfo) -> CounselRole {
    let id = id.to_lowercase();
    if contains_any(&id, &["pro", "ultra", "max"]) {
        return CounselRole::Refactor;
    }
    if let Some(size) = model
        .metadata
        .get("details")
        .and_then(|value| value.get("parameter_size"))
        .and_then(|value| value.as_str())
        .and_then(parse_size_billions)
        .or_else(|| parse_size_billions(&id))
    {
        if size <= 25.0 {
            CounselRole::Evaluator
        } else if size < 100.0 {
            CounselRole::SmallChange
        } else {
            CounselRole::Refactor
        }
    } else if contains_any(&id, &["flash", "mini", "light"]) {
        CounselRole::Evaluator
    } else {
        CounselRole::SmallChange
    }
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

fn newer_model(current: Option<ModelInfo>, candidate: &ModelInfo) -> Option<ModelInfo> {
    match current {
        Some(current) if compare_models(&current, candidate) != Ordering::Less => Some(current),
        _ => Some(candidate.clone()),
    }
}

fn model_id(model: &ModelInfo) -> String {
    model.id.clone()
}

fn compare_models(a: &ModelInfo, b: &ModelInfo) -> Ordering {
    a.created_at
        .cmp(&b.created_at)
        .then_with(|| a.id.cmp(&b.id))
}

fn contains_any(value: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| value.contains(needle))
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::*;
    use crate::config::{EndpointConfig, ProviderKind};

    #[test]
    fn classifies_counsel_tasks() {
        assert_eq!(
            classify_counsel_task("review this module"),
            CounselTask::Evaluate
        );
        assert_eq!(
            classify_counsel_task("fix the endpoint bug"),
            CounselTask::SmallChange
        );
        assert_eq!(
            classify_counsel_task("refactor the provider architecture"),
            CounselTask::Refactor
        );
    }

    #[test]
    fn plans_anthropic_counsel_roles() {
        let endpoint = EndpointConfig::new("anthropic", ProviderKind::Anthropic);
        let models = [
            dated_model("claude-3-haiku", 1),
            dated_model("claude-3-sonnet", 2),
            dated_model("claude-3-opus", 3),
        ];

        let small = plan_counsel(&endpoint, models.iter(), "fix the typo").unwrap();
        let refactor = plan_counsel(&endpoint, models.iter(), "refactor modules").unwrap();

        assert_eq!(small.evaluator_model, "claude-3-haiku");
        assert_eq!(small.worker_model, "claude-3-sonnet");
        assert_eq!(refactor.worker_model, "claude-3-opus");
    }

    #[test]
    fn plans_ollama_cloud_counsel_roles() {
        let endpoint = EndpointConfig::new("ollama-pro", ProviderKind::OllamaCloud);
        let models = [
            dated_model("gpt-oss:20b-cloud", 1),
            dated_model("gpt-oss:120b-cloud", 2),
            dated_model("deepseek-v4-pro:cloud", 3),
        ];

        let plan = plan_counsel(&endpoint, models.iter(), "refactor provider system").unwrap();

        assert_eq!(plan.evaluator_model, "gpt-oss:20b-cloud");
        assert_eq!(plan.worker_model, "deepseek-v4-pro:cloud");
    }

    #[test]
    fn evaluation_prompt_is_bounded() {
        let prompt = "a".repeat(COUNSEL_EVALUATION_CHAR_BUDGET * 2);
        let evaluation = build_evaluation_prompt(&prompt, 10_000);

        assert!(evaluation.len() < COUNSEL_EVALUATION_CHAR_BUDGET + 700);
        assert!(evaluation.contains("truncated for token efficiency"));
    }

    fn dated_model(id: &str, seconds: i64) -> ModelInfo {
        let mut model = ModelInfo::new(id);
        model.created_at = Some(Utc.timestamp_opt(seconds, 0).unwrap());
        model
    }
}
