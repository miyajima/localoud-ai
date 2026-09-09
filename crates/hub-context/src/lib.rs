pub mod handoff;
use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use cap_std::{ambient_authority, fs::Dir};
use hub_core::*;
use hub_db::Store;
use protocol_types::{AgentTool, ToolCall, ToolDefinition, ToolResult};
use serde::{Deserialize, Serialize};
use std::{
    path::{Component, PathBuf},
    sync::{Arc, Mutex},
};

/// An explicitly labeled estimate, not a claim about the provider's tokenizer.
pub fn estimate_tokens(text: &str) -> u32 {
    text.len().div_ceil(3).try_into().unwrap_or(u32::MAX)
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextSource {
    Repo,
    SessionStore,
    OrgBrain,
    DecisionStore,
    TaskHistory,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextRequest {
    pub source: ContextSource,
    pub query: String,
    pub path: Option<String>,
    pub token_budget: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalResult {
    pub source: ContextSource,
    pub references: Vec<String>,
    pub text: String,
    pub truncated: bool,
}
/// Pure preflight, shared by plan validation and persisted capsule construction.
pub fn prepare_capsule(
    task: TaskId,
    project: ProjectId,
    goal: String,
    acceptance: Vec<String>,
    constraints: Vec<String>,
    mut items: Vec<ContextItem>,
    budget: ContextBudget,
) -> Result<ContextCapsule> {
    budget.validate().map_err(|e| anyhow!(e))?;
    if goal.trim().is_empty() {
        bail!("capsule goal is empty");
    }
    for i in &mut items {
        if hub_policy::redact(&i.text) != i.text {
            bail!("context item contains secret-like text");
        }
        i.token_estimate = estimate_tokens(&i.text);
    }
    if hub_policy::redact(&goal) != goal {
        bail!("goal contains secret-like text");
    }
    let c = ContextCapsule {
        id: ContextCapsuleId::default(),
        task_id: task,
        project_id: project,
        goal,
        acceptance_criteria: acceptance,
        constraints,
        items,
        budget,
    };
    let serialized = serde_json::to_string(&c)?;
    if hub_policy::redact(&serialized) != serialized {
        bail!("capsule contains secret-like text");
    }
    let initial = estimate_tokens(&serialized);
    if initial > c.budget.initial_tokens {
        bail!(
            "initial capsule exceeds budget ({initial} > {})",
            c.budget.initial_tokens
        );
    }
    Ok(c)
}
pub struct Broker {
    pub store: Arc<Mutex<Store>>,
    pub task: TaskId,
    pub project: ProjectId,
    pub root: PathBuf,
    pub memory: Option<Arc<dyn hub_memory::DurableMemoryProvider>>,
}
impl Broker {
    pub fn build_initial(
        &self,
        goal: String,
        acceptance: Vec<String>,
        constraints: Vec<String>,
        items: Vec<ContextItem>,
        budget: ContextBudget,
    ) -> Result<ContextCapsule> {
        let c = prepare_capsule(
            self.task,
            self.project,
            goal,
            acceptance,
            constraints,
            items,
            budget,
        )?;
        let initial = estimate_tokens(&serde_json::to_string(&c)?);
        self.store
            .lock()
            .map_err(|_| anyhow!("database lock poisoned"))?
            .save_capsule(&c, initial)?;
        Ok(c)
    }
    pub fn inspect(&self) -> Result<ContextInspection> {
        let store = self
            .store
            .lock()
            .map_err(|_| anyhow!("database lock poisoned"))?;
        let (capsule, initial_tokens) = store.capsule(self.task)?.context("capsule not found")?;
        let retrievals = store.retrievals(self.task)?;
        let retrieved_tokens = retrievals.iter().map(|r| r.token_estimate).sum::<u32>();
        Ok(ContextInspection {
            capsule,
            initial_tokens,
            retrieved_tokens,
            current_estimate: initial_tokens.saturating_add(retrieved_tokens),
            retrievals,
            parent_conversation_inherited: false,
        })
    }
    pub fn retrieve(&self, request: ContextRequest) -> Result<RetrievalResult> {
        let result = self.retrieve_inner(request.clone());
        if let Err(error) = &result {
            let denial = ContextRetrieval {
                id: uuid::Uuid::new_v4().to_string(),
                task_id: self.task,
                source: serde_json::to_string(&request.source)?
                    .trim_matches('"')
                    .into(),
                query: hub_policy::redact(&request.query),
                token_budget: request.token_budget,
                token_estimate: 0,
                result_ref: format!("denied: {error}"),
            };
            self.store
                .lock()
                .map_err(|_| anyhow!("database lock poisoned"))?
                .record_retrieval_denial(&denial)?;
        }
        result
    }
    fn retrieve_inner(&self, request: ContextRequest) -> Result<RetrievalResult> {
        if request.query.trim().is_empty()
            || request.query.len() > 1000
            || request.token_budget < 64
        {
            bail!("query and at least 64 tokens are required");
        }
        let inspection = self.inspect()?;
        if request.token_budget > inspection.capsule.budget.max_single_retrieval_tokens {
            bail!("single retrieval budget exceeded");
        }
        let remaining = inspection
            .capsule
            .budget
            .max_total_tokens
            .saturating_sub(inspection.current_estimate);
        if request.token_budget > remaining {
            bail!("requested retrieval exceeds remaining budget");
        }
        let entries: Vec<(String, String)> = match request.source {
            ContextSource::Repo => {
                let path = request
                    .path
                    .as_ref()
                    .context("repo retrieval requires an explicit relative path")?;
                if path.is_empty()
                    || std::path::Path::new(path)
                        .components()
                        .any(|c| !matches!(c, Component::Normal(_)))
                {
                    bail!("invalid repository path");
                }
                for c in std::path::Path::new(path).components() {
                    let n = c.as_os_str().to_string_lossy().to_lowercase();
                    if n == ".git"
                        || n == ".ssh"
                        || n == ".codex"
                        || n.starts_with(".env")
                        || n.ends_with(".key")
                        || n.ends_with(".pem")
                    {
                        bail!("sensitive path cannot be retrieved");
                    }
                }
                let dir = Dir::open_ambient_dir(&self.root, ambient_authority())?;
                if dir.symlink_metadata(path)?.file_type().is_symlink()
                    || dir.metadata(path)?.len() > 512000
                {
                    bail!("path is not a bounded regular source file");
                }
                let text = dir.read_to_string(path)?;
                vec![(path.clone(), text)]
            }
            ContextSource::DecisionStore => self
                .store
                .lock()
                .map_err(|_| anyhow!("database lock poisoned"))?
                .search_decisions(self.project, &request.query)?,
            ContextSource::SessionStore => self
                .store
                .lock()
                .map_err(|_| anyhow!("database lock poisoned"))?
                .search_session(self.project, &request.query)?,
            ContextSource::TaskHistory => self
                .store
                .lock()
                .map_err(|_| anyhow!("database lock poisoned"))?
                .tasks(self.project)?
                .into_iter()
                .filter(|t| {
                    t.description
                        .to_lowercase()
                        .contains(&request.query.to_lowercase())
                })
                .take(8)
                .map(|t| {
                    (
                        t.id.to_string(),
                        format!("{}: {:?}\n{}", t.title, t.status, t.description),
                    )
                })
                .collect(),
            ContextSource::OrgBrain => bail!("OrgBrain is not configured"),
        };
        self.finish(request, entries)
    }
    fn finish(
        &self,
        request: ContextRequest,
        entries: Vec<(String, String)>,
    ) -> Result<RetrievalResult> {
        let mut result = RetrievalResult {
            source: request.source.clone(),
            references: entries.iter().map(|(r, _)| r.clone()).collect(),
            text: hub_policy::redact(
                &entries
                    .iter()
                    .map(|(r, t)| format!("[{r}]\n{t}"))
                    .collect::<Vec<_>>()
                    .join("\n\n"),
            ),
            truncated: false,
        };
        while estimate_tokens(&serde_json::to_string(&result)?) > request.token_budget {
            result.truncated = true;
            if result.text.is_empty() {
                bail!("retrieval metadata exceeds budget");
            }
            let n = result
                .text
                .char_indices()
                .rev()
                .nth(32)
                .map(|(i, _)| i)
                .unwrap_or(0);
            result.text.truncate(n);
        }
        let payload = serde_json::to_string(&result)?;
        let r = ContextRetrieval {
            id: uuid::Uuid::new_v4().to_string(),
            task_id: self.task,
            source: serde_json::to_string(&request.source)?
                .trim_matches('"')
                .into(),
            query: hub_policy::redact(&request.query),
            token_budget: request.token_budget,
            token_estimate: estimate_tokens(&payload),
            result_ref: result.references.join(","),
        };
        self.store
            .lock()
            .map_err(|_| anyhow!("database lock poisoned"))?
            .commit_retrieval(&r, &payload)?;
        Ok(result)
    }
    pub async fn retrieve_async(&self, request: ContextRequest) -> Result<RetrievalResult> {
        if !matches!(request.source, ContextSource::OrgBrain) {
            return self.retrieve(request);
        }
        let result = async {
            let i = self.inspect()?;
            if request.query.trim().is_empty()
                || request.query.len() > 500
                || request.token_budget < 64
                || request.token_budget > i.capsule.budget.max_single_retrieval_tokens
                || request.token_budget
                    > i.capsule
                        .budget
                        .max_total_tokens
                        .saturating_sub(i.current_estimate)
            {
                bail!("memory retrieval query or budget rejected");
            }
            let memory = self
                .memory
                .as_ref()
                .context("OrgBrain is not configured for this project")?;
            let items = memory
                .search(MemorySearchRequest {
                    query: request.query.clone(),
                    limit: 8,
                })
                .await?;
            self.finish(
                request.clone(),
                items
                    .into_iter()
                    .map(|i| {
                        (
                            i.id,
                            format!("{}\nProvenance: {}", i.text, i.source_refs.join(", ")),
                        )
                    })
                    .collect(),
            )
        }
        .await;
        if let Err(e) = &result {
            self.store
                .lock()
                .map_err(|_| anyhow!("database lock poisoned"))?
                .record_retrieval_denial(&ContextRetrieval {
                    id: uuid::Uuid::new_v4().to_string(),
                    task_id: self.task,
                    source: "org_brain".into(),
                    query: hub_policy::redact(&request.query),
                    token_budget: request.token_budget,
                    token_estimate: 0,
                    result_ref: format!("denied: {e}"),
                })?;
        }
        result
    }
    pub fn tool_definition() -> ToolDefinition {
        ToolDefinition{name:"hub_context".into(),description:"Pull missing task context from an explicit source. All retrievals are logged and consume the worker's budget. Do not request the full parent conversation.".into(),schema:serde_json::json!({"type":"object","properties":{"source":{"enum":["repo","session_store","org_brain","decision_store","task_history"]},"query":{"type":"string"},"path":{"type":["string","null"]},"token_budget":{"type":"integer","minimum":64}},"required":["source","query","path","token_budget"],"additionalProperties":false})}
    }
}
#[async_trait]
impl AgentTool for Broker {
    async fn call(&self, request: ToolCall) -> Result<ToolResult> {
        if request.name != "hub_context" {
            bail!("unknown tool");
        }
        let request: ContextRequest = serde_json::from_value(request.arguments)?;
        match self.retrieve_async(request).await {
            Ok(result) => Ok(ToolResult {
                text: serde_json::to_string(&result)?,
                success: true,
            }),
            Err(e) => Ok(ToolResult {
                text: format!("Context request denied: {e:#}"),
                success: false,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Result<(tempfile::TempDir, Broker)> {
        let d = tempfile::tempdir()?;
        assert!(std::process::Command::new("git")
            .arg("init")
            .arg(d.path())
            .output()?
            .status
            .success());
        let store = Arc::new(Mutex::new(Store::open(&d.path().join("hub.db"))?));
        let p = store.lock().unwrap().register_project(d.path())?;
        let task = Task::new(p.id, "bounded", "implement a small feature");
        store.lock().unwrap().save_task(&task)?;
        let b = Broker {
            memory: None,
            store,
            task: task.id,
            project: p.id,
            root: d.path().into(),
        };
        Ok((d, b))
    }
    #[test]
    fn capsule_has_no_parent_inheritance_and_budget_is_persisted() -> Result<()> {
        let (_d, b) = fixture()?;
        let c = b.build_initial(
            "goal".into(),
            vec!["works".into()],
            vec![],
            vec![],
            ContextBudget {
                initial_tokens: 500,
                max_total_tokens: 1000,
                max_single_retrieval_tokens: 200,
            },
        )?;
        assert!(estimate_tokens(&serde_json::to_string(&c)?) < 500);
        b.store.lock().unwrap().store_decision(
            b.project,
            "decision-1",
            "use rotation",
            "avoid replay",
        )?;
        let r = b.retrieve(ContextRequest {
            source: ContextSource::DecisionStore,
            query: "rotation".into(),
            path: None,
            token_budget: 200,
        })?;
        assert!(r.text.contains("avoid replay"));
        let i = b.inspect()?;
        assert_eq!(i.retrievals.len(), 1);
        assert!(i.retrieved_tokens > 0);
        assert!(!i.parent_conversation_inherited);
        Ok(())
    }
    #[test]
    fn overbudget_and_escape_are_logged_without_consuming_tokens() -> Result<()> {
        let (_d, b) = fixture()?;
        b.build_initial(
            "goal".into(),
            vec![],
            vec![],
            vec![],
            ContextBudget {
                initial_tokens: 500,
                max_total_tokens: 1000,
                max_single_retrieval_tokens: 200,
            },
        )?;
        assert!(b
            .retrieve(ContextRequest {
                source: ContextSource::Repo,
                query: "source".into(),
                path: Some("../outside".into()),
                token_budget: 200
            })
            .is_err());
        assert!(b
            .retrieve(ContextRequest {
                source: ContextSource::DecisionStore,
                query: "decision".into(),
                path: None,
                token_budget: 201
            })
            .is_err());
        let i = b.inspect()?;
        assert_eq!(i.retrieved_tokens, 0);
        assert_eq!(i.retrievals.len(), 2);
        Ok(())
    }
    #[test]
    fn concurrent_retrievals_cannot_overspend() -> Result<()> {
        let (_d, b) = fixture()?;
        b.build_initial(
            "goal".into(),
            vec![],
            vec![],
            vec![],
            ContextBudget {
                initial_tokens: 500,
                max_total_tokens: 550,
                max_single_retrieval_tokens: 400,
            },
        )?;
        let initial = b.inspect()?.initial_tokens;
        let amount = (550 - initial) / 2 + 1;
        let mut store = b.store.lock().unwrap();
        let make = || ContextRetrieval {
            id: uuid::Uuid::new_v4().to_string(),
            task_id: b.task,
            source: "decision_store".into(),
            query: "q".into(),
            token_budget: amount,
            token_estimate: amount,
            result_ref: "test".into(),
        };
        store.commit_retrieval(&make(), "{}")?;
        assert!(store.commit_retrieval(&make(), "{}").is_err());
        Ok(())
    }
}
