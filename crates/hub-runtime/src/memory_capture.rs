use crate::workers::WorkerOutcome;
use anyhow::{anyhow, bail, Result};
use hub_core::*;
use hub_db::Store;
use hub_memory::DurableMemoryProvider;
use protocol_types::local::LocalModelProvider;
use std::sync::Mutex;

pub async fn capture(
    store: &Mutex<Store>,
    local: &dyn LocalModelProvider,
    memory: Option<&dyn DurableMemoryProvider>,
    task: &Task,
    outcome: &WorkerOutcome,
) -> Result<Vec<DurableMemoryItem>> {
    let scope = hub_policy::scope_digest(&task.description, &outcome.diff);
    let approved = {
        let s = store
            .lock()
            .map_err(|_| anyhow!("database lock poisoned"))?;
        s.setting(&format!("review_scope:{}", task.id))?.as_deref() == Some(&scope)
            && s.setting(&format!("review:{}", task.id))?
                .and_then(|v| serde_json::from_str::<serde_json::Value>(&v).ok())
                .is_some_and(|v| v["verdict"] == "approve")
    };
    let candidate_scope = format!("capture-v2:{scope}:{approved}");
    let cached = {
        let s = store
            .lock()
            .map_err(|_| anyhow!("database lock poisoned"))?;
        if s.setting(&format!("memory_candidate_scope:{}", task.id))?
            .as_deref()
            == Some(&candidate_scope)
        {
            s.setting(&format!("memory_candidates:{}", task.id))?
                .map(|v| serde_json::from_str::<Vec<DurableMemoryItem>>(&v))
                .transpose()?
        } else {
            None
        }
    };
    let items = if let Some(items) = cached {
        items
    } else {
        let task_ref = format!("task-{}", task.id);
        let diff_ref = format!("diff-{}", &scope[..16]);
        let review_ref = format!("review-{}", task.id);
        let mut refs = vec![task_ref.clone(), diff_ref.clone()];
        let summary=format!("Worker finished implementing {}. Review approval for this exact goal and diff: {approved}. No unresolved failure evidence is supplied. Never infer a test failure from a count of historical command attempts.",task.title);
        let files = outcome
            .diff
            .lines()
            .filter_map(|l| l.strip_prefix("+++ b/"))
            .collect::<Vec<_>>();
        let added = outcome
            .diff
            .lines()
            .filter(|l| l.starts_with('+') && !l.starts_with("+++"))
            .count();
        let removed = outcome
            .diff
            .lines()
            .filter(|l| l.starts_with('-') && !l.starts_with("---"))
            .count();
        let mut evidence=vec![format!("{task_ref}: Task requirements (not test evidence): {}",task.description),format!("{diff_ref}: Observed completed diff: files {}; {added} added lines, {removed} removed lines. These numbers are line counts, not process exit codes.",files.join(", "))];
        if approved {
            refs.push(review_ref.clone());
            evidence.push(format!("{review_ref}: Final review approved the exact held goal and diff. Do not invent unresolved verification work or current failures."));
        }
        let generated = local.extract_memory(&summary, &evidence).await?;
        store
            .lock()
            .map_err(|_| anyhow!("database lock poisoned"))?
            .record_usage(&ModelUsageRecord {
                id: uuid::Uuid::new_v4().to_string(),
                provider: "spark".into(),
                model: "Spark-X2.5-4B-MLX-8bit".into(),
                task_id: Some(task.id),
                turn_id: None,
                prompt_tokens: Some(generated.usage.prompt_tokens),
                cached_tokens: None,
                completion_tokens: Some(generated.usage.completion_tokens),
                estimated_cost: None,
                latency_ms: Some(generated.usage.latency_ms),
            })?;
        if generated.output.candidates.iter().any(|c| {
            c.durable && matches!(c.kind, MemoryKind::Failure | MemoryKind::UnresolvedIssue)
        }) {
            bail!("extractor claimed a failure/issue without supporting failure evidence; nothing persisted");
        }
        let items = hub_memory::durable_items(task.project_id, task.id, generated.output, &refs)?;
        let s = store
            .lock()
            .map_err(|_| anyhow!("database lock poisoned"))?;
        s.set_setting(
            &format!("memory_candidates:{}", task.id),
            &serde_json::to_string(&items)?,
        )?;
        s.set_setting(
            &format!("memory_candidate_scope:{}", task.id),
            &candidate_scope,
        )?;
        s.set_setting(&format!("memory_saved:{}", task.id), "false")?;
        items
    };
    let already_saved = store
        .lock()
        .map_err(|_| anyhow!("database lock poisoned"))?
        .setting(&format!("memory_saved:{}", task.id))?
        .as_deref()
        == Some("true");
    if let Some(memory) = memory {
        if approved && memory.persistence_enabled() && !items.is_empty() && !already_saved {
            memory.store(items.clone()).await?;
            store
                .lock()
                .map_err(|_| anyhow!("database lock poisoned"))?
                .set_setting(&format!("memory_saved:{}", task.id), "true")?;
        }
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use protocol_types::local::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Local;
    #[async_trait]
    impl LocalModelProvider for Local {
        async fn extract_memory(
            &self,
            _: &str,
            evidence: &[String],
        ) -> Result<Generation<MemoryExtraction>> {
            Ok(Generation {
                output: MemoryExtraction {
                    candidates: vec![MemoryCandidate {
                        kind: MemoryKind::Constraint,
                        durable: true,
                        statement: "Keep public names stable".into(),
                        reason: "Existing callers depend on them".into(),
                        source_refs: vec![evidence[0].split(':').next().unwrap().into()],
                    }],
                },
                usage: LocalUsage {
                    prompt_tokens: 10,
                    completion_tokens: 10,
                    latency_ms: 1,
                },
            })
        }
        async fn available(&self) -> Result<()> {
            Ok(())
        }
        async fn classify_task(&self, _: &RoutingInput) -> Result<Generation<RoutingDecision>> {
            unreachable!()
        }
        async fn implement(&self, _: &str, _: &[FileContext]) -> Result<Generation<EditProposal>> {
            unreachable!()
        }
        async fn summarize_progress(&self, _: &[String]) -> Result<Generation<ProgressSummary>> {
            unreachable!()
        }
        async fn review_diff(&self, _: &str, _: &str) -> Result<Generation<ReviewResult>> {
            unreachable!()
        }
        async fn draft_context(&self, _: &str, _: &[String]) -> Result<Generation<CapsuleDraft>> {
            unreachable!()
        }
        async fn generate_retrieval_query(&self, _: &str) -> Result<Generation<RetrievalQuery>> {
            unreachable!()
        }
    }
    struct Memory(AtomicUsize);
    #[async_trait]
    impl DurableMemoryProvider for Memory {
        async fn search(&self, _: MemorySearchRequest) -> Result<Vec<MemoryItem>> {
            Ok(vec![])
        }
        async fn store(&self, _: Vec<DurableMemoryItem>) -> Result<()> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }
    #[tokio::test]
    async fn persistence_requires_approval_of_exact_goal_and_diff() -> Result<()> {
        let d = tempfile::tempdir()?;
        std::process::Command::new("git")
            .arg("init")
            .arg(d.path())
            .output()?;
        let store = Mutex::new(Store::open(&d.path().join("hub.db"))?);
        let p = store.lock().unwrap().register_project(d.path())?;
        let task = Task::new(p.id, "fixture", "Keep public names stable");
        store.lock().unwrap().save_task(&task)?;
        let mut outcome = WorkerOutcome {
            task_id: task.id,
            thread_id: HubThreadId::default(),
            worktree: Worktree {
                id: WorktreeId::default(),
                task_id: task.id,
                path: d.path().into(),
                base_sha: "fixture".into(),
            },
            diff: "+++ b/a.py\n+pass\n".into(),
            latency_ms: 1,
            command_failures: 3,
        };
        let memory = Memory(AtomicUsize::new(0));
        capture(&store, &Local, Some(&memory), &task, &outcome).await?;
        assert_eq!(memory.0.load(Ordering::SeqCst), 0);
        {
            let s = store.lock().unwrap();
            s.set_setting(&format!("review:{}", task.id), "{\"verdict\":\"approve\"}")?;
            s.set_setting(&format!("review_scope:{}", task.id), "wrong-diff")?;
        }
        capture(&store, &Local, Some(&memory), &task, &outcome).await?;
        assert_eq!(memory.0.load(Ordering::SeqCst), 0);
        store.lock().unwrap().set_setting(
            &format!("review_scope:{}", task.id),
            &hub_policy::scope_digest(&task.description, &outcome.diff),
        )?;
        capture(&store, &Local, Some(&memory), &task, &outcome).await?;
        assert_eq!(memory.0.load(Ordering::SeqCst), 1);
        capture(&store, &Local, Some(&memory), &task, &outcome).await?;
        assert_eq!(memory.0.load(Ordering::SeqCst), 1);
        outcome.diff.push_str("+changed\n");
        capture(&store, &Local, Some(&memory), &task, &outcome).await?;
        assert_eq!(memory.0.load(Ordering::SeqCst), 1);
        Ok(())
    }
}
