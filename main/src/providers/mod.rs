use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::config::{EndpointConfig, ProviderKind};
use crate::errors::CntxError;

const MAX_METADATA_STRING_CHARS: usize = 256;
const MAX_METADATA_ARRAY_ITEMS: usize = 8;
const MAX_METADATA_OBJECT_KEYS: usize = 16;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ModelInfo {
    pub id: String,
    pub display_name: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub owned_by: Option<String>,
    pub family: Option<String>,
    pub context_window: Option<usize>,
    pub metadata: BTreeMap<String, Value>,
}

impl ModelInfo {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            display_name: None,
            created_at: None,
            owned_by: None,
            family: None,
            context_window: None,
            metadata: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Clone, Debug)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub max_tokens: Option<usize>,
}

#[async_trait]
pub trait ProviderAdapter: Send + Sync {
    async fn list_models(&self, endpoint: &EndpointConfig) -> Result<Vec<ModelInfo>>;

    async fn stream_chat(
        &self,
        endpoint: &EndpointConfig,
        request: ChatRequest,
        on_delta: &mut (dyn FnMut(String) + Send),
    ) -> Result<()>;
}

pub fn adapter_for(provider: ProviderKind) -> Box<dyn ProviderAdapter> {
    match provider {
        ProviderKind::OpenAi => Box::new(OpenAiLikeAdapter::new(ProviderKind::OpenAi)),
        ProviderKind::OpenAiCompatible => {
            Box::new(OpenAiLikeAdapter::new(ProviderKind::OpenAiCompatible))
        }
        ProviderKind::Anthropic => Box::new(AnthropicAdapter),
        ProviderKind::OllamaLocal => Box::new(OllamaAdapter::new(false)),
        ProviderKind::OllamaCloud => Box::new(OllamaAdapter::new(true)),
    }
}

/// Maximum retry attempts for transient provider errors.
const MAX_RETRIES: u32 = 3;

/// Initial backoff delay in milliseconds.
const INITIAL_BACKOFF_MS: u64 = 1_000;

/// Stream a chat request with retry/backoff on transient errors (429, 500,
/// 502, 503, 504, connection failures). Non-retryable errors are returned
/// immediately.
pub async fn stream_chat_with_retry(
    adapter: &dyn ProviderAdapter,
    endpoint: &EndpointConfig,
    request: ChatRequest,
    on_delta: &mut (dyn FnMut(String) + Send),
) -> Result<()> {
    let mut backoff = INITIAL_BACKOFF_MS;
    for attempt in 0..=MAX_RETRIES {
        match adapter
            .stream_chat(endpoint, request.clone(), on_delta)
            .await
        {
            Ok(()) => return Ok(()),
            Err(e) if attempt < MAX_RETRIES && is_retryable_error(&e) => {
                eprintln!(
                    "  retrying in {}s (attempt {}/{})",
                    backoff / 1000,
                    attempt + 1,
                    MAX_RETRIES
                );
                tokio::time::sleep(Duration::from_millis(backoff)).await;
                backoff *= 2;
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Returns true when the error looks like a transient provider issue worth
/// retrying (rate limit, server error, connection timeout).
fn is_retryable_error(error: &anyhow::Error) -> bool {
    let msg = error.to_string().to_lowercase();
    msg.contains("429")
        || msg.contains("rate limit")
        || msg.contains("500")
        || msg.contains("502")
        || msg.contains("503")
        || msg.contains("504")
        || msg.contains("internal server error")
        || msg.contains("bad gateway")
        || msg.contains("service unavailable")
        || msg.contains("gateway timeout")
        || msg.contains("connection")
        || msg.contains("timeout")
        || msg.contains("timed out")
        || msg.contains("reset")
}

fn client(endpoint: &EndpointConfig) -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(endpoint.timeout_secs))
        .build()
        .context("failed to build HTTP client")
}

fn join_url(base_url: &str, path: &str) -> String {
    format!(
        "{}/{}",
        base_url.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

/// Resolve a request path for an endpoint, allowing custom providers to
/// override the default path via `metadata[<key>]`.
fn endpoint_path(endpoint: &EndpointConfig, key: &str, default: &str) -> String {
    endpoint
        .metadata
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or(default)
        .to_string()
}

fn headers(endpoint: &EndpointConfig, provider: &ProviderKind) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

    match provider {
        ProviderKind::OpenAi | ProviderKind::OpenAiCompatible | ProviderKind::OllamaCloud => {
            if let Some(key) = endpoint.resolved_api_key() {
                headers.insert(
                    AUTHORIZATION,
                    HeaderValue::from_str(&format!("Bearer {key}"))
                        .context("invalid authorization header")?,
                );
            } else if provider.requires_key_by_default() {
                return Err(CntxError::MissingApiKey(endpoint.name.clone()).into());
            }
        }
        ProviderKind::Anthropic => {
            let key = endpoint
                .resolved_api_key()
                .ok_or_else(|| CntxError::MissingApiKey(endpoint.name.clone()))?;
            headers.insert("x-api-key", HeaderValue::from_str(&key)?);
            headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
        }
        ProviderKind::OllamaLocal => {
            if let Some(key) = endpoint.resolved_api_key() {
                headers.insert(
                    AUTHORIZATION,
                    HeaderValue::from_str(&format!("Bearer {key}"))?,
                );
            }
        }
    }

    for (key, value) in &endpoint.custom_headers {
        headers.insert(
            HeaderName::from_bytes(key.as_bytes())?,
            HeaderValue::from_str(value)?,
        );
    }
    Ok(headers)
}

pub struct OpenAiLikeAdapter {
    provider: ProviderKind,
}

impl OpenAiLikeAdapter {
    fn new(provider: ProviderKind) -> Self {
        Self { provider }
    }
}

#[async_trait]
impl ProviderAdapter for OpenAiLikeAdapter {
    async fn list_models(&self, endpoint: &EndpointConfig) -> Result<Vec<ModelInfo>> {
        let response = client(endpoint)?
            .get(join_url(
                &endpoint.base_url,
                &endpoint_path(endpoint, "models_path", "models"),
            ))
            .headers(headers(endpoint, &self.provider)?)
            .send()
            .await?
            .error_for_status()?
            .json::<Value>()
            .await?;

        parse_openai_models(&response)
    }

    async fn stream_chat(
        &self,
        endpoint: &EndpointConfig,
        request: ChatRequest,
        on_delta: &mut (dyn FnMut(String) + Send),
    ) -> Result<()> {
        let messages: Vec<Value> = request
            .messages
            .iter()
            .map(|message| json!({ "role": message.role, "content": message.content }))
            .collect();
        let mut body = json!({
            "model": request.model,
            "messages": messages,
            "stream": true,
        });
        if let Some(max_tokens) = request.max_tokens {
            body["max_tokens"] = json!(max_tokens);
        }

        let mut stream = client(endpoint)?
            .post(join_url(
                &endpoint.base_url,
                &endpoint_path(endpoint, "chat_path", "chat/completions"),
            ))
            .headers(headers(endpoint, &self.provider)?)
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .bytes_stream();

        let mut pending = String::new();
        while let Some(chunk) = stream.next().await {
            pending.push_str(&String::from_utf8_lossy(&chunk?));
            consume_sse(&mut pending, |data| {
                if data == "[DONE]" {
                    return;
                }
                if let Ok(value) = serde_json::from_str::<Value>(data) {
                    if let Some(content) = value
                        .pointer("/choices/0/delta/content")
                        .and_then(Value::as_str)
                    {
                        on_delta(content.to_string());
                    }
                }
            });
        }
        Ok(())
    }
}

pub struct AnthropicAdapter;

#[async_trait]
impl ProviderAdapter for AnthropicAdapter {
    async fn list_models(&self, endpoint: &EndpointConfig) -> Result<Vec<ModelInfo>> {
        let response = client(endpoint)?
            .get(join_url(
                &endpoint.base_url,
                &endpoint_path(endpoint, "models_path", "models"),
            ))
            .headers(headers(endpoint, &ProviderKind::Anthropic)?)
            .send()
            .await?
            .error_for_status()?
            .json::<Value>()
            .await?;

        parse_anthropic_models(&response)
    }

    async fn stream_chat(
        &self,
        endpoint: &EndpointConfig,
        request: ChatRequest,
        on_delta: &mut (dyn FnMut(String) + Send),
    ) -> Result<()> {
        let messages: Vec<Value> = request
            .messages
            .iter()
            .filter(|message| message.role != "system")
            .map(|message| json!({ "role": message.role, "content": message.content }))
            .collect();
        let system = request
            .messages
            .iter()
            .find(|message| message.role == "system")
            .map(|message| message.content.clone());
        let mut body = json!({
            "model": request.model,
            "messages": messages,
            "max_tokens": request.max_tokens.unwrap_or(4096),
            "stream": true,
        });
        if let Some(system) = system {
            body["system"] = Value::String(system);
        }

        let mut stream = client(endpoint)?
            .post(join_url(
                &endpoint.base_url,
                &endpoint_path(endpoint, "chat_path", "messages"),
            ))
            .headers(headers(endpoint, &ProviderKind::Anthropic)?)
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .bytes_stream();

        let mut pending = String::new();
        while let Some(chunk) = stream.next().await {
            pending.push_str(&String::from_utf8_lossy(&chunk?));
            consume_sse(&mut pending, |data| {
                if let Ok(value) = serde_json::from_str::<Value>(data) {
                    if let Some(content) = value.pointer("/delta/text").and_then(Value::as_str) {
                        on_delta(content.to_string());
                    }
                }
            });
        }
        Ok(())
    }
}

pub struct OllamaAdapter {
    cloud: bool,
}

impl OllamaAdapter {
    fn new(cloud: bool) -> Self {
        Self { cloud }
    }
}

#[async_trait]
impl ProviderAdapter for OllamaAdapter {
    async fn list_models(&self, endpoint: &EndpointConfig) -> Result<Vec<ModelInfo>> {
        let provider = if self.cloud {
            ProviderKind::OllamaCloud
        } else {
            ProviderKind::OllamaLocal
        };
        let response = client(endpoint)?
            .get(join_url(
                &endpoint.base_url,
                &endpoint_path(endpoint, "models_path", "api/tags"),
            ))
            .headers(headers(endpoint, &provider)?)
            .send()
            .await?
            .error_for_status()?
            .json::<Value>()
            .await?;

        parse_ollama_models(&response)
    }

    async fn stream_chat(
        &self,
        endpoint: &EndpointConfig,
        request: ChatRequest,
        on_delta: &mut (dyn FnMut(String) + Send),
    ) -> Result<()> {
        let provider = if self.cloud {
            ProviderKind::OllamaCloud
        } else {
            ProviderKind::OllamaLocal
        };
        let messages: Vec<Value> = request
            .messages
            .iter()
            .map(|message| json!({ "role": message.role, "content": message.content }))
            .collect();
        let mut body = json!({
            "model": request.model,
            "messages": messages,
            "stream": true,
        });
        if let Some(max_tokens) = request.max_tokens {
            body["options"] = json!({ "num_predict": max_tokens });
        }

        let mut stream = client(endpoint)?
            .post(join_url(
                &endpoint.base_url,
                &endpoint_path(endpoint, "chat_path", "api/chat"),
            ))
            .headers(headers(endpoint, &provider)?)
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .bytes_stream();

        let mut pending = String::new();
        while let Some(chunk) = stream.next().await {
            pending.push_str(&String::from_utf8_lossy(&chunk?));
            consume_lines(&mut pending, |line| {
                if let Ok(value) = serde_json::from_str::<Value>(line) {
                    if let Some(content) = value.pointer("/message/content").and_then(Value::as_str)
                    {
                        on_delta(content.to_string());
                    }
                }
            });
        }
        Ok(())
    }
}

pub fn parse_openai_models(value: &Value) -> Result<Vec<ModelInfo>> {
    let data = value
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| CntxError::UnsupportedProviderResponse("openai".to_string()))?;
    let mut models = Vec::with_capacity(data.len());
    for item in data {
        let Some(id) = item.get("id").and_then(Value::as_str) else {
            continue;
        };
        let mut model = ModelInfo::new(id);
        model.created_at = item
            .get("created")
            .and_then(Value::as_i64)
            .and_then(|seconds| Utc.timestamp_opt(seconds, 0).single());
        model.owned_by = item
            .get("owned_by")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        model.metadata = compact_metadata(item);
        models.push(model);
    }
    Ok(models)
}

pub fn parse_anthropic_models(value: &Value) -> Result<Vec<ModelInfo>> {
    let data = value
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| CntxError::UnsupportedProviderResponse("anthropic".to_string()))?;
    let mut models = Vec::with_capacity(data.len());
    for item in data {
        let Some(id) = item.get("id").and_then(Value::as_str) else {
            continue;
        };
        let mut model = ModelInfo::new(id);
        model.display_name = item
            .get("display_name")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        model.created_at = item
            .get("created_at")
            .and_then(Value::as_str)
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&Utc));
        model.metadata = compact_metadata(item);
        models.push(model);
    }
    Ok(models)
}

pub fn parse_ollama_models(value: &Value) -> Result<Vec<ModelInfo>> {
    let data = value
        .get("models")
        .and_then(Value::as_array)
        .ok_or_else(|| CntxError::UnsupportedProviderResponse("ollama".to_string()))?;
    let mut models = Vec::with_capacity(data.len());
    for item in data {
        let id = item
            .get("model")
            .or_else(|| item.get("name"))
            .and_then(Value::as_str);
        let Some(id) = id else {
            continue;
        };
        let mut model = ModelInfo::new(id);
        model.display_name = item
            .get("name")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        model.created_at = item
            .get("modified_at")
            .and_then(Value::as_str)
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&Utc));
        model.family = item
            .pointer("/details/family")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        model.metadata = compact_metadata(item);
        models.push(model);
    }
    Ok(models)
}

fn compact_metadata(value: &Value) -> BTreeMap<String, Value> {
    value
        .as_object()
        .map(|object| {
            object
                .iter()
                .take(MAX_METADATA_OBJECT_KEYS)
                .map(|(key, value)| (key.clone(), compact_value(value, 0)))
                .collect()
        })
        .unwrap_or_default()
}

fn compact_value(value: &Value, depth: usize) -> Value {
    match value {
        Value::String(value) => {
            Value::String(value.chars().take(MAX_METADATA_STRING_CHARS).collect())
        }
        Value::Array(values) if depth < 2 => Value::Array(
            values
                .iter()
                .take(MAX_METADATA_ARRAY_ITEMS)
                .map(|value| compact_value(value, depth + 1))
                .collect(),
        ),
        Value::Object(values) if depth < 2 => Value::Object(
            values
                .iter()
                .take(MAX_METADATA_OBJECT_KEYS)
                .map(|(key, value)| (key.clone(), compact_value(value, depth + 1)))
                .collect(),
        ),
        Value::Array(_) | Value::Object(_) => Value::Null,
        _ => value.clone(),
    }
}

fn consume_sse(pending: &mut String, mut on_data: impl FnMut(&str)) {
    while let Some(index) = pending.find("\n\n") {
        {
            let event = &pending[..index];
            for line in event.lines() {
                if let Some(data) = line.strip_prefix("data:") {
                    on_data(data.trim());
                }
            }
        }
        pending.drain(..index + 2);
    }
}

fn consume_lines(pending: &mut String, mut on_line: impl FnMut(&str)) {
    while let Some(index) = pending.find('\n') {
        {
            let line = pending[..index].trim();
            if !line.is_empty() {
                on_line(line);
            }
        }
        pending.drain(..index + 1);
    }
}

pub fn validate_chat_request(request: &ChatRequest) -> Result<()> {
    if request.model.trim().is_empty() {
        return Err(anyhow!("chat request model cannot be empty"));
    }
    if request.messages.is_empty() {
        return Err(anyhow!("chat request must include at least one message"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_openai_model_list() {
        let raw = json!({
            "object": "list",
            "data": [{ "id": "gpt-test", "created": 1, "owned_by": "openai" }]
        });

        let models = parse_openai_models(&raw).unwrap();
        assert_eq!(models[0].id, "gpt-test");
        assert_eq!(models[0].owned_by.as_deref(), Some("openai"));
    }

    #[test]
    fn parses_ollama_tags() {
        let raw = json!({
            "models": [{
                "name": "llama3.2:latest",
                "model": "llama3.2:latest",
                "details": { "family": "llama", "parameter_size": "3.2B" }
            }]
        });

        let models = parse_ollama_models(&raw).unwrap();
        assert_eq!(models[0].id, "llama3.2:latest");
        assert_eq!(models[0].family.as_deref(), Some("llama"));
    }

    #[test]
    fn compacts_large_provider_metadata() {
        let raw = json!({
            "object": "list",
            "data": [{
                "id": "gpt-test",
                "created": 1,
                "owned_by": "openai",
                "description": "x".repeat(MAX_METADATA_STRING_CHARS + 100),
                "capabilities": (0..MAX_METADATA_ARRAY_ITEMS + 10).collect::<Vec<_>>()
            }]
        });

        let model = parse_openai_models(&raw).unwrap().remove(0);
        let description = model
            .metadata
            .get("description")
            .and_then(Value::as_str)
            .unwrap();
        let capabilities = model
            .metadata
            .get("capabilities")
            .and_then(Value::as_array)
            .unwrap();

        assert_eq!(description.chars().count(), MAX_METADATA_STRING_CHARS);
        assert_eq!(capabilities.len(), MAX_METADATA_ARRAY_ITEMS);
    }

    #[test]
    fn streaming_sse_parser_drains_consumed_events() {
        let mut pending = "event: delta\ndata: {\"ok\":true}\n\npartial".to_string();
        let mut seen = Vec::new();

        consume_sse(&mut pending, |data| seen.push(data.to_string()));

        assert_eq!(seen, vec!["{\"ok\":true}"]);
        assert_eq!(pending, "partial");
    }

    #[test]
    fn streaming_line_parser_drains_consumed_lines() {
        let mut pending = "{\"a\":1}\n{\"b\":2}".to_string();
        let mut seen = Vec::new();

        consume_lines(&mut pending, |line| seen.push(line.to_string()));

        assert_eq!(seen, vec!["{\"a\":1}"]);
        assert_eq!(pending, "{\"b\":2}");
    }
}
