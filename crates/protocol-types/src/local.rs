use anyhow::Result;
use async_trait::async_trait;
use hub_core::{Complexity, ExecutorKind, RiskLevel};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingInput {
    pub request: String,
    pub known_files: Vec<String>,
    pub estimated_loc: Option<u32>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EstimatedScope {
    pub files: u32,
    pub loc: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingDecision {
    pub executor: ExecutorKind,
    pub complexity: Complexity,
    pub risk: RiskLevel,
    pub needs_plan: bool,
    pub needs_final_astra_review: bool,
    pub estimated_scope: EstimatedScope,
    pub confidence: f64,
    pub reason: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub latency_ms: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Generation<T> {
    pub output: T,
    pub usage: LocalUsage,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgressSummary {
    pub status: String,
    pub summary: Vec<String>,
    pub blockers: Vec<String>,
    pub scope_drift: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewResult {
    pub verdict: ReviewVerdict,
    pub findings: Vec<String>,
    pub summary: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewVerdict {
    Approve,
    Rework,
    HumanRequired,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileContext {
    pub path: String,
    pub content: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Edit {
    pub path: String,
    pub old: String,
    pub new: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditProposal {
    pub edits: Vec<Edit>,
    pub summary: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapsuleDraft {
    pub goal: String,
    pub acceptance_criteria: Vec<String>,
    pub constraints: Vec<String>,
    pub selected_refs: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetrievalQuery {
    pub query: String,
    pub source: String,
}
#[async_trait]
pub trait LocalModelProvider: Send + Sync {
    fn model_id(&self) -> &str {
        "unknown"
    }
    async fn extract_memory(
        &self,
        _summary: &str,
        _evidence: &[String],
    ) -> Result<Generation<hub_core::MemoryExtraction>> {
        anyhow::bail!("memory extraction is unavailable")
    }
    async fn available(&self) -> Result<()>;
    async fn classify_task(&self, input: &RoutingInput) -> Result<Generation<RoutingDecision>>;
    async fn implement(
        &self,
        request: &str,
        files: &[FileContext],
    ) -> Result<Generation<EditProposal>>;
    async fn summarize_progress(&self, events: &[String]) -> Result<Generation<ProgressSummary>>;
    async fn review_diff(&self, request: &str, diff: &str) -> Result<Generation<ReviewResult>>;
    async fn draft_context(
        &self,
        request: &str,
        available_refs: &[String],
    ) -> Result<Generation<CapsuleDraft>>;
    async fn generate_retrieval_query(&self, need: &str) -> Result<Generation<RetrievalQuery>>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteReport {
    pub decision: RoutingDecision,
    pub usage: Option<LocalUsage>,
    pub fallback: bool,
}
