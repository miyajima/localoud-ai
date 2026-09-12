use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use protocol_types::providers::{
    ContentBlock, InferenceEvent, InferenceRequest, InferenceResponse, InferenceUsage,
    ModelCapabilities, ProviderModel, ProviderProfile, ProviderProtocol, TranscriptMessage,
    TranscriptRole,
};
use reqwest::{header, Client, RequestBuilder, Response, StatusCode, Url};
use serde_json::{json, Value};
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tokio::sync::Semaphore;

const RESPONSE_LIMIT: usize = 1_000_000;
const ANTHROPIC_VERSION: &str = "2023-06-01";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderFailureKind {
    RateLimited,
    Authentication,
    ContextLength,
    Refusal,
    StreamInterrupted,
    ResponseTooLarge,
    InvalidResponse,
    Transport,
    Unknown,
}

impl ProviderFailureKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RateLimited => "rate_limited",
            Self::Authentication => "authentication",
            Self::ContextLength => "context_length",
            Self::Refusal => "refusal",
            Self::StreamInterrupted => "stream_interrupted",
            Self::ResponseTooLarge => "response_too_large",
            Self::InvalidResponse => "invalid_response",
            Self::Transport => "transport",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug)]
pub struct ProviderFailure {
    pub kind: ProviderFailureKind,
    message: String,
}

impl std::fmt::Display for ProviderFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ProviderFailure {}

fn provider_failure(kind: ProviderFailureKind, message: impl Into<String>) -> anyhow::Error {
    ProviderFailure {
        kind,
        message: message.into(),
    }
    .into()
}

pub fn failure_kind(error: &anyhow::Error) -> ProviderFailureKind {
    error
        .chain()
        .find_map(|source| source.downcast_ref::<ProviderFailure>())
        .map(|failure| failure.kind)
        .unwrap_or(ProviderFailureKind::Unknown)
}

fn normalize_provider_failure(error: anyhow::Error) -> anyhow::Error {
    if failure_kind(&error) != ProviderFailureKind::Unknown {
        return error;
    }
    let message = format!("{error:#}");
    let lower = message.to_ascii_lowercase();
    let kind = if error
        .chain()
        .any(|source| source.downcast_ref::<serde_json::Error>().is_some())
    {
        ProviderFailureKind::InvalidResponse
    } else if lower.contains("ended before") || lower.contains("connection closed") {
        ProviderFailureKind::StreamInterrupted
    } else if lower.contains("invalid json")
        || lower.contains("invalid event stream")
        || lower.contains("non-utf-8")
        || lower.contains("missing field")
        || lower.contains("incomplete tool")
    {
        ProviderFailureKind::InvalidResponse
    } else {
        ProviderFailureKind::Unknown
    };
    provider_failure(kind, message)
}

fn http_failure_kind(status: StatusCode, body: &str) -> ProviderFailureKind {
    if status == StatusCode::TOO_MANY_REQUESTS {
        return ProviderFailureKind::RateLimited;
    }
    if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
        return ProviderFailureKind::Authentication;
    }
    let lower = body.to_ascii_lowercase();
    if lower.contains("context_length")
        || lower.contains("context window")
        || lower.contains("maximum context")
        || lower.contains("prompt is too long")
    {
        ProviderFailureKind::ContextLength
    } else if lower.contains("content_filter")
        || lower.contains("content filter")
        || lower.contains("safety policy")
        || lower.contains("refusal")
    {
        ProviderFailureKind::Refusal
    } else if status.is_server_error() {
        ProviderFailureKind::Transport
    } else {
        ProviderFailureKind::Unknown
    }
}

fn provider_payload_failure(label: &str, value: &Value) -> anyhow::Error {
    let rendered = value.to_string();
    let lower = rendered.to_ascii_lowercase();
    let kind = if lower.contains("rate_limit")
        || lower.contains("rate limit")
        || lower.contains("too_many_requests")
        || lower.contains("overloaded")
    {
        ProviderFailureKind::RateLimited
    } else if lower.contains("authentication")
        || lower.contains("invalid_api_key")
        || lower.contains("invalid api key")
        || lower.contains("permission_error")
        || lower.contains("unauthorized")
        || lower.contains("forbidden")
    {
        ProviderFailureKind::Authentication
    } else if lower.contains("context_length")
        || lower.contains("context window")
        || lower.contains("maximum context")
        || lower.contains("prompt is too long")
    {
        ProviderFailureKind::ContextLength
    } else if lower.contains("content_filter")
        || lower.contains("content filter")
        || lower.contains("safety policy")
        || lower.contains("refusal")
    {
        ProviderFailureKind::Refusal
    } else {
        ProviderFailureKind::InvalidResponse
    };
    provider_failure(kind, format!("{label}: {}", hub_policy::redact(&rendered)))
}

#[async_trait]
pub trait InferenceTransport: Send + Sync {
    fn profile(&self) -> &ProviderProfile;
    async fn models(&self) -> Result<Vec<ProviderModel>>;
    async fn infer(&self, request: InferenceRequest) -> Result<InferenceResponse>;
    async fn infer_stream(&self, request: InferenceRequest) -> Result<Vec<InferenceEvent>> {
        Ok(vec![InferenceEvent::Completed {
            response: self.infer(request).await?,
        }])
    }
}

pub struct ApiTransport {
    profile: ProviderProfile,
    base_url: Url,
    client: Client,
    permits: Arc<Semaphore>,
}

#[derive(Default)]
pub struct ProviderRegistry {
    profiles: BTreeMap<String, ProviderProfile>,
    transports: BTreeMap<String, Arc<dyn InferenceTransport>>,
}

impl ProviderRegistry {
    pub fn from_profiles(profiles: impl IntoIterator<Item = ProviderProfile>) -> Result<Self> {
        let mut registry = Self::default();
        for profile in profiles {
            registry.insert(profile)?;
        }
        Ok(registry)
    }

    pub fn insert(&mut self, profile: ProviderProfile) -> Result<()> {
        profile.validate()?;
        let id = profile.id.clone();
        self.transports.remove(&id);
        if profile.enabled
            && matches!(
                profile.protocol,
                ProviderProtocol::OpenAiChat
                    | ProviderProtocol::OpenAiResponses
                    | ProviderProtocol::AnthropicMessages
            )
        {
            self.transports
                .insert(id.clone(), Arc::new(ApiTransport::new(profile.clone())?));
        }
        self.profiles.insert(id, profile);
        Ok(())
    }

    pub fn profiles(&self) -> Vec<ProviderProfile> {
        self.profiles.values().cloned().collect()
    }

    pub fn profile(&self, id: &str) -> Option<&ProviderProfile> {
        self.profiles.get(id)
    }

    pub fn transport(&self, id: &str) -> Result<Arc<dyn InferenceTransport>> {
        self.transports
            .get(id)
            .cloned()
            .with_context(|| format!("provider profile {id} has no HTTP inference transport"))
    }

    pub async fn models(&self, id: &str) -> Result<Vec<ProviderModel>> {
        self.transport(id)?.models().await
    }

    pub async fn infer(&self, request: InferenceRequest) -> Result<InferenceResponse> {
        self.transport(&request.target.profile_id)?
            .infer(request)
            .await
    }

    pub async fn infer_stream(&self, request: InferenceRequest) -> Result<Vec<InferenceEvent>> {
        self.transport(&request.target.profile_id)?
            .infer_stream(request)
            .await
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SseEvent {
    event: Option<String>,
    data: String,
}

impl ApiTransport {
    pub fn new(profile: ProviderProfile) -> Result<Self> {
        profile.validate()?;
        anyhow::ensure!(
            !matches!(profile.protocol, ProviderProtocol::CodexAppServer),
            "Codex app-server does not use the HTTP inference transport"
        );
        let mut base_url = Url::parse(profile.base_url.as_deref().context("missing base URL")?)?;
        if !base_url.path().ends_with('/') {
            base_url.set_path(&format!("{}/", base_url.path()));
        }
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(120))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self {
            permits: Arc::new(Semaphore::new(usize::from(profile.max_concurrency))),
            profile,
            base_url,
            client,
        })
    }

    fn endpoint(&self, path: &str) -> Result<Url> {
        Ok(self.base_url.join(path.trim_start_matches('/'))?)
    }

    fn credential(&self) -> Result<Option<String>> {
        self.profile
            .credential_env
            .as_ref()
            .map(|name| {
                std::env::var(name).map_err(|_| {
                    provider_failure(
                        ProviderFailureKind::Authentication,
                        format!("credential environment variable {name} is not set"),
                    )
                })
            })
            .transpose()
    }

    fn authenticate(&self, request: RequestBuilder) -> Result<RequestBuilder> {
        let Some(secret) = self.credential()? else {
            return Ok(request);
        };
        Ok(match self.profile.protocol {
            ProviderProtocol::AnthropicMessages => request.header("x-api-key", secret),
            _ => request.bearer_auth(secret),
        })
    }

    async fn send_json(&self, request: RequestBuilder, label: &str) -> Result<Value> {
        let response = request.send().await.map_err(|error| {
            provider_failure(
                ProviderFailureKind::Transport,
                format!("{label} request failed: {error}"),
            )
        })?;
        let status = response.status();
        let bytes = limited_body(response, label).await?;
        if !status.is_success() {
            let body = hub_policy::redact(
                &String::from_utf8_lossy(&bytes)
                    .chars()
                    .take(600)
                    .collect::<String>(),
            );
            return Err(provider_failure(
                http_failure_kind(status, &body),
                format!("{label} returned {status}: {body}"),
            ));
        }
        serde_json::from_slice(&bytes).with_context(|| format!("{label} returned invalid JSON"))
    }

    async fn send_sse(&self, request: RequestBuilder, label: &str) -> Result<Vec<SseEvent>> {
        let response = request.send().await.map_err(|error| {
            provider_failure(
                ProviderFailureKind::Transport,
                format!("{label} request failed: {error}"),
            )
        })?;
        let status = response.status();
        let bytes = limited_body(response, label).await?;
        if !status.is_success() {
            let body = hub_policy::redact(
                &String::from_utf8_lossy(&bytes)
                    .chars()
                    .take(600)
                    .collect::<String>(),
            );
            return Err(provider_failure(
                http_failure_kind(status, &body),
                format!("{label} returned {status}: {body}"),
            ));
        }
        let text = std::str::from_utf8(&bytes)
            .with_context(|| format!("{label} returned non-UTF-8 event data"))?;
        parse_sse(text)
            .with_context(|| format!("{label} returned invalid event stream"))
            .map_err(normalize_provider_failure)
    }

    async fn openai_models(&self) -> Result<Vec<ProviderModel>> {
        let request = self.authenticate(self.client.get(self.endpoint("models")?))?;
        let value = self.send_json(request, "provider models endpoint").await?;
        let items = value["data"]
            .as_array()
            .context("provider models response has no data array")?;
        Ok(items
            .iter()
            .filter_map(|item| item["id"].as_str())
            .map(|id| ProviderModel {
                id: id.into(),
                display_name: id.into(),
                capabilities: capabilities_for(self.profile.protocol),
            })
            .collect())
    }

    async fn anthropic_models(&self) -> Result<Vec<ProviderModel>> {
        let request = self.authenticate(
            self.client
                .get(self.endpoint("models")?)
                .header("anthropic-version", ANTHROPIC_VERSION),
        )?;
        let value = self.send_json(request, "Anthropic Models API").await?;
        let items = value["data"]
            .as_array()
            .context("Anthropic Models response has no data array")?;
        Ok(items
            .iter()
            .filter_map(|item| item["id"].as_str().map(|id| (id, item)))
            .map(|(id, item)| ProviderModel {
                id: id.into(),
                display_name: item["display_name"].as_str().unwrap_or(id).into(),
                capabilities: capabilities_for(self.profile.protocol),
            })
            .collect())
    }

    async fn infer_openai_chat(&self, request: InferenceRequest) -> Result<Vec<InferenceEvent>> {
        let mut messages = Vec::new();
        if let Some(system) = &request.system {
            messages.push(json!({"role":"system","content":system}));
        }
        messages.extend(openai_chat_messages(&request.messages)?);
        let mut body = json!({
            "model": request.target.model_id,
            "messages": messages,
            "stream": true,
            "stream_options": {"include_usage": true},
            "max_tokens": request.max_output_tokens,
        });
        if let Some(effort) = request.target.effort {
            body["reasoning_effort"] = json!(effort);
        }
        if !request.tools.is_empty() {
            body["tools"] = json!(request.tools.into_iter().map(|tool| json!({"type":"function","function":{"name":tool.name,"description":tool.description,"parameters":tool.input_schema}})).collect::<Vec<_>>());
        }
        let request = self.authenticate(
            self.client
                .post(self.endpoint("chat/completions")?)
                .header(header::ACCEPT, "text/event-stream")
                .json(&body),
        )?;
        parse_openai_chat_sse(self.send_sse(request, "OpenAI Chat API").await?)
    }

    async fn infer_openai_responses(
        &self,
        request: InferenceRequest,
    ) -> Result<Vec<InferenceEvent>> {
        let mut body = json!({
            "model": request.target.model_id,
            "input": openai_response_input(&request.messages)?,
            "stream": true,
            "store": false,
            "max_output_tokens": request.max_output_tokens,
        });
        if let Some(system) = request.system {
            body["instructions"] = json!(system);
        }
        if let Some(effort) = request.target.effort {
            body["reasoning"] = json!({"effort":effort});
        }
        if !request.tools.is_empty() {
            body["tools"] = json!(request.tools.into_iter().map(|tool| json!({"type":"function","name":tool.name,"description":tool.description,"parameters":tool.input_schema,"strict":true})).collect::<Vec<_>>());
        }
        let request = self.authenticate(
            self.client
                .post(self.endpoint("responses")?)
                .header(header::ACCEPT, "text/event-stream")
                .json(&body),
        )?;
        parse_openai_responses_sse(self.send_sse(request, "OpenAI Responses API").await?)
    }

    async fn infer_anthropic(&self, request: InferenceRequest) -> Result<Vec<InferenceEvent>> {
        let mut body = json!({
            "model": request.target.model_id,
            "messages": anthropic_messages(&request.messages)?,
            "max_tokens": request.max_output_tokens,
            "stream": true,
        });
        if let Some(system) = request.system {
            body["system"] = json!(system);
        }
        if let Some(effort) = request.target.effort {
            body["thinking"] = json!({"type":"adaptive"});
            body["output_config"] = json!({"effort":effort});
        }
        if !request.tools.is_empty() {
            body["tools"] = json!(request.tools.into_iter().map(|tool| json!({"name":tool.name,"description":tool.description,"input_schema":tool.input_schema})).collect::<Vec<_>>());
        }
        let request = self.authenticate(
            self.client
                .post(self.endpoint("messages")?)
                .header("anthropic-version", ANTHROPIC_VERSION)
                .header(header::ACCEPT, "text/event-stream")
                .header(header::CONTENT_TYPE, "application/json")
                .json(&body),
        )?;
        parse_anthropic_sse(self.send_sse(request, "Anthropic Messages API").await?)
    }

    async fn infer_events(&self, request: InferenceRequest) -> Result<Vec<InferenceEvent>> {
        match self.profile.protocol {
            ProviderProtocol::OpenAiChat => self.infer_openai_chat(request).await,
            ProviderProtocol::OpenAiResponses => self.infer_openai_responses(request).await,
            ProviderProtocol::AnthropicMessages => self.infer_anthropic(request).await,
            ProviderProtocol::LocalDedicated => {
                bail!("dedicated local generation remains handled by provider-spark")
            }
            ProviderProtocol::CodexAppServer => unreachable!(),
        }
    }
}

#[async_trait]
impl InferenceTransport for ApiTransport {
    fn profile(&self) -> &ProviderProfile {
        &self.profile
    }

    async fn models(&self) -> Result<Vec<ProviderModel>> {
        let _permit = self.permits.acquire().await?;
        match self.profile.protocol {
            ProviderProtocol::AnthropicMessages => self.anthropic_models().await,
            ProviderProtocol::OpenAiChat | ProviderProtocol::OpenAiResponses => {
                self.openai_models().await
            }
            ProviderProtocol::LocalDedicated => {
                bail!("dedicated local model discovery remains handled by provider-spark")
            }
            ProviderProtocol::CodexAppServer => unreachable!(),
        }
    }

    async fn infer(&self, request: InferenceRequest) -> Result<InferenceResponse> {
        request.validate()?;
        anyhow::ensure!(
            request.target.profile_id == self.profile.id,
            "inference target does not match provider profile"
        );
        anyhow::ensure!(self.profile.enabled, "provider profile is disabled");
        let _permit = self.permits.acquire().await?;
        completed_response(
            self.infer_events(request)
                .await
                .map_err(normalize_provider_failure)?,
        )
        .map_err(normalize_provider_failure)
    }

    async fn infer_stream(&self, request: InferenceRequest) -> Result<Vec<InferenceEvent>> {
        request.validate()?;
        anyhow::ensure!(
            request.target.profile_id == self.profile.id,
            "inference target does not match provider profile"
        );
        anyhow::ensure!(self.profile.enabled, "provider profile is disabled");
        let _permit = self.permits.acquire().await?;
        self.infer_events(request)
            .await
            .map_err(normalize_provider_failure)
    }
}

pub fn capabilities_for(protocol: ProviderProtocol) -> ModelCapabilities {
    match protocol {
        ProviderProtocol::CodexAppServer => ModelCapabilities {
            tools: true,
            streaming: true,
            structured_output: true,
            efforts: vec!["low".into(), "medium".into(), "high".into()],
            ..Default::default()
        },
        ProviderProtocol::OpenAiResponses => ModelCapabilities {
            tools: true,
            streaming: true,
            structured_output: true,
            efforts: vec!["low".into(), "medium".into(), "high".into()],
            ..Default::default()
        },
        ProviderProtocol::AnthropicMessages => ModelCapabilities {
            tools: true,
            streaming: true,
            structured_output: false,
            efforts: vec!["low".into(), "medium".into(), "high".into(), "max".into()],
            ..Default::default()
        },
        ProviderProtocol::OpenAiChat => ModelCapabilities {
            tools: false,
            streaming: true,
            structured_output: false,
            ..Default::default()
        },
        ProviderProtocol::LocalDedicated => ModelCapabilities {
            structured_output: true,
            ..Default::default()
        },
    }
}

fn openai_chat_messages(messages: &[TranscriptMessage]) -> Result<Vec<Value>> {
    let mut result = Vec::new();
    for message in messages {
        let role = role_name(message.role);
        let text = message
            .content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        let tool_calls = message.content.iter().filter_map(|block| match block {
            ContentBlock::ToolUse { id, name, arguments } => Some(json!({"id":id,"type":"function","function":{"name":name,"arguments":arguments.to_string()}})),
            _ => None,
        }).collect::<Vec<_>>();
        let tool_results = message
            .content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::ToolResult {
                    tool_use_id, text, ..
                } => Some((tool_use_id, text)),
                _ => None,
            })
            .collect::<Vec<_>>();
        if !tool_results.is_empty() {
            for (id, text) in tool_results {
                result.push(json!({"role":"tool","tool_call_id":id,"content":text}));
            }
        } else if !tool_calls.is_empty() {
            result.push(json!({"role":"assistant","content":if text.is_empty(){Value::Null}else{json!(text)},"tool_calls":tool_calls}));
        } else if !text.is_empty() {
            result.push(json!({"role":role,"content":text}));
        }
    }
    Ok(result)
}

fn openai_response_input(messages: &[TranscriptMessage]) -> Result<Vec<Value>> {
    let mut result = Vec::new();
    for message in messages {
        let text = message
            .content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        if !text.is_empty() {
            result.push(json!({"role":role_name(message.role),"content":text}));
        }
        for block in &message.content {
            match block {
                ContentBlock::ToolUse { id, name, arguments } => result.push(json!({"type":"function_call","call_id":id,"name":name,"arguments":arguments.to_string()})),
                ContentBlock::ToolResult { tool_use_id, text, .. } => result.push(json!({"type":"function_call_output","call_id":tool_use_id,"output":text})),
                _ => {}
            }
        }
    }
    Ok(result)
}

fn anthropic_messages(messages: &[TranscriptMessage]) -> Result<Vec<Value>> {
    let mut result = Vec::new();
    for message in messages {
        if message.role == TranscriptRole::System {
            continue;
        }
        let mut content = Vec::new();
        if message.role == TranscriptRole::Assistant {
            if let Some(Value::Array(blocks)) = &message.provider_state {
                content.extend(blocks.iter().cloned());
            }
        }
        for block in &message.content {
            match block {
                ContentBlock::Text { text } => content.push(json!({"type":"text","text":text})),
                ContentBlock::ToolUse { id, name, arguments } => content.push(json!({"type":"tool_use","id":id,"name":name,"input":arguments})),
                ContentBlock::ToolResult { tool_use_id, text, is_error } => content.push(json!({"type":"tool_result","tool_use_id":tool_use_id,"content":text,"is_error":is_error})),
                ContentBlock::Refusal { .. } => {}
            }
        }
        if !content.is_empty() {
            let role = if message.role == TranscriptRole::Assistant {
                "assistant"
            } else {
                "user"
            };
            result.push(json!({"role":role,"content":content}));
        }
    }
    Ok(result)
}

fn role_name(role: TranscriptRole) -> &'static str {
    match role {
        TranscriptRole::System => "system",
        TranscriptRole::User => "user",
        TranscriptRole::Assistant => "assistant",
        TranscriptRole::Tool => "tool",
    }
}

fn parse_sse(input: &str) -> Result<Vec<SseEvent>> {
    let normalized = input.replace("\r\n", "\n").replace('\r', "\n");
    let mut events = Vec::new();
    let mut event = None;
    let mut data = Vec::new();
    let flush = |events: &mut Vec<SseEvent>, event: &mut Option<String>, data: &mut Vec<String>| {
        if !data.is_empty() || event.is_some() {
            events.push(SseEvent {
                event: event.take(),
                data: std::mem::take(data).join("\n"),
            });
        }
    };
    for line in normalized.lines() {
        if line.is_empty() {
            flush(&mut events, &mut event, &mut data);
            continue;
        }
        if line.starts_with(':') {
            continue;
        }
        let (field, raw) = line.split_once(':').unwrap_or((line, ""));
        let value = raw.strip_prefix(' ').unwrap_or(raw);
        match field {
            "event" => event = Some(value.into()),
            "data" => data.push(value.into()),
            _ => {}
        }
    }
    flush(&mut events, &mut event, &mut data);
    anyhow::ensure!(!events.is_empty(), "event stream was empty");
    Ok(events)
}

fn completed_response(events: Vec<InferenceEvent>) -> Result<InferenceResponse> {
    events
        .into_iter()
        .rev()
        .find_map(|event| match event {
            InferenceEvent::Completed { response } => Some(response),
            _ => None,
        })
        .context("provider stream ended before completion")
}

#[derive(Default)]
struct ToolBuilder {
    id: String,
    name: String,
    arguments: String,
}

fn parse_openai_chat_sse(source: Vec<SseEvent>) -> Result<Vec<InferenceEvent>> {
    let mut result = Vec::new();
    let mut text = String::new();
    let mut refusal = String::new();
    let mut calls: BTreeMap<u64, ToolBuilder> = BTreeMap::new();
    let mut usage = InferenceUsage::default();
    let mut stop_reason = None;
    let mut completed = false;
    for event in source {
        if event.data.trim() == "[DONE]" {
            completed = true;
            continue;
        }
        if event.data.trim().is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(&event.data)?;
        if let Some(error) = value.get("error").filter(|value| !value.is_null()) {
            return Err(provider_payload_failure("OpenAI Chat stream error", error));
        }
        if value.get("usage").is_some_and(Value::is_object) {
            usage = openai_usage(&value);
        }
        for choice in value["choices"].as_array().into_iter().flatten() {
            if choice["index"].as_u64().unwrap_or(0) != 0 {
                continue;
            }
            if let Some(reason) = choice["finish_reason"].as_str() {
                stop_reason = Some(reason.to_owned());
            }
            let delta = &choice["delta"];
            if let Some(value) = delta["content"].as_str().filter(|value| !value.is_empty()) {
                text.push_str(value);
                result.push(InferenceEvent::TextDelta { text: value.into() });
            }
            if let Some(value) = delta["refusal"].as_str().filter(|value| !value.is_empty()) {
                refusal.push_str(value);
            }
            for call in delta["tool_calls"].as_array().into_iter().flatten() {
                let index = call["index"].as_u64().unwrap_or(0);
                let builder = calls.entry(index).or_default();
                if let Some(id) = call["id"].as_str() {
                    builder.id.push_str(id);
                }
                if let Some(name) = call["function"]["name"].as_str() {
                    builder.name.push_str(name);
                }
                let arguments = call["function"]["arguments"].as_str().unwrap_or("");
                builder.arguments.push_str(arguments);
                result.push(InferenceEvent::ToolUseDelta {
                    id: builder.id.clone(),
                    name: builder.name.clone(),
                    arguments_delta: arguments.into(),
                });
            }
        }
    }
    anyhow::ensure!(completed, "OpenAI Chat stream ended before [DONE]");
    let mut content = Vec::new();
    if !text.is_empty() {
        content.push(ContentBlock::Text { text });
    }
    if !refusal.is_empty() {
        content.push(ContentBlock::Refusal {
            category: None,
            text: Some(refusal),
        });
    }
    for (_, call) in calls {
        anyhow::ensure!(
            !call.id.is_empty() && !call.name.is_empty(),
            "incomplete tool call"
        );
        let arguments = serde_json::from_str(if call.arguments.is_empty() {
            "{}"
        } else {
            &call.arguments
        })?;
        content.push(ContentBlock::ToolUse {
            id: call.id,
            name: call.name,
            arguments,
        });
    }
    let response = InferenceResponse {
        content,
        usage,
        stop_reason: stop_reason.unwrap_or_else(|| "completed".into()),
        provider_state: None,
    };
    result.push(InferenceEvent::Completed { response });
    Ok(result)
}

fn parse_openai_responses_sse(source: Vec<SseEvent>) -> Result<Vec<InferenceEvent>> {
    let mut result = Vec::new();
    let mut response = None;
    for event in source {
        let event_name = event.event.as_deref().unwrap_or("");
        if event.data.trim().is_empty() || event.data.trim() == "[DONE]" {
            continue;
        }
        let value: Value = serde_json::from_str(&event.data)?;
        match event_name {
            "response.output_text.delta" => {
                if let Some(delta) = value["delta"].as_str() {
                    result.push(InferenceEvent::TextDelta { text: delta.into() });
                }
            }
            "response.function_call_arguments.delta" => {
                result.push(InferenceEvent::ToolUseDelta {
                    id: value["call_id"]
                        .as_str()
                        .or_else(|| value["item_id"].as_str())
                        .unwrap_or("")
                        .into(),
                    name: value["name"].as_str().unwrap_or("").into(),
                    arguments_delta: value["delta"].as_str().unwrap_or("").into(),
                });
            }
            "response.completed" => {
                response = Some(parse_openai_response(
                    value.get("response").cloned().unwrap_or(value),
                )?);
            }
            "error" | "response.failed" | "response.incomplete" => {
                return Err(provider_payload_failure(
                    &format!("OpenAI Responses stream ended with {event_name}"),
                    &value,
                ));
            }
            _ => {}
        }
    }
    let response = response.context("OpenAI Responses stream ended before response.completed")?;
    result.push(InferenceEvent::Completed { response });
    Ok(result)
}

#[derive(Default)]
struct AnthropicBlock {
    kind: String,
    id: String,
    name: String,
    text: String,
    signature: String,
    redacted_data: Option<Value>,
    arguments: String,
}

fn parse_anthropic_sse(source: Vec<SseEvent>) -> Result<Vec<InferenceEvent>> {
    let mut result = Vec::new();
    let mut blocks: BTreeMap<u64, AnthropicBlock> = BTreeMap::new();
    let mut usage = InferenceUsage::default();
    let mut stop_reason = None;
    let mut stop_details = None;
    let mut completed = false;
    for event in source {
        if event.data.trim().is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(&event.data)?;
        let kind = event
            .event
            .as_deref()
            .or_else(|| value["type"].as_str())
            .unwrap_or("");
        match kind {
            "message_start" => {
                let message = &value["message"];
                usage.input_tokens = message["usage"]["input_tokens"].as_u64().unwrap_or(0);
                usage.cached_input_tokens = message["usage"]["cache_read_input_tokens"]
                    .as_u64()
                    .unwrap_or(0);
            }
            "content_block_start" => {
                let index = value["index"]
                    .as_u64()
                    .context("content block has no index")?;
                let source = &value["content_block"];
                let block = blocks.entry(index).or_default();
                block.kind = source["type"].as_str().unwrap_or("").into();
                block.id = source["id"].as_str().unwrap_or("").into();
                block.name = source["name"].as_str().unwrap_or("").into();
                block.text = source["text"]
                    .as_str()
                    .or_else(|| source["thinking"].as_str())
                    .unwrap_or("")
                    .into();
                block.signature = source["signature"].as_str().unwrap_or("").into();
                block.redacted_data = source.get("data").cloned();
                if source["input"].is_object() && source["input"] != json!({}) {
                    block.arguments = source["input"].to_string();
                }
            }
            "content_block_delta" => {
                let index = value["index"]
                    .as_u64()
                    .context("content delta has no index")?;
                let delta = &value["delta"];
                let block = blocks.entry(index).or_default();
                match delta["type"].as_str() {
                    Some("text_delta") => {
                        let text = delta["text"].as_str().unwrap_or("");
                        block.text.push_str(text);
                        result.push(InferenceEvent::TextDelta { text: text.into() });
                    }
                    Some("input_json_delta") => {
                        let part = delta["partial_json"].as_str().unwrap_or("");
                        block.arguments.push_str(part);
                        result.push(InferenceEvent::ToolUseDelta {
                            id: block.id.clone(),
                            name: block.name.clone(),
                            arguments_delta: part.into(),
                        });
                    }
                    Some("thinking_delta") => {
                        block.kind = "thinking".into();
                        block
                            .text
                            .push_str(delta["thinking"].as_str().unwrap_or(""));
                    }
                    Some("signature_delta") => {
                        block.kind = "thinking".into();
                        block
                            .signature
                            .push_str(delta["signature"].as_str().unwrap_or(""));
                    }
                    _ => {}
                }
            }
            "message_delta" => {
                stop_reason = value["delta"]["stop_reason"].as_str().map(Into::into);
                stop_details = value["delta"].get("stop_details").cloned();
                usage.output_tokens = value["usage"]["output_tokens"].as_u64().unwrap_or(0);
            }
            "message_stop" => completed = true,
            "error" => return Err(provider_payload_failure("Anthropic stream error", &value)),
            _ => {}
        }
    }
    anyhow::ensure!(completed, "Anthropic stream ended before message_stop");
    let mut content = Vec::new();
    let mut provider_blocks = Vec::new();
    for (_, block) in blocks {
        match block.kind.as_str() {
            "text" if !block.text.is_empty() => {
                content.push(ContentBlock::Text { text: block.text })
            }
            "tool_use" => {
                anyhow::ensure!(
                    !block.id.is_empty() && !block.name.is_empty(),
                    "incomplete tool_use"
                );
                content.push(ContentBlock::ToolUse {
                    id: block.id,
                    name: block.name,
                    arguments: serde_json::from_str(if block.arguments.is_empty() {
                        "{}"
                    } else {
                        &block.arguments
                    })?,
                });
            }
            "thinking" => provider_blocks.push(json!({
                "type": "thinking",
                "thinking": block.text,
                "signature": block.signature,
            })),
            "redacted_thinking" => provider_blocks.push(json!({
                "type": "redacted_thinking",
                "data": block.redacted_data.unwrap_or(Value::Null),
            })),
            "refusal" => content.push(ContentBlock::Refusal {
                category: None,
                text: (!block.text.is_empty()).then_some(block.text),
            }),
            _ => {}
        }
    }
    if let Some(details) = stop_details.filter(|value| !value.is_null()) {
        content.push(ContentBlock::Refusal {
            category: details["category"].as_str().map(Into::into),
            text: Some(hub_policy::redact(&details.to_string())),
        });
    }
    let response = InferenceResponse {
        content,
        usage,
        stop_reason: stop_reason.unwrap_or_else(|| "unknown".into()),
        provider_state: (!provider_blocks.is_empty()).then_some(Value::Array(provider_blocks)),
    };
    result.push(InferenceEvent::Completed { response });
    Ok(result)
}

#[cfg(test)]
fn parse_openai_chat(value: Value) -> Result<InferenceResponse> {
    let message = value["choices"]
        .as_array()
        .and_then(|v| v.first())
        .and_then(|v| v["message"].as_object())
        .context("OpenAI Chat response has no assistant message")?;
    let mut content = Vec::new();
    if let Some(text) = message
        .get("content")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
    {
        content.push(ContentBlock::Text { text: text.into() });
    }
    if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
        for call in calls {
            let id = call["id"].as_str().context("tool call has no id")?;
            let name = call["function"]["name"]
                .as_str()
                .context("tool call has no name")?;
            let arguments =
                serde_json::from_str(call["function"]["arguments"].as_str().unwrap_or("{}"))?;
            content.push(ContentBlock::ToolUse {
                id: id.into(),
                name: name.into(),
                arguments,
            });
        }
    }
    Ok(InferenceResponse {
        content,
        usage: openai_usage(&value),
        stop_reason: value["choices"][0]["finish_reason"]
            .as_str()
            .unwrap_or("unknown")
            .into(),
        provider_state: None,
    })
}

fn parse_openai_response(value: Value) -> Result<InferenceResponse> {
    let mut content = Vec::new();
    for item in value["output"]
        .as_array()
        .context("Responses API response has no output array")?
    {
        match item["type"].as_str() {
            Some("message") => {
                for part in item["content"].as_array().into_iter().flatten() {
                    match part["type"].as_str() {
                        Some("output_text") => {
                            if let Some(text) = part["text"].as_str() {
                                content.push(ContentBlock::Text { text: text.into() });
                            }
                        }
                        Some("refusal") => content.push(ContentBlock::Refusal {
                            category: None,
                            text: part["refusal"].as_str().map(Into::into),
                        }),
                        _ => {}
                    }
                }
            }
            Some("function_call") => {
                let arguments = serde_json::from_str(item["arguments"].as_str().unwrap_or("{}"))?;
                content.push(ContentBlock::ToolUse {
                    id: item["call_id"]
                        .as_str()
                        .context("function call has no call_id")?
                        .into(),
                    name: item["name"]
                        .as_str()
                        .context("function call has no name")?
                        .into(),
                    arguments,
                });
            }
            _ => {}
        }
    }
    Ok(InferenceResponse {
        content,
        usage: InferenceUsage {
            input_tokens: value["usage"]["input_tokens"].as_u64().unwrap_or(0),
            cached_input_tokens: value["usage"]["input_tokens_details"]["cached_tokens"]
                .as_u64()
                .unwrap_or(0),
            output_tokens: value["usage"]["output_tokens"].as_u64().unwrap_or(0),
        },
        stop_reason: value["status"].as_str().unwrap_or("unknown").into(),
        provider_state: None,
    })
}

#[cfg(test)]
fn parse_anthropic(value: Value) -> Result<InferenceResponse> {
    let mut content = Vec::new();
    let mut provider_state = Vec::new();
    for block in value["content"]
        .as_array()
        .context("Anthropic response has no content array")?
    {
        match block["type"].as_str() {
            Some("text") => {
                if let Some(text) = block["text"].as_str() {
                    content.push(ContentBlock::Text { text: text.into() });
                }
            }
            Some("tool_use") => content.push(ContentBlock::ToolUse {
                id: block["id"].as_str().context("tool_use has no id")?.into(),
                name: block["name"]
                    .as_str()
                    .context("tool_use has no name")?
                    .into(),
                arguments: block["input"].clone(),
            }),
            Some("refusal") => content.push(ContentBlock::Refusal {
                category: block["category"].as_str().map(Into::into),
                text: block["text"].as_str().map(Into::into),
            }),
            Some("thinking" | "redacted_thinking") => provider_state.push(block.clone()),
            _ => {}
        }
    }
    if let Some(details) = value.get("stop_details").filter(|v| !v.is_null()) {
        content.push(ContentBlock::Refusal {
            category: details["category"].as_str().map(Into::into),
            text: Some(hub_policy::redact(&details.to_string())),
        });
    }
    Ok(InferenceResponse {
        content,
        usage: InferenceUsage {
            input_tokens: value["usage"]["input_tokens"].as_u64().unwrap_or(0),
            cached_input_tokens: value["usage"]["cache_read_input_tokens"]
                .as_u64()
                .unwrap_or(0),
            output_tokens: value["usage"]["output_tokens"].as_u64().unwrap_or(0),
        },
        stop_reason: value["stop_reason"].as_str().unwrap_or("unknown").into(),
        provider_state: (!provider_state.is_empty()).then_some(Value::Array(provider_state)),
    })
}

fn openai_usage(value: &Value) -> InferenceUsage {
    InferenceUsage {
        input_tokens: value["usage"]["prompt_tokens"].as_u64().unwrap_or(0),
        cached_input_tokens: value["usage"]["prompt_tokens_details"]["cached_tokens"]
            .as_u64()
            .unwrap_or(0),
        output_tokens: value["usage"]["completion_tokens"].as_u64().unwrap_or(0),
    }
}

async fn limited_body(mut response: Response, label: &str) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| {
        provider_failure(
            ProviderFailureKind::StreamInterrupted,
            format!("{label} response stream failed: {error}"),
        )
    })? {
        if bytes.len() + chunk.len() > RESPONSE_LIMIT {
            return Err(provider_failure(
                ProviderFailureKind::ResponseTooLarge,
                format!("{label} response exceeds limit"),
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol_types::providers::ProviderLocality;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn local_profile(protocol: ProviderProtocol, base_url: String) -> ProviderProfile {
        ProviderProfile {
            id: "fixture".into(),
            name: "Fixture".into(),
            protocol,
            base_url: Some(base_url),
            locality: ProviderLocality::Local,
            credential_env: None,
            max_concurrency: 1,
            enabled: true,
            revision: 1,
        }
    }

    fn inference_request() -> InferenceRequest {
        InferenceRequest {
            target: protocol_types::providers::ModelTarget {
                profile_id: "fixture".into(),
                model_id: "fixture-model".into(),
                effort: None,
            },
            system: Some("system".into()),
            messages: vec![TranscriptMessage {
                role: TranscriptRole::User,
                content: vec![ContentBlock::Text {
                    text: "hello".into(),
                }],
                provider_state: None,
            }],
            tools: vec![],
            max_output_tokens: 100,
        }
    }

    async fn serve_once(
        status: &str,
        content_type: &str,
        chunks: Vec<Vec<u8>>,
    ) -> Result<(String, tokio::task::JoinHandle<Result<Vec<u8>>>)> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let status = status.to_owned();
        let content_type = content_type.to_owned();
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await?;
            let mut request = Vec::new();
            let mut buffer = [0_u8; 4096];
            let mut expected = None;
            loop {
                let count = socket.read(&mut buffer).await?;
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..count]);
                if expected.is_none() {
                    if let Some(header_end) =
                        request.windows(4).position(|part| part == b"\r\n\r\n")
                    {
                        let headers = String::from_utf8_lossy(&request[..header_end]);
                        let length = headers
                            .lines()
                            .find_map(|line| {
                                let (name, value) = line.split_once(':')?;
                                name.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>().ok())
                                    .flatten()
                            })
                            .unwrap_or(0);
                        expected = Some(header_end + 4 + length);
                    }
                }
                if expected.is_some_and(|length| request.len() >= length) {
                    break;
                }
            }
            let body_length: usize = chunks.iter().map(Vec::len).sum();
            socket.write_all(format!(
                "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {body_length}\r\nConnection: close\r\n\r\n"
            ).as_bytes()).await?;
            for chunk in chunks {
                socket.write_all(&chunk).await?;
                tokio::task::yield_now().await;
            }
            Ok(request)
        });
        Ok((format!("http://{address}/v1"), handle))
    }

    async fn serve_concurrent(
        requests: usize,
    ) -> Result<(
        String,
        Arc<AtomicUsize>,
        tokio::task::JoinHandle<Result<()>>,
    )> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let active = Arc::new(AtomicUsize::new(0));
        let maximum = Arc::new(AtomicUsize::new(0));
        let observed = maximum.clone();
        let handle = tokio::spawn(async move {
            let mut jobs = tokio::task::JoinSet::new();
            for _ in 0..requests {
                let (mut socket, _) = listener.accept().await?;
                let active = active.clone();
                let maximum = maximum.clone();
                jobs.spawn(async move {
                    let mut request = vec![0_u8; 16_384];
                    let _ = socket.read(&mut request).await?;
                    let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                    maximum.fetch_max(now, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(60)).await;
                    let body = concat!(
                        "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"done\"},\"finish_reason\":\"stop\"}]}\n\n",
                        "data: [DONE]\n\n"
                    );
                    socket
                        .write_all(
                            format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                                body.len()
                            )
                            .as_bytes(),
                        )
                        .await?;
                    active.fetch_sub(1, Ordering::SeqCst);
                    Result::<()>::Ok(())
                });
            }
            while let Some(result) = jobs.join_next().await {
                result??;
            }
            Ok(())
        });
        Ok((format!("http://{address}/v1"), observed, handle))
    }

    #[test]
    fn parses_each_native_response_shape() -> Result<()> {
        let chat = parse_openai_chat(
            json!({"choices":[{"message":{"content":"done","tool_calls":[{"id":"c1","type":"function","function":{"name":"read","arguments":"{\"path\":\"a\"}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":3,"completion_tokens":2}}),
        )?;
        assert_eq!(chat.content.len(), 2);
        let responses = parse_openai_response(
            json!({"status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"done"}]},{"type":"function_call","call_id":"c1","name":"read","arguments":"{}"}],"usage":{"input_tokens":4,"output_tokens":2}}),
        )?;
        assert_eq!(responses.content.len(), 2);
        let anthropic = parse_anthropic(
            json!({"content":[{"type":"text","text":"done"},{"type":"tool_use","id":"c1","name":"read","input":{}}],"stop_reason":"tool_use","usage":{"input_tokens":5,"output_tokens":2}}),
        )?;
        assert_eq!(anthropic.content.len(), 2);
        Ok(())
    }

    #[test]
    fn profile_construction_rejects_redirectable_or_insecure_cloud_urls() {
        let profile = ProviderProfile {
            id: "test".into(),
            name: "Test".into(),
            protocol: ProviderProtocol::OpenAiResponses,
            base_url: Some("http://api.example.com/v1".into()),
            locality: protocol_types::providers::ProviderLocality::Cloud,
            credential_env: None,
            max_concurrency: 1,
            enabled: true,
            revision: 1,
        };
        assert!(ApiTransport::new(profile).is_err());
    }

    #[test]
    fn parses_split_sse_shapes_and_rejects_cutoff() -> Result<()> {
        let chat = parse_openai_chat_sse(parse_sse(concat!(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"do\"}}]}\r\n\r\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"ne\",\"tool_calls\":[{\"index\":0,\"id\":\"c1\",\"function\":{\"name\":\"read\",\"arguments\":\"{\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"path\\\":\\\"a\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":2}}\n\n",
            "data: [DONE]\n\n"
        ))?)?;
        let completed = completed_response(chat)?;
        assert_eq!(completed.content.len(), 2);
        assert_eq!(completed.usage.output_tokens, 2);

        let responses = parse_openai_responses_sse(parse_sse(concat!(
            "event: response.output_text.delta\ndata: {\"delta\":\"done\"}\n\n",
            "event: future.unknown\ndata: {\"value\":1}\n\n",
            "event: response.completed\ndata: {\"response\":{\"status\":\"completed\",\"output\":[{\"type\":\"message\",\"content\":[{\"type\":\"output_text\",\"text\":\"done\"}]}],\"usage\":{\"input_tokens\":4,\"output_tokens\":2}}}\n\n"
        ))?)?;
        assert_eq!(completed_response(responses)?.usage.input_tokens, 4);

        let anthropic = parse_anthropic_sse(parse_sse(concat!(
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":5}}}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\",\"thinking\":\"\",\"signature\":\"\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"reason\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"signature_delta\",\"signature\":\"signed\"}}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"text_delta\",\"text\":\"done\"}}\n\n",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"refusal\",\"stop_details\":{\"category\":\"policy\",\"reason\":\"fixture\"}},\"usage\":{\"output_tokens\":2}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"
        ))?)?;
        let anthropic = completed_response(anthropic)?;
        assert_eq!(anthropic.stop_reason, "refusal");
        assert_eq!(
            anthropic.provider_state,
            Some(json!([{"type":"thinking","thinking":"reason","signature":"signed"}]))
        );
        assert!(matches!(
            anthropic.content.last(),
            Some(ContentBlock::Refusal { category: Some(category), text: Some(text) })
                if category == "policy" && text.contains("fixture")
        ));
        assert!(parse_openai_chat_sse(parse_sse(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"partial\"}}]}\n\n"
        )?)
        .is_err());
        Ok(())
    }

    #[test]
    fn normalizes_native_stream_error_payloads_and_malformed_json() -> Result<()> {
        let malformed = parse_openai_chat_sse(parse_sse("data: {not-json}\n\n")?).unwrap_err();
        assert_eq!(
            failure_kind(&normalize_provider_failure(malformed)),
            ProviderFailureKind::InvalidResponse
        );

        let rate_limited = parse_openai_chat_sse(parse_sse(
            "data: {\"error\":{\"type\":\"rate_limit_error\",\"message\":\"slow down\"}}\n\n",
        )?)
        .unwrap_err();
        assert_eq!(
            failure_kind(&rate_limited),
            ProviderFailureKind::RateLimited
        );

        let refused = parse_openai_responses_sse(parse_sse(concat!(
            "event: response.incomplete\n",
            "data: {\"response\":{\"incomplete_details\":{\"reason\":\"content_filter\"}}}\n\n"
        ))?)
        .unwrap_err();
        assert_eq!(failure_kind(&refused), ProviderFailureKind::Refusal);

        let context = parse_anthropic_sse(parse_sse(concat!(
            "event: error\n",
            "data: {\"type\":\"error\",\"error\":{\"type\":\"invalid_request_error\",\"message\":\"maximum context window exceeded\"}}\n\n"
        ))?)
        .unwrap_err();
        assert_eq!(failure_kind(&context), ProviderFailureKind::ContextLength);
        Ok(())
    }

    #[tokio::test]
    async fn http_transport_sends_streaming_request_and_handles_chunked_fixture() -> Result<()> {
        let body = concat!(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"done\"},\"finish_reason\":\"stop\"}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":2}}\n\n",
            "data: [DONE]\n\n"
        );
        let midpoint = body.len() / 2;
        let (base_url, server) = serve_once(
            "200 OK",
            "text/event-stream",
            vec![
                body.as_bytes()[..midpoint].to_vec(),
                body.as_bytes()[midpoint..].to_vec(),
            ],
        )
        .await?;
        let transport = ApiTransport::new(local_profile(ProviderProtocol::OpenAiChat, base_url))?;
        let response = transport.infer(inference_request()).await?;
        assert_eq!(response.usage.input_tokens, 3);
        let request = String::from_utf8(server.await??)?;
        assert!(request.starts_with("POST /v1/chat/completions HTTP/1.1"));
        assert!(request
            .to_ascii_lowercase()
            .contains("accept: text/event-stream"));
        let json: Value = serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap())?;
        assert_eq!(json["stream"], true);
        assert_eq!(json["stream_options"]["include_usage"], true);
        assert_eq!(json["messages"][0]["role"], "system");
        Ok(())
    }

    #[tokio::test]
    async fn http_errors_are_redacted_and_response_limit_is_enforced() -> Result<()> {
        let (base_url, server) = serve_once(
            "401 Unauthorized",
            "application/json",
            vec![br#"{"error":"sk-abcdefgh12345678"}"#.to_vec()],
        )
        .await?;
        let transport = ApiTransport::new(local_profile(ProviderProtocol::OpenAiChat, base_url))?;
        let error = transport.infer(inference_request()).await.unwrap_err();
        server.await??;
        assert_eq!(failure_kind(&error), ProviderFailureKind::Authentication);
        assert!(error.to_string().contains("[REDACTED]"));
        assert!(!error.to_string().contains("sk-abcdefgh12345678"));

        let oversized = vec![b'x'; RESPONSE_LIMIT + 1];
        let (base_url, server) = serve_once("200 OK", "text/event-stream", vec![oversized]).await?;
        let transport = ApiTransport::new(local_profile(ProviderProtocol::OpenAiChat, base_url))?;
        let error = transport.infer(inference_request()).await.unwrap_err();
        assert_eq!(failure_kind(&error), ProviderFailureKind::ResponseTooLarge);
        assert!(error.to_string().contains("response exceeds limit"));
        server.await??;

        let (base_url, server) = serve_once(
            "429 Too Many Requests",
            "application/json",
            vec![br#"{"error":"rate limited"}"#.to_vec()],
        )
        .await?;
        let transport = ApiTransport::new(local_profile(ProviderProtocol::OpenAiChat, base_url))?;
        let error = transport.infer(inference_request()).await.unwrap_err();
        assert_eq!(failure_kind(&error), ProviderFailureKind::RateLimited);
        assert!(error.to_string().contains("429 Too Many Requests"));
        server.await??;

        let (base_url, server) = serve_once(
            "400 Bad Request",
            "application/json",
            vec![br#"{"error":{"code":"context_length_exceeded"}}"#.to_vec()],
        )
        .await?;
        let transport = ApiTransport::new(local_profile(ProviderProtocol::OpenAiChat, base_url))?;
        let error = transport.infer(inference_request()).await.unwrap_err();
        assert_eq!(failure_kind(&error), ProviderFailureKind::ContextLength);
        server.await??;
        Ok(())
    }

    #[tokio::test]
    async fn concurrency_is_limited_per_profile_but_not_shared_between_profiles() -> Result<()> {
        let (base_url, maximum, server) = serve_concurrent(2).await?;
        let transport = Arc::new(ApiTransport::new(local_profile(
            ProviderProtocol::OpenAiChat,
            base_url,
        ))?);
        let (first, second) = tokio::join!(
            transport.infer(inference_request()),
            transport.infer(inference_request())
        );
        first?;
        second?;
        server.await??;
        assert_eq!(maximum.load(Ordering::SeqCst), 1);

        let (base_url, maximum, server) = serve_concurrent(2).await?;
        let mut left_profile = local_profile(ProviderProtocol::OpenAiChat, base_url.clone());
        left_profile.id = "left".into();
        let mut right_profile = local_profile(ProviderProtocol::OpenAiChat, base_url);
        right_profile.id = "right".into();
        let left = ApiTransport::new(left_profile)?;
        let right = ApiTransport::new(right_profile)?;
        let mut left_request = inference_request();
        left_request.target.profile_id = "left".into();
        let mut right_request = inference_request();
        right_request.target.profile_id = "right".into();
        let (first, second) = tokio::join!(left.infer(left_request), right.infer(right_request));
        first?;
        second?;
        server.await??;
        assert_eq!(maximum.load(Ordering::SeqCst), 2);
        Ok(())
    }

    #[test]
    fn authentication_headers_are_protocol_specific_without_exposing_secret() -> Result<()> {
        let env_name = "LOCALOUD_PROVIDER_API_TEST_KEY";
        std::env::set_var(env_name, "sk-abcdefgh12345678");
        let mut profile = ProviderProfile {
            id: "auth".into(),
            name: "Auth".into(),
            protocol: ProviderProtocol::OpenAiResponses,
            base_url: Some("https://api.example.test/v1".into()),
            locality: ProviderLocality::Cloud,
            credential_env: Some(env_name.into()),
            max_concurrency: 1,
            enabled: true,
            revision: 1,
        };
        let transport = ApiTransport::new(profile.clone())?;
        let request = transport
            .authenticate(transport.client.get(transport.endpoint("models")?))?
            .build()?;
        assert_eq!(
            request.headers()[header::AUTHORIZATION],
            "Bearer sk-abcdefgh12345678"
        );
        profile.protocol = ProviderProtocol::AnthropicMessages;
        let transport = ApiTransport::new(profile)?;
        let request = transport
            .authenticate(transport.client.get(transport.endpoint("models")?))?
            .build()?;
        assert_eq!(request.headers()["x-api-key"], "sk-abcdefgh12345678");
        std::env::remove_var(env_name);
        Ok(())
    }
}
