//! Provider-neutral runtime contracts. No Codex wire JSON types.
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::sync::broadcast;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderThread {
    pub id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderTurn {
    pub thread_id: String,
    pub id: String,
    pub status: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub text: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadSnapshot {
    pub thread: ProviderThread,
    pub active_turn: Option<ProviderTurn>,
    pub messages: Vec<Message>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEvent {
    #[serde(default)]
    pub details: Option<EventDetails>,
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
    pub item_id: Option<String>,
    pub kind: String,
    pub text: String,
}
#[async_trait]
pub trait CodingAgentProvider: Send + Sync {
    async fn start_worker(
        &self,
        root: PathBuf,
        tools: Vec<ToolDefinition>,
        handler: std::sync::Arc<dyn AgentTool>,
    ) -> Result<ProviderThread>;

    async fn attach_worker(
        &self,
        thread: &ProviderThread,
        handler: std::sync::Arc<dyn AgentTool>,
    ) -> Result<()>;
    async fn start_thread(&self, root: PathBuf) -> Result<ProviderThread>;
    async fn start_thread_with_model(
        &self,
        _root: PathBuf,
        _model: &str,
    ) -> Result<ProviderThread> {
        anyhow::bail!("Explicit model selection is unavailable for this provider")
    }
    async fn resume_thread(&self, thread: &ProviderThread, root: PathBuf)
        -> Result<ThreadSnapshot>;
    async fn resume_thread_with_model(
        &self,
        _thread: &ProviderThread,
        _root: PathBuf,
        _model: &str,
    ) -> Result<ThreadSnapshot> {
        anyhow::bail!("Pinned model resume is unavailable for this provider")
    }
    async fn read_thread(&self, thread: &ProviderThread) -> Result<ThreadSnapshot>;
    async fn start_turn(&self, thread: &ProviderThread, text: String) -> Result<ProviderTurn>;
    async fn start_turn_with_reasoning(
        &self,
        thread: &ProviderThread,
        text: String,
        _model: &str,
        effort: Option<&str>,
    ) -> Result<ProviderTurn> {
        if effort.is_some() {
            anyhow::bail!("Reasoning selection is unavailable for this provider");
        }
        self.start_turn(thread, text).await
    }
    async fn steer_turn(&self, turn: &ProviderTurn, text: String) -> Result<()>;
    async fn interrupt_turn(&self, turn: &ProviderTurn) -> Result<()>;
    fn events(&self) -> broadcast::Receiver<AgentEvent>;
}

pub mod local;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventDetails {
    Tool {
        name: String,
        arguments: serde_json::Value,
        success: Option<bool>,
    },
    Usage {
        model: Option<String>,
        last_input_tokens: u64,
        last_cached_tokens: u64,
        last_output_tokens: u64,
        total_input_tokens: u64,
        total_cached_tokens: u64,
        total_output_tokens: u64,
    },
    Command {
        command: String,
        exit_code: Option<i32>,
    },
}
#[derive(Debug, Clone)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub schema: serde_json::Value,
}
#[derive(Debug, Clone)]
pub struct ToolCall {
    pub name: String,
    pub arguments: serde_json::Value,
    pub thread_id: String,
    pub turn_id: String,
}
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub text: String,
    pub success: bool,
}
#[async_trait]
pub trait AgentTool: Send + Sync {
    async fn call(&self, request: ToolCall) -> Result<ToolResult>;
}
