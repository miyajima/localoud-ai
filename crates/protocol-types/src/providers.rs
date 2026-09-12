use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderProtocol {
    CodexAppServer,
    LocalDedicated,
    OpenAiChat,
    OpenAiResponses,
    AnthropicMessages,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderLocality {
    Local,
    Cloud,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderProfile {
    pub id: String,
    pub name: String,
    pub protocol: ProviderProtocol,
    pub base_url: Option<String>,
    pub locality: ProviderLocality,
    pub credential_env: Option<String>,
    pub max_concurrency: u8,
    pub enabled: bool,
    pub revision: u64,
}

impl ProviderProfile {
    pub fn validate(&self) -> Result<()> {
        if self.id.trim().is_empty()
            || self.id.len() > 80
            || !self
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        {
            bail!("provider profile ID must use 1-80 letters, numbers, '-' or '_'");
        }
        if self.name.trim().is_empty() || self.name.chars().count() > 120 {
            bail!("provider profile name must use 1-120 characters");
        }
        if !(1..=8).contains(&self.max_concurrency) {
            bail!("provider concurrency must be 1-8");
        }
        if let Some(name) = &self.credential_env {
            let mut bytes = name.bytes();
            if name.len() > 128
                || !bytes
                    .next()
                    .is_some_and(|b| b == b'_' || b.is_ascii_uppercase())
                || !bytes.all(|b| b == b'_' || b.is_ascii_uppercase() || b.is_ascii_digit())
            {
                bail!("credential environment variable name is invalid");
            }
        }
        match self.protocol {
            ProviderProtocol::CodexAppServer => {
                if self.locality != ProviderLocality::Local
                    || self.base_url.is_some()
                    || self.credential_env.is_some()
                {
                    bail!("Codex app-server does not accept API endpoint credentials");
                }
            }
            ProviderProtocol::LocalDedicated
            | ProviderProtocol::OpenAiChat
            | ProviderProtocol::OpenAiResponses
            | ProviderProtocol::AnthropicMessages => {
                let raw = self
                    .base_url
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("provider profile requires a base URL"))?;
                let url = reqwest::Url::parse(raw)?;
                if !url.username().is_empty()
                    || url.password().is_some()
                    || url.query().is_some()
                    || url.fragment().is_some()
                {
                    bail!("provider URL cannot contain credentials, query, or fragment");
                }
                let loopback = matches!(url.host_str(), Some("127.0.0.1" | "localhost"));
                if self.locality == ProviderLocality::Local {
                    if url.scheme() != "http" || !loopback || self.credential_env.is_some() {
                        bail!("local providers require credential-free loopback HTTP");
                    }
                } else if url.scheme() != "https" {
                    bail!("cloud providers require HTTPS");
                }
                if self.protocol == ProviderProtocol::LocalDedicated
                    && self.locality != ProviderLocality::Local
                {
                    bail!("dedicated local protocol requires local loopback transport");
                }
                if self.protocol == ProviderProtocol::AnthropicMessages
                    && self.locality != ProviderLocality::Cloud
                {
                    bail!("Anthropic Messages requires a cloud HTTPS profile");
                }
            }
        }
        Ok(())
    }

    pub fn credential_present(&self) -> bool {
        self.credential_env
            .as_ref()
            .is_none_or(|name| std::env::var_os(name).is_some())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelTarget {
    pub profile_id: String,
    pub model_id: String,
    pub effort: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetResolution {
    DifficultyDefault,
    PlannerDefault,
    SessionOverride,
    PlanStepOverride,
    ReviewerDefault,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionModelPolicy {
    pub default_target: ModelTarget,
    pub reviewer_target: Option<ModelTarget>,
    pub allow_turn_override: bool,
}

impl SessionModelPolicy {
    pub fn validate(&self) -> Result<()> {
        self.default_target.validate()?;
        if let Some(target) = &self.reviewer_target {
            target.validate()?;
        }
        Ok(())
    }
}

impl ModelTarget {
    pub fn validate(&self) -> Result<()> {
        if self.profile_id.trim().is_empty()
            || self.model_id.trim().is_empty()
            || self.profile_id.len() > 80
            || self.model_id.len() > 300
            || self
                .effort
                .as_ref()
                .is_some_and(|v| v.trim().is_empty() || v.len() > 50)
        {
            bail!("provider, model, or effort selection is invalid");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelCapabilities {
    pub tools: bool,
    pub streaming: bool,
    pub structured_output: bool,
    pub efforts: Vec<String>,
    pub max_input_tokens: Option<u64>,
    pub max_output_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderModel {
    pub id: String,
    pub display_name: String,
    pub capabilities: ModelCapabilities,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        arguments: Value,
    },
    ToolResult {
        tool_use_id: String,
        text: String,
        is_error: bool,
    },
    Refusal {
        category: Option<String>,
        text: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TranscriptMessage {
    pub role: TranscriptRole,
    pub content: Vec<ContentBlock>,
    /// Provider-native state (for example Anthropic thinking blocks). It may
    /// only be replayed inside the segment that produced it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_state: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InferenceTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InferenceRequest {
    pub target: ModelTarget,
    pub system: Option<String>,
    pub messages: Vec<TranscriptMessage>,
    pub tools: Vec<InferenceTool>,
    pub max_output_tokens: u32,
}

impl InferenceRequest {
    pub fn validate(&self) -> Result<()> {
        self.target.validate()?;
        if self.messages.is_empty() {
            bail!("inference request requires at least one message");
        }
        if self.max_output_tokens == 0 || self.max_output_tokens > 131_072 {
            bail!("max output tokens must be 1-131072");
        }
        if self.tools.len() > 128 {
            bail!("inference request has too many tools");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InferenceUsage {
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InferenceResponse {
    pub content: Vec<ContentBlock>,
    pub usage: InferenceUsage,
    pub stop_reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_state: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InferenceEvent {
    TextDelta {
        text: String,
    },
    ToolUseDelta {
        id: String,
        name: String,
        arguments_delta: String,
    },
    Completed {
        response: InferenceResponse,
    },
}
