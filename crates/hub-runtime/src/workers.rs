use crate::Sessions;
use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use hub_context::Broker;
use hub_core::*;
use hub_db::Store;
use hub_events::EventBus;
use hub_scheduler::TaskRunner;
use hub_worktree::GitWorktrees;
use protocol_types::{local::LocalModelProvider, AgentEvent, CodingAgentProvider};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerBrief {
    pub acceptance_criteria: Vec<String>,
    pub constraints: Vec<String>,
    pub context_items: Vec<ContextItem>,
    pub budget: ContextBudget,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerOutcome {
    pub task_id: TaskId,
    pub thread_id: HubThreadId,
    pub worktree: Worktree,
    pub diff: String,
    pub latency_ms: u64,
    pub command_failures: u32,
}
pub struct Workers {
    pub store: Arc<Mutex<Store>>,
    pub sessions: Arc<Sessions>,
    pub provider: Arc<dyn CodingAgentProvider>,
    pub local: Option<Arc<dyn LocalModelProvider>>,
    pub memory: Option<Arc<dyn hub_memory::DurableMemoryProvider>>,
    pub bus: EventBus,
    pub worktrees: GitWorktrees,
    pub base_sha: String,
    pub briefs: HashMap<TaskId, WorkerBrief>,
    pub outcomes: Mutex<HashMap<TaskId, WorkerOutcome>>,
}
impl Workers {
    async fn run_inner(&self, mut task: Task) -> Result<()> {
        let started = Instant::now();
        let brief = self.briefs.get(&task.id).context("missing worker brief")?;
        let existing = {
            let store = self
                .store
                .lock()
                .map_err(|_| anyhow!("database lock poisoned"))?;
            let w = store.worktree_for_task(task.id)?;
            let mut mappings = Vec::new();
            for mapping in store.threads()? {
                if store.thread_task(mapping.id)? == Some(task.id) {
                    mappings.push(mapping);
                }
            }
            if mappings.len() > 1 {
                bail!("multiple worker threads require manual reconciliation");
            }
            w.map(|w| (w, mappings.into_iter().next()))
        };
        let mut events = self.provider.events();
        let (worktree, mapping, capsule, resumed) = if let Some((worktree, mapping)) = existing {
            self.worktrees.diff(&worktree).await?; // Validate ownership and retained base before resuming.
            let mapping = mapping.context("worktree exists without a worker thread; inspect interrupted preparation before continuing")?;
            let capsule = self
                .store
                .lock()
                .map_err(|_| anyhow!("database lock poisoned"))?
                .capsule(task.id)?
                .context("resumed worker capsule missing")?
                .0;
            let snapshot = self.sessions.resume(mapping.id).await?;
            (worktree, mapping, capsule, Some(snapshot))
        } else {
            let worktree = self.worktrees.create(task.id, &self.base_sha).await?;
            {
                let store = self
                    .store
                    .lock()
                    .map_err(|_| anyhow!("database lock poisoned"))?;
                store.save_worktree(&worktree)?;
                task.worktree_id = Some(worktree.id);
                store.save_task(&task)?;
            }
            self.bus.publish(AgentEvent {
                details: None,
                thread_id: None,
                turn_id: None,
                item_id: None,
                kind: "worktree_created".into(),
                text: serde_json::json!({"task_id":task.id,"worktree":worktree}).to_string(),
            })?;
            for dependency in &task.dependencies {
                let patch = self
                    .outcomes
                    .lock()
                    .map_err(|_| anyhow!("outcome lock poisoned"))?
                    .get(dependency)
                    .context("dependency has no completed result")?
                    .diff
                    .clone();
                self.worktrees
                    .apply_dependency_patch(&worktree, &patch)
                    .await?;
                self.bus.publish(AgentEvent{details:None,thread_id:None,turn_id:None,item_id:None,kind:"dependency_applied".into(),text:serde_json::json!({"task_id":task.id,"dependency_id":dependency,"patch_digest":hub_policy::scope_digest("dependency",&patch)}).to_string()})?;
            }
            let broker = Arc::new(Broker {
                memory: self.memory.clone(),
                store: self.store.clone(),
                task: task.id,
                project: task.project_id,
                root: worktree.path.clone(),
            });
            let mut selected = brief.context_items.clone();
            if let Some(local) = &self.local {
                let refs: Vec<_> = selected.iter().map(|i| i.reference.clone()).collect();
                match local.draft_context(&task.description, &refs).await {
                    Ok(draft) => {
                        if draft.output.selected_refs.iter().any(|r| !refs.contains(r)) {
                            bail!("context drafter selected an unknown reference");
                        }
                        selected.retain(|i| draft.output.selected_refs.contains(&i.reference));
                        self.store
                            .lock()
                            .map_err(|_| anyhow!("database lock poisoned"))?
                            .record_usage(&ModelUsageRecord {
                                id: uuid::Uuid::new_v4().to_string(),
                                provider: "spark".into(),
                                model: "Spark-X2.5-4B-MLX-8bit".into(),
                                task_id: Some(task.id),
                                turn_id: None,
                                prompt_tokens: Some(draft.usage.prompt_tokens),
                                cached_tokens: None,
                                completion_tokens: Some(draft.usage.completion_tokens),
                                estimated_cost: None,
                                latency_ms: Some(draft.usage.latency_ms),
                            })?;
                    }
                    Err(e) => self.bus.publish(AgentEvent {
                        details: None,
                        thread_id: None,
                        turn_id: None,
                        item_id: None,
                        kind: "context_drafter_unavailable".into(),
                        text: format!("Task {} uses deterministic selected context: {e}", task.id),
                    })?,
                }
            }
            let capsule = broker.build_initial(
                task.description.clone(),
                brief.acceptance_criteria.clone(),
                brief.constraints.clone(),
                selected,
                brief.budget.clone(),
            )?;
            task.context_capsule_id = Some(capsule.id);
            task.assigned_executor = Some(ExecutorKind::Codex);
            self.store
                .lock()
                .map_err(|_| anyhow!("database lock poisoned"))?
                .save_task(&task)?;
            let mapping = self
                .sessions
                .create_worker(
                    &task,
                    &worktree,
                    broker.clone(),
                    vec![Broker::tool_definition()],
                )
                .await?;
            self.bus.publish(AgentEvent {
                details: None,
                thread_id: None,
                turn_id: None,
                item_id: None,
                kind: "worker_assigned".into(),
                text: serde_json::to_string(&mapping)?,
            })?;
            (worktree, mapping, capsule, None)
        };
        let prompt=format!("You are an independent worker in an isolated Git worktree. This capsule is the selected task context; the parent conversation has NOT been copied. Use hub_context to pull missing decisions or source excerpts. Respect its budget. Work only in this worktree. Do not delegate or merge into another branch. Implement the task and verify the acceptance criteria using your sandboxed tools. Do not claim tests passed unless you ran them.\n\n{}",serde_json::to_string_pretty(&capsule)?);
        let turn = if let Some(snapshot) = resumed {
            if let Some(turn) = snapshot.active_turn {
                turn
            } else {
                self.sessions.start(mapping.id, format!("Continue the existing task after Hub restart. Preserve the existing work and verify all acceptance criteria. Do not recreate or merge worktrees.\n\n{prompt}")).await?
            }
        } else {
            self.sessions.start(mapping.id, prompt).await?
        };
        let deadline = tokio::time::Instant::now() + Duration::from_secs(600);
        let mut command_failures = 0;
        let status = loop {
            match tokio::time::timeout_at(deadline, events.recv()).await {
                Ok(Ok(e)) => {
                    if e.thread_id.as_deref() != Some(&mapping.provider_thread_id) {
                        if e.kind == "disconnected" {
                            bail!("provider disconnected; resume this worker before retrying");
                        }
                        continue;
                    }
                    if e.kind == "item_completed"
                        && matches!(e.details,Some(protocol_types::EventDetails::Command{exit_code:Some(code),..}) if code!=0)
                    {
                        command_failures += 1;
                    }
                    if e.kind == "turn_completed" && e.turn_id.as_deref() == Some(&turn.id) {
                        break e.text;
                    }
                }
                Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => {
                    let _ = self.sessions.interrupt(mapping.id).await;
                    bail!("worker event stream lagged; review journal before continuing");
                }
                _ => {
                    let _ = self.sessions.interrupt(mapping.id).await;
                    bail!("worker timed out or event stream closed");
                }
            }
        };
        if status != "completed" {
            bail!("worker turn ended with status {status}");
        }
        let diff = self.worktrees.diff(&worktree).await?;
        if hub_policy::redact(&diff) != diff {
            bail!("worker diff contains secret-like content; inspect locally");
        }
        let outcome = WorkerOutcome {
            task_id: task.id,
            thread_id: mapping.id,
            worktree,
            diff,
            latency_ms: started.elapsed().as_millis() as u64,
            command_failures,
        };
        self.store
            .lock()
            .map_err(|_| anyhow!("database lock poisoned"))?
            .set_setting(
                &format!("outcome:{}", task.id),
                &serde_json::to_string(&outcome)?,
            )?;
        self.outcomes
            .lock()
            .map_err(|_| anyhow!("outcome lock poisoned"))?
            .insert(task.id, outcome);
        if let Some(local) = &self.local {
            let saved = self
                .outcomes
                .lock()
                .map_err(|_| anyhow!("outcomes lock poisoned"))?
                .get(&task.id)
                .context("missing outcome")?
                .clone();
            let capture = crate::memory_capture::capture(
                &self.store,
                local.as_ref(),
                self.memory.as_deref(),
                &task,
                &saved,
            )
            .await;
            if let Err(e) = capture {
                self.bus.publish(AgentEvent{details:None,thread_id:Some(mapping.provider_thread_id),turn_id:Some(turn.id),item_id:None,kind:"memory_capture_failed".into(),text:format!("Task {}: {e:#}; code result remains available, memory was not confirmed saved",task.id)})?;
            }
        }
        Ok(())
    }
}
#[async_trait]
impl TaskRunner for Workers {
    async fn run(&self, task: Task) -> Result<()> {
        let id = task.id;
        let result = self.run_inner(task).await;
        if let Err(e) = &result {
            self.bus.publish(AgentEvent {
                details: None,
                thread_id: None,
                turn_id: None,
                item_id: None,
                kind: "worker_failed".into(),
                text: format!("Task {id}: {e:#}"),
            })?;
        }
        result
    }
    async fn changed(&self, task: &Task) -> Result<()> {
        // Preserve worktree/capsule associations written during execution.
        let store = self
            .store
            .lock()
            .map_err(|_| anyhow!("database lock poisoned"))?;
        let mut persisted = store
            .tasks(task.project_id)?
            .into_iter()
            .find(|t| t.id == task.id)
            .unwrap_or_else(|| task.clone());
        persisted.status = task.status;
        persisted.attempts = task.attempts;
        store.save_task(&persisted)?;
        drop(store);
        self.bus.publish(AgentEvent {
            details: None,
            thread_id: None,
            turn_id: None,
            item_id: None,
            kind: "task_state".into(),
            text: serde_json::to_string(&persisted)?,
        })
    }
}
