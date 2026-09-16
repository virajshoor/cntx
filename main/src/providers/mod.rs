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
    /// Stable conversation/session id sent as `x-opencode-session` on
    /// OpenCode Go endpoints. The same conversation keeps the same id across
    /// tools, counsel, retries, and compaction; `/clear` starts a new one.
    pub session_id: Option<String>,
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
        let mut emitted = false;
        let mut forward = |delta: String| {
            emitted |= !delta.is_empty();
            on_delta(delta);
        };
        let result = tokio::select! {
            result = adapter.stream_chat(endpoint, request.clone(), &mut forward) => result,
            _ = crate::interactive::wait_for_interrupt() => Err(anyhow!("provider request interrupted")),
        };
        match result {
            Ok(()) => return Ok(()),
            Err(e) if !emitted && attempt < MAX_RETRIES && is_retryable_error(&e) => {
                eprintln!(
                    "  retrying in {}s (attempt {}/{})",
                    backoff / 1000,
                    attempt + 1,
                    MAX_RETRIES
                );
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_millis(backoff)) => {},
                    _ = crate::interactive::wait_for_interrupt() => return Err(anyhow!("provider request interrupted")),
                }
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

pub(crate) fn client(endpoint: &EndpointConfig) -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(endpoint.timeout_secs))
        .build()
        .context("failed to build HTTP client")
}

pub(crate) fn join_url(base_url: &str, path: &str) -> String {
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
    // Client identity: always send our own user agent so subscription
    // gateways (including OpenCode Go) recognize the client.
    headers.insert(
        reqwest::header::USER_AGENT,
        HeaderValue::from_str(&format!("cntx/{}", env!("CARGO_PKG_VERSION")))?,
    );

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

/// Attach the OpenCode Go session header on Go endpoints when a session id
/// is present in the request.
fn with_session_header(
    mut headers: HeaderMap,
    endpoint: &EndpointConfig,
    request: &ChatRequest,
) -> HeaderMap {
    if endpoint.preset_identity() == Some("opencode-go") {
        if let Some(session) = request.session_id.as_deref() {
            if let Ok(value) = HeaderValue::from_str(session) {
                headers.insert("x-opencode-session", value);
            }
        }
    }
    headers
}

/// Normalized model id for an endpoint: `opencode-go/<id>` collapses to
/// `<id>` only for Go preset endpoints; no other prefixes are stripped.
pub fn normalize_model_for_endpoint(endpoint: &EndpointConfig, model: &str) -> String {
    if endpoint.preset_identity() == Some("opencode-go") {
        if let Some(stripped) = model.strip_prefix("opencode-go/") {
            return stripped.to_string();
        }
    }
    model.to_string()
}

/// Resolve the API protocol for a Go preset endpoint: an explicit metadata
/// `protocol` override wins, otherwise the model family decides. Unknown
/// families stay unknown so callers can request an override.
pub fn go_protocol(endpoint: &EndpointConfig, model: &str) -> i32 {
    if endpoint.preset_identity() != Some("opencode-go") {
        return crate::core::GO_PROTOCOL_UNKNOWN;
    }
    if let Some(override_name) = endpoint.protocol_override() {
        return match override_name.trim().to_lowercase().as_str() {
            "chat" | "chat-completions" => crate::core::GO_PROTOCOL_CHAT,
            "messages" => crate::core::GO_PROTOCOL_MESSAGES,
            "responses" => crate::core::GO_PROTOCOL_RESPONSES,
            _ => crate::core::GO_PROTOCOL_UNKNOWN,
        };
    }
    crate::core::go_protocol_for_model(model)
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
        let model = normalize_model_for_endpoint(endpoint, &request.model);
        match go_protocol(endpoint, &model) {
            crate::core::GO_PROTOCOL_MESSAGES => {
                stream_messages_compatible(endpoint, &model, request, on_delta).await
            }
            crate::core::GO_PROTOCOL_RESPONSES => {
                stream_responses(endpoint, &model, request, on_delta).await
            }
            crate::core::GO_PROTOCOL_UNKNOWN if endpoint.preset_identity() == Some("opencode-go") =>
                Err(anyhow!("unknown Go model family or protocol override; set endpoint metadata.protocol to chat, messages, or responses")),
            _ => stream_chat_completions(endpoint, &model, request, on_delta).await,
        }
    }
}

/// Standard `/chat/completions` streaming.
async fn stream_chat_completions(
    endpoint: &EndpointConfig,
    model: &str,
    request: ChatRequest,
    on_delta: &mut (dyn FnMut(String) + Send),
) -> Result<()> {
    let messages: Vec<Value> = request
        .messages
        .iter()
        .map(|message| json!({ "role": message.role, "content": message.content }))
        .collect();
    let mut body = json!({
        "model": model,
        "messages": messages,
        "stream": true,
    });
    if let Some(max_tokens) = request.max_tokens {
        body["max_tokens"] = json!(max_tokens);
    }

    let request_headers = with_session_header(
        headers(endpoint, &openai_kind_for(endpoint))?,
        endpoint,
        &request,
    );
    let mut stream = client(endpoint)?
        .post(join_url(
            &endpoint.base_url,
            &endpoint_path(endpoint, "chat_path", "chat/completions"),
        ))
        .headers(request_headers)
        .json(&body)
        .send()
        .await?
        .error_for_status()?
        .bytes_stream();

    let mut pending = Vec::new();
    while let Some(chunk) = stream.next().await {
        pending.extend_from_slice(&chunk?);
        if pending.len() > 1024 * 1024 {
            anyhow::bail!("provider stream record exceeds 1 MiB");
        }
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

/// Anthropic-compatible `/messages` streaming as used by Go's MiniMax and
/// Qwen families. Authentication stays the endpoint's Bearer scheme; system
/// messages are merged so no skills/summaries/goal instructions are lost.
async fn stream_messages_compatible(
    endpoint: &EndpointConfig,
    model: &str,
    request: ChatRequest,
    on_delta: &mut (dyn FnMut(String) + Send),
) -> Result<()> {
    let messages: Vec<Value> = request
        .messages
        .iter()
        .filter(|message| message.role != "system")
        .map(|message| json!({ "role": message.role, "content": message.content }))
        .collect();
    let system = merged_system_text(&request.messages);
    let mut body = json!({
        "model": model,
        "messages": messages,
        "max_tokens": request.max_tokens.unwrap_or(4096),
        "stream": true,
    });
    if let Some(system) = system {
        body["system"] = Value::String(system);
    }

    let request_headers = with_session_header(
        headers(endpoint, &openai_kind_for(endpoint))?,
        endpoint,
        &request,
    );
    let mut stream = client(endpoint)?
        .post(join_url(
            &endpoint.base_url,
            &endpoint_path(endpoint, "chat_path", "messages"),
        ))
        .headers(request_headers)
        .json(&body)
        .send()
        .await?
        .error_for_status()?
        .bytes_stream();

    let mut pending = Vec::new();
    while let Some(chunk) = stream.next().await {
        pending.extend_from_slice(&chunk?);
        if pending.len() > 1024 * 1024 {
            anyhow::bail!("provider stream record exceeds 1 MiB");
        }
        consume_sse(&mut pending, |data| {
            if let Ok(value) = serde_json::from_str::<Value>(data) {
                // Anthropic content_block_delta events carry text at
                // /delta/text; a bare {type:"text", text:...} fallback
                // covers gateway variants.
                if let Some(content) = value.pointer("/delta/text").and_then(Value::as_str) {
                    on_delta(content.to_string());
                } else if value.get("type").and_then(Value::as_str) == Some("text") {
                    if let Some(content) = value.get("text").and_then(Value::as_str) {
                        on_delta(content.to_string());
                    }
                }
            }
        });
    }
    Ok(())
}

/// OpenAI Responses `/responses` streaming (GPT/Grok/Muse families on Go).
/// Parses text deltas, end-of-response events, and API error events.
async fn stream_responses(
    endpoint: &EndpointConfig,
    model: &str,
    request: ChatRequest,
    on_delta: &mut (dyn FnMut(String) + Send),
) -> Result<()> {
    let mut input = Vec::with_capacity(request.messages.len());
    let mut system_parts: Vec<&str> = Vec::new();
    for message in &request.messages {
        if message.role == "system" {
            system_parts.push(&message.content);
            continue;
        }
        let type_name = if message.role == "assistant" {
            "output_text"
        } else {
            "input_text"
        };
        input.push(json!({
            "role": message.role,
            "content": [{ "type": type_name, "text": message.content }]
        }));
    }
    let mut body = json!({
        "model": model,
        "input": input,
        "stream": true,
    });
    body["instructions"] = Value::String(system_parts.join("\n\n"));
    if let Some(max_tokens) = request.max_tokens {
        body["max_output_tokens"] = json!(max_tokens);
    }

    let request_headers = with_session_header(
        headers(endpoint, &openai_kind_for(endpoint))?,
        endpoint,
        &request,
    );
    let mut stream = client(endpoint)?
        .post(join_url(&endpoint.base_url, "responses"))
        .headers(request_headers)
        .json(&body)
        .send()
        .await?
        .error_for_status()?
        .bytes_stream();

    let mut pending = Vec::new();
    let mut ended = false;
    while let Some(chunk) = stream.next().await {
        pending.extend_from_slice(&chunk?);
        if pending.len() > 1024 * 1024 {
            anyhow::bail!("provider stream record exceeds 1 MiB");
        }
        let mut error_message: Option<String> = None;
        consume_sse(&mut pending, |data| {
            if data == "[DONE]" {
                ended = true;
                return;
            }
            let Ok(value) = serde_json::from_str::<Value>(data) else {
                return;
            };
            match value.get("type").and_then(Value::as_str) {
                Some("response.output_text.delta") => {
                    if let Some(delta) = value.get("delta").and_then(Value::as_str) {
                        on_delta(delta.to_string());
                    }
                }
                Some("response.completed") => {
                    ended = true;
                }
                Some("response.incomplete") => {
                    ended = true;
                    error_message =
                        Some("response incomplete; output limit or provider interruption".into());
                }
                Some("response.failed") => {
                    ended = true;
                    error_message = Some(
                        value
                            .pointer("/response/error/message")
                            .or_else(|| value.pointer("/response/status"))
                            .map(stringify_value)
                            .unwrap_or_else(|| "response failed".to_string()),
                    );
                }
                Some("error") => {
                    ended = true;
                    error_message = Some(
                        value
                            .pointer("/message")
                            .map(stringify_value)
                            .unwrap_or_else(|| "stream error".to_string()),
                    );
                }
                _ => {}
            }
        });
        if let Some(message) = error_message {
            anyhow::bail!("Go responses stream error: {message}");
        }
        if ended {
            break;
        }
    }
    if !ended {
        anyhow::bail!("responses stream ended before completion");
    }
    Ok(())
}

fn openai_kind_for(endpoint: &EndpointConfig) -> ProviderKind {
    endpoint.provider.clone()
}

fn stringify_value(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// Merge every system message instead of dropping extra ones so skills,
/// summaries, and goal instructions all reach Anthropic-compatible models.
fn merged_system_text(messages: &[ChatMessage]) -> Option<String> {
    let parts: Vec<&str> = messages
        .iter()
        .filter(|message| message.role == "system")
        .map(|message| message.content.as_str())
        .collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
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
        // Merge ALL system messages: base instructions, skills, summaries,
        // and goal instructions must all reach the model.
        let system = merged_system_text(&request.messages);
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

        let mut pending = Vec::new();
        while let Some(chunk) = stream.next().await {
            pending.extend_from_slice(&chunk?);
            if pending.len() > 1024 * 1024 {
                anyhow::bail!("provider stream record exceeds 1 MiB");
            }
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

        let mut pending = Vec::new();
        while let Some(chunk) = stream.next().await {
            pending.extend_from_slice(&chunk?);
            if pending.len() > 1024 * 1024 {
                anyhow::bail!("provider stream record exceeds 1 MiB");
            }
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
            .and_then(|value| value.as_str())
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

/// Context length from an Ollama `/api/show` response. The field name is
/// `<family>.context_length` (e.g. `llama.context_length`), so match by
/// suffix instead of hardcoding families.
pub fn parse_ollama_context_window(value: &Value) -> Option<usize> {
    value
        .as_object()?
        .iter()
        .find(|(key, _)| key.ends_with(".context_length"))
        .and_then(|(_, v)| v.as_u64())
        .and_then(|window| usize::try_from(window).ok())
        .filter(|window| *window > 0)
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

fn consume_sse(pending: &mut Vec<u8>, mut on_data: impl FnMut(&str)) {
    loop {
        let lf = pending
            .windows(2)
            .position(|w| w == b"\n\n")
            .map(|i| (i, 2));
        let crlf = pending
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .map(|i| (i, 4));
        let Some((index, delimiter)) = lf.into_iter().chain(crlf).min_by_key(|v| v.0) else {
            break;
        };
        let event = String::from_utf8_lossy(&pending[..index]);
        let data = event
            .lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .map(str::trim)
            .collect::<Vec<_>>()
            .join("\n");
        if !data.is_empty() {
            on_data(&data);
        }
        pending.drain(..index + delimiter);
    }
}

fn consume_lines(pending: &mut Vec<u8>, mut on_line: impl FnMut(&str)) {
    while let Some(index) = pending.iter().position(|b| *b == b'\n') {
        let line = String::from_utf8_lossy(&pending[..index]);
        if !line.trim().is_empty() {
            on_line(line.trim());
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
        let mut pending = b"event: delta\ndata: {\"ok\":true}\n\npartial".to_vec();
        let mut seen = Vec::new();

        consume_sse(&mut pending, |data| seen.push(data.to_string()));

        assert_eq!(seen, vec!["{\"ok\":true}"]);
        assert_eq!(pending, b"partial");
    }

    #[test]
    fn streaming_line_parser_drains_consumed_lines() {
        let mut pending = b"{\"a\":1}\n{\"b\":2}".to_vec();
        let mut seen = Vec::new();

        consume_lines(&mut pending, |line| seen.push(line.to_string()));

        assert_eq!(seen, vec!["{\"a\":1}"]);
        assert_eq!(pending, b"{\"b\":2}");
    }

    #[test]
    fn sse_parser_accepts_crlf_events() {
        let mut pending = b"data: {\"a\":1}\r\n\r\ndata: {\"b\":2}\r\n\r\n".to_vec();
        let mut seen = Vec::new();

        consume_sse(&mut pending, |data| seen.push(data.to_string()));

        assert_eq!(seen, vec!["{\"a\":1}", "{\"b\":2}"]);
        assert!(pending.is_empty());
    }

    #[test]
    fn sse_parser_keeps_split_utf8_pending_until_complete() {
        // A UTF-8 sequence split across stream chunks must stay pending as
        // raw bytes; the completed event decodes losslessly.
        let mut pending = b"data: {\"t\":\"\xC3".to_vec();
        let mut seen = Vec::new();
        consume_sse(&mut pending, |data| seen.push(data.to_string()));
        assert!(seen.is_empty());

        pending.extend_from_slice(b"\xA9\"}\n\n");
        consume_sse(&mut pending, |data| seen.push(data.to_string()));
        assert_eq!(seen, vec!["{\"t\":\"é\"}"]);
        assert!(pending.is_empty());
    }

    #[test]
    fn line_parser_keeps_split_utf8_pending_until_complete() {
        let mut pending = b"{\"t\":\"\xC3".to_vec();
        let mut seen = Vec::new();
        consume_lines(&mut pending, |line| seen.push(line.to_string()));
        assert!(seen.is_empty());

        pending.extend_from_slice(b"\xA9\"}\n");
        consume_lines(&mut pending, |line| seen.push(line.to_string()));
        assert_eq!(seen, vec!["{\"t\":\"é\"}"]);
        assert!(pending.is_empty());
    }

    #[test]
    fn sse_parser_keeps_unterminated_event_pending() {
        let mut pending = b"data: {\"a\":1}".to_vec();
        let mut seen = Vec::new();

        consume_sse(&mut pending, |data| seen.push(data.to_string()));

        assert!(seen.is_empty());
        assert_eq!(pending, b"data: {\"a\":1}");
    }

    #[test]
    fn parses_ollama_context_window_from_show_response() {
        assert_eq!(
            parse_ollama_context_window(&json!({
                "llama.context_length": 131072,
                "modelfile": "..."
            })),
            Some(131072)
        );
        assert_eq!(
            parse_ollama_context_window(&json!({ "other.field": 1 })),
            None
        );
        assert_eq!(
            parse_ollama_context_window(&json!({ "a.context_length": 0 })),
            None
        );
    }
}
