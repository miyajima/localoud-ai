//! A2A v1 message primitives used at Localoud agent boundaries.
//!
//! Provider adapters still translate these envelopes into each provider's
//! native request format. A2A is the handoff contract, not an LLM API format.

use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const PROTOCOL_VERSION: &str = "1.0";
pub const CONTEXT_CAPSULE_EXTENSION: &str = "urn:localoud:a2a:context-capsule:v1";
pub const CONTEXT_CAPSULE_MEDIA_TYPE: &str = "application/vnd.localoud.context-capsule+json";
pub const REVIEW_CAPSULE_MEDIA_TYPE: &str = "application/vnd.localoud.review-capsule+json";
pub const RESEARCH_CAPSULE_MEDIA_TYPE: &str = "application/vnd.localoud.research-capsule+json";
pub const READ_EXTENSION: &str = "urn:localoud:a2a:read-evidence:v1";
pub const READ_REQUEST_MEDIA_TYPE: &str = "application/vnd.localoud.read-request+json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    #[serde(rename = "ROLE_USER")]
    User,
    #[serde(rename = "ROLE_AGENT")]
    Agent,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Part {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub metadata: Map<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
}

impl Part {
    pub fn text(value: impl Into<String>) -> Self {
        Self {
            text: Some(value.into()),
            raw: None,
            url: None,
            data: None,
            metadata: Map::new(),
            filename: None,
            media_type: Some("text/plain".into()),
        }
    }

    pub fn data(value: Value, media_type: impl Into<String>) -> Self {
        Self {
            text: None,
            raw: None,
            url: None,
            data: Some(value),
            metadata: Map::new(),
            filename: None,
            media_type: Some(media_type.into()),
        }
    }

    pub fn validate(&self) -> Result<()> {
        let content_fields = [
            self.text.is_some(),
            self.raw.is_some(),
            self.url.is_some(),
            self.data.is_some(),
        ]
        .into_iter()
        .filter(|present| *present)
        .count();
        ensure!(
            content_fields == 1,
            "A2A Part must contain exactly one of text, raw, url, or data"
        );
        if let Some(media_type) = &self.media_type {
            ensure!(
                !media_type.trim().is_empty() && media_type.len() <= 200,
                "A2A Part mediaType is invalid"
            );
        }
        if let Some(filename) = &self.filename {
            ensure!(
                !filename.trim().is_empty()
                    && filename.len() <= 500
                    && !filename.chars().any(char::is_control),
                "A2A Part filename is invalid"
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub message_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    pub role: Role,
    pub parts: Vec<Part>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub metadata: Map<String, Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reference_task_ids: Vec<String>,
}

impl Message {
    pub fn validate(&self) -> Result<()> {
        ensure!(valid_id(&self.message_id), "A2A messageId is invalid");
        ensure!(
            !self.parts.is_empty(),
            "A2A Message needs at least one Part"
        );
        for id in [self.context_id.as_deref(), self.task_id.as_deref()]
            .into_iter()
            .flatten()
            .chain(self.reference_task_ids.iter().map(String::as_str))
        {
            ensure!(valid_id(id), "A2A context or task identifier is invalid");
        }
        ensure!(
            self.extensions
                .iter()
                .all(|value| !value.trim().is_empty() && value.len() <= 500),
            "A2A extension URI is invalid"
        );
        for part in &self.parts {
            part.validate()?;
        }
        Ok(())
    }
}

fn valid_id(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 500 && !value.chars().any(char::is_control)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendMessageRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant: Option<String>,
    pub message: Message,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub configuration: Option<SendMessageConfiguration>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub metadata: Map<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendMessageConfiguration {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accepted_output_modes: Vec<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub return_immediately: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

pub fn data_message(
    message_id: impl Into<String>,
    context_id: Option<String>,
    task_id: Option<String>,
    data: Value,
    media_type: impl Into<String>,
    reference_task_ids: Vec<String>,
) -> Result<Message> {
    let message = Message {
        message_id: message_id.into(),
        context_id,
        task_id,
        role: Role::User,
        parts: vec![Part::data(data, media_type)],
        metadata: Map::new(),
        extensions: vec![CONTEXT_CAPSULE_EXTENSION.into()],
        reference_task_ids,
    };
    message.validate()?;
    Ok(message)
}

/// Adapt an A2A envelope for providers that accept only text prompts.
pub fn render_for_native_provider(instruction: &str, message: &Message) -> Result<String> {
    message.validate()?;
    ensure!(
        !instruction.trim().is_empty(),
        "native adapter instruction is empty"
    );
    Ok(format!(
        "{instruction}\n\n<a2a-message protocol-version=\"{PROTOCOL_VERSION}\">\n{}\n</a2a-message>",
        serde_json::to_string_pretty(message)?
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn v1_message_uses_camel_case_and_flattened_data_part() -> Result<()> {
        let message = data_message(
            "message-1",
            Some("context-1".into()),
            Some("task-1".into()),
            json!({"goal":"test"}),
            CONTEXT_CAPSULE_MEDIA_TYPE,
            vec!["dependency-1".into()],
        )?;
        let value = serde_json::to_value(&message)?;
        assert_eq!(value["role"], "ROLE_USER");
        assert_eq!(value["messageId"], "message-1");
        assert_eq!(value["referenceTaskIds"][0], "dependency-1");
        assert_eq!(value["parts"][0]["data"]["goal"], "test");
        assert!(value["parts"][0].get("kind").is_none());
        Ok(())
    }

    #[test]
    fn part_rejects_multiple_content_fields() {
        let mut part = Part::text("test");
        part.data = Some(json!({}));
        assert!(part.validate().is_err());
    }
}
