use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

macro_rules! id {
    ($($name:ident),*) => {$ (
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub Uuid);
        impl Default for $name { fn default() -> Self { Self(Uuid::new_v4()) } }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { self.0.fmt(f) }
        }
    )*};
}
id!(
    ProjectId,
    PlanId,
    TaskId,
    WorkerId,
    ContextCapsuleId,
    HubThreadId,
    WorktreeId
);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub root: PathBuf,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutorKind {
    Spark,
    Codex,
    Astra,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutorPreference {
    Auto,
    Spark,
    Codex,
    Astra,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Complexity {
    Trivial,
    Normal,
    Deep,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Blocked,
    Ready,
    Running,
    Reviewing,
    Completed,
    Failed,
    Cancelled,
}
impl TaskStatus {
    pub fn can_transition(self, next: Self) -> bool {
        use TaskStatus::*;
        matches!(
            (self, next),
            (Pending, Ready | Blocked | Cancelled)
                | (Blocked, Ready | Cancelled)
                | (Ready, Running | Cancelled)
                | (Running, Reviewing | Failed | Cancelled)
                | (Reviewing, Completed | Running | Failed | Cancelled)
                | (Failed, Ready | Cancelled)
        )
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    pub project_id: ProjectId,
    pub plan_id: Option<PlanId>,
    pub parent_task_id: Option<TaskId>,
    pub title: String,
    pub description: String,
    pub dependencies: Vec<TaskId>,
    pub preferred_executor: ExecutorPreference,
    pub assigned_executor: Option<ExecutorKind>,
    pub risk: RiskLevel,
    pub complexity: Complexity,
    pub status: TaskStatus,
    pub context_capsule_id: Option<ContextCapsuleId>,
    pub worktree_id: Option<WorktreeId>,
    pub attempts: u32,
    pub max_attempts: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadMapping {
    pub id: HubThreadId,
    pub project_id: ProjectId,
    pub provider: String,
    pub provider_thread_id: String,
    pub title: String,
    pub status: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextBudget {
    pub initial_tokens: u32,
    pub max_total_tokens: u32,
    pub max_single_retrieval_tokens: u32,
}
impl ContextBudget {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.initial_tokens == 0
            || self.initial_tokens > self.max_total_tokens
            || self.max_single_retrieval_tokens > self.max_total_tokens
        {
            return Err("invalid context budget");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextItem {
    pub source: String,
    pub reference: String,
    pub text: String,
    pub token_estimate: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextCapsule {
    pub id: ContextCapsuleId,
    pub task_id: TaskId,
    pub project_id: ProjectId,
    pub goal: String,
    pub acceptance_criteria: Vec<String>,
    pub constraints: Vec<String>,
    pub items: Vec<ContextItem>,
    pub budget: ContextBudget,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AstraAccessMode {
    ChatGptIntegrated,
    CodexIntegrated,
    OpenAiApi,
    Disabled,
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn completed_is_terminal() {
        assert!(!TaskStatus::Completed.can_transition(TaskStatus::Running));
        assert!(TaskStatus::Failed.can_transition(TaskStatus::Ready));
    }
    #[test]
    fn invalid_budget_rejected() {
        assert!(ContextBudget {
            initial_tokens: 9,
            max_total_tokens: 8,
            max_single_retrieval_tokens: 1
        }
        .validate()
        .is_err());
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelUsageRecord {
    pub id: String,
    pub provider: String,
    pub model: String,
    pub task_id: Option<TaskId>,
    pub turn_id: Option<String>,
    pub prompt_tokens: Option<u64>,
    pub cached_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub estimated_cost: Option<f64>,
    pub latency_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Worktree {
    pub id: WorktreeId,
    pub task_id: TaskId,
    pub path: PathBuf,
    pub base_sha: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextRetrieval {
    pub id: String,
    pub task_id: TaskId,
    pub source: String,
    pub query: String,
    pub token_budget: u32,
    pub token_estimate: u32,
    pub result_ref: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextInspection {
    pub capsule: ContextCapsule,
    pub initial_tokens: u32,
    pub retrieved_tokens: u32,
    pub current_estimate: u32,
    pub retrievals: Vec<ContextRetrieval>,
    pub parent_conversation_inherited: bool,
}

impl Task {
    pub fn new(
        project_id: ProjectId,
        title: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            id: TaskId::default(),
            project_id,
            plan_id: None,
            parent_task_id: None,
            title: title.into(),
            description: description.into(),
            dependencies: Vec::new(),
            preferred_executor: ExecutorPreference::Auto,
            assigned_executor: None,
            risk: RiskLevel::Medium,
            complexity: Complexity::Normal,
            status: TaskStatus::Pending,
            context_capsule_id: None,
            worktree_id: None,
            attempts: 0,
            max_attempts: 2,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    Decision,
    Reason,
    RejectedApproach,
    Failure,
    Constraint,
    Warning,
    Result,
    UnresolvedIssue,
    NextAction,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryCandidate {
    pub kind: MemoryKind,
    pub durable: bool,
    pub statement: String,
    pub reason: String,
    pub source_refs: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryExtraction {
    pub candidates: Vec<MemoryCandidate>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DurableMemoryItem {
    pub external_key: String,
    pub project_id: ProjectId,
    pub task_id: TaskId,
    pub candidate: MemoryCandidate,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySearchRequest {
    pub query: String,
    pub limit: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryItem {
    pub id: String,
    pub text: String,
    pub source_refs: Vec<String>,
}
