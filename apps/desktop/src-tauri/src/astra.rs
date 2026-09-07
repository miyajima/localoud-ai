use crate::AppState;
use hub_core::*;
use hub_planner::{
    Astra, FailureDiagnosisRequest, FinalReview, FinalReviewRequest, PlannerReviewer,
    PlanningRequest, RecoveryPlan,
};
use hub_runtime::plan::ExecutionPlan;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub mode: AstraAccessMode,
    pub model: String,
}
#[tauri::command]
pub fn astra_settings(state: tauri::State<AppState>) -> Result<Settings, String> {
    settings(&state)
}
fn settings(state: &AppState) -> Result<Settings, String> {
    let s = state.store.lock().map_err(|e| e.to_string())?;
    match s.setting("astra_settings").map_err(|e| e.to_string())? {
        Some(v) => serde_json::from_str(&v).map_err(|e| e.to_string()),
        None => Ok(Settings {
            mode: AstraAccessMode::Disabled,
            model: "gpt-6-astra".into(),
        }),
    }
}
#[tauri::command]
pub fn set_astra_settings(config: Settings, state: tauri::State<AppState>) -> Result<(), String> {
    if !matches!(
        config.mode,
        AstraAccessMode::Disabled | AstraAccessMode::CodexIntegrated
    ) {
        return Err(
            "この版は Codex 認証の利用と無効化に対応しています。直接 API の課金設定は別です。"
                .into(),
        );
    }
    if config.model.trim().is_empty() {
        return Err("モデル ID が必要です。".into());
    }
    state
        .store
        .lock()
        .map_err(|e| e.to_string())?
        .set_setting(
            "astra_settings",
            &serde_json::to_string(&config).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())
}
async fn adapter(project: ProjectId, state: &AppState) -> Result<Astra, String> {
    let config = settings(state)?;
    if matches!(config.mode, AstraAccessMode::Disabled) {
        return Err(
            "Astra は無効です。接続設定で有効にするか、Plan の JSON を編集して実行できます。"
                .into(),
        );
    }
    state.sessions().await?;
    let provider = state
        .connection
        .lock()
        .await
        .as_ref()
        .ok_or("provider unavailable")?
        .provider
        .clone();
    let root = state
        .store
        .lock()
        .map_err(|e| e.to_string())?
        .projects()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|p| p.id == project)
        .ok_or("project not found")?
        .root;
    Ok(Astra {
        mode: config.mode,
        model: config.model,
        provider,
        root,
    })
}
#[tauri::command]
pub async fn create_astra_plan(
    project_id: String,
    goal: String,
    state: tauri::State<'_, AppState>,
) -> Result<ExecutionPlan, String> {
    let id = ProjectId(project_id.parse().map_err(|_| "invalid project ID")?);
    let result = adapter(id, &state)
        .await?
        .create_plan(PlanningRequest {
            goal,
            constraints: vec!["Never merge into the parent branch".into()],
            evidence: vec![],
        })
        .await
        .map_err(|e| format!("{e:#}"))?;
    state
        .store
        .lock()
        .map_err(|e| e.to_string())?
        .set_setting(
            &format!("plan-draft:{id}"),
            &serde_json::to_string(&result).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    Ok(result)
}
fn task_for(thread: HubThreadId, state: &AppState) -> Result<Task, String> {
    let s = state.store.lock().map_err(|e| e.to_string())?;
    let mapping = s
        .threads()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|t| t.id == thread)
        .ok_or("thread not found")?;
    let task = s
        .thread_task(thread)
        .map_err(|e| e.to_string())?
        .ok_or("Plan worker を選択してください。")?;
    s.tasks(mapping.project_id)
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|t| t.id == task)
        .ok_or("task not found".into())
}
#[tauri::command]
pub async fn final_review(
    thread_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<FinalReview, String> {
    let id = crate::parse_thread(thread_id)?;
    let mut task = task_for(id, &state)?;
    if task.status == TaskStatus::Running
        || state
            .store
            .lock()
            .map_err(|e| e.to_string())?
            .threads()
            .map_err(|e| e.to_string())?
            .iter()
            .any(|t| t.id == id && matches!(t.status.as_str(), "running" | "inProgress"))
    {
        return Err("worker の停止後にレビューしてください。".into());
    }
    let (worktree, capsule, mut evidence) = {
        let s = state.store.lock().map_err(|e| e.to_string())?;
        let w = s
            .worktree_for_task(task.id)
            .map_err(|e| e.to_string())?
            .ok_or("worktree missing")?;
        let c = s
            .capsule(task.id)
            .map_err(|e| e.to_string())?
            .ok_or("capsule missing")?
            .0;
        let mut evidence = s
            .review_evidence(id)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|e| e.chars().take(3000).collect())
            .collect::<Vec<String>>();
        evidence.push(format!(
            "Hub's persisted context retrieval ledger: {}",
            serde_json::to_string(&s.retrievals(task.id).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?
        ));
        evidence.extend(
            s.task_control_evidence(task.id)
                .map_err(|e| e.to_string())?,
        );
        (w, c, evidence)
    };
    let root = state
        .store
        .lock()
        .map_err(|e| e.to_string())?
        .projects()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|p| p.id == task.project_id)
        .ok_or("project not found")?
        .root;
    let diff = hub_worktree::GitWorktrees::new(&root)
        .map_err(|e| e.to_string())?
        .diff(&worktree)
        .await
        .map_err(|e| e.to_string())?;
    evidence.push(
        hub_worktree::GitWorktrees::new(&root)
            .map_err(|e| e.to_string())?
            .review_provenance(&worktree)
            .await
            .map_err(|e| e.to_string())?,
    );
    let scope = hub_policy::scope_digest(&task.description, &diff);
    let review = adapter(task.project_id, &state)
        .await?
        .review_result(FinalReviewRequest {
            task_id: task.id,
            goal: task.description.clone(),
            acceptance_criteria: capsule.acceptance_criteria,
            diff,
            evidence,
        })
        .await
        .map_err(|e| format!("{e:#}"))?;
    state
        .store
        .lock()
        .map_err(|e| e.to_string())?
        .set_setting(
            &format!("review:{}", task.id),
            &serde_json::to_string(&review).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    state
        .store
        .lock()
        .map_err(|e| e.to_string())?
        .set_setting(&format!("review_scope:{}", task.id), &scope)
        .map_err(|e| e.to_string())?;
    task.status = match review.verdict {
        hub_planner::Verdict::Approve => TaskStatus::Completed,
        _ => TaskStatus::Reviewing,
    };
    state
        .store
        .lock()
        .map_err(|e| e.to_string())?
        .save_task(&task)
        .map_err(|e| e.to_string())?;
    if matches!(review.verdict, hub_planner::Verdict::Approve) {
        if let Err(e) = crate::memory::extract_memory(id.to_string(), state.clone()).await {
            let _ = state.bus.publish(protocol_types::AgentEvent {
                details: None,
                thread_id: None,
                turn_id: None,
                item_id: None,
                kind: "memory_unavailable".into(),
                text: format!("Final review approved; memory capture remains unconfirmed: {e}"),
            });
        }
    }
    Ok(review)
}
#[tauri::command]
pub async fn diagnose_task(
    thread_id: String,
    question: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<RecoveryPlan, String> {
    let id = crate::parse_thread(thread_id)?;
    let task = task_for(id, &state)?;
    if task.attempts < 2 && question.as_ref().is_none_or(|q| q.trim().is_empty()) {
        return Err("診断には2回以上の試行か、設計上の質問が必要です。".into());
    }
    let failures = state
        .store
        .lock()
        .map_err(|e| e.to_string())?
        .event_history(id, 0)
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter_map(|(_, b)| serde_json::from_str::<protocol_types::AgentEvent>(&b).ok())
        .filter(|e| e.kind == "error" || e.kind == "turn_completed" || e.kind == "item_completed")
        .rev()
        .take(12)
        .map(|e| e.text.chars().take(1800).collect())
        .collect();
    let r = adapter(task.project_id, &state)
        .await?
        .diagnose_failure(FailureDiagnosisRequest {
            task_id: task.id,
            goal: task.description,
            attempts: task.attempts,
            failures,
            architecture_question: question,
        })
        .await
        .map_err(|e| format!("{e:#}"))?;
    state
        .store
        .lock()
        .map_err(|e| e.to_string())?
        .set_setting(
            &format!("recovery:{}", task.id),
            &serde_json::to_string(&r).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    state
        .store
        .lock()
        .map_err(|e| e.to_string())?
        .set_setting(&format!("recovery_consumed:{}", task.id), "false")
        .map_err(|e| e.to_string())?;
    Ok(r)
}
#[tauri::command]
pub async fn apply_rework(
    thread_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let id = crate::parse_thread(thread_id)?;
    let mut task = task_for(id, &state)?;
    if task.attempts >= task.max_attempts {
        return Err(
            "試行上限です。Astra 診断で再計画するか、設計上の判断を確認してください。".into(),
        );
    }
    let review: FinalReview = serde_json::from_str(
        &state
            .store
            .lock()
            .map_err(|e| e.to_string())?
            .setting(&format!("review:{}", task.id))
            .map_err(|e| e.to_string())?
            .ok_or("最終レビューがありません。")?,
    )
    .map_err(|e| e.to_string())?;
    if !matches!(review.verdict, hub_planner::Verdict::Rework) {
        return Err("修正依頼のレビューがありません。".into());
    }
    let instruction = review.rework_instruction.ok_or("修正指示がありません。")?;
    let sessions = state.sessions().await?;
    sessions.resume(id).await.map_err(|e| e.to_string())?;
    let provider = state
        .connection
        .lock()
        .await
        .as_ref()
        .ok_or("provider unavailable")?
        .provider
        .clone();
    use protocol_types::CodingAgentProvider;
    let mut events = provider.events();
    let turn=sessions.start(id,format!("Address this final review in the same worktree. Re-run acceptance checks. Do not delegate or merge.\n{instruction}")).await.map_err(|e|e.to_string())?;
    task.attempts += 1;
    task.status = TaskStatus::Running;
    state
        .store
        .lock()
        .map_err(|e| e.to_string())?
        .save_task(&task)
        .map_err(|e| e.to_string())?;
    let store = state.store.clone();
    let bus = state.bus.clone();
    tauri::async_runtime::spawn(async move {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(600);
        loop {
            match tokio::time::timeout_at(deadline, events.recv()).await {
                Ok(Ok(e))
                    if e.turn_id.as_deref() == Some(&turn.id) && e.kind == "turn_completed" =>
                {
                    task.status = if e.text == "completed" {
                        TaskStatus::Reviewing
                    } else {
                        TaskStatus::Failed
                    };
                    break;
                }
                Ok(Ok(_)) => {}
                _ => {
                    let _ = sessions.interrupt(id).await;
                    task.status = TaskStatus::Failed;
                    break;
                }
            }
        }
        if let Ok(s) = store.lock() {
            let _ = s.save_task(&task);
        }
        let _ = bus.publish(protocol_types::AgentEvent {
            details: None,
            thread_id: None,
            turn_id: None,
            item_id: None,
            kind: "task_state".into(),
            text: serde_json::to_string(&task).unwrap_or_default(),
        });
    });
    Ok(())
}

#[tauri::command]
pub fn worker_insights(
    thread_id: String,
    state: tauri::State<AppState>,
) -> Result<serde_json::Value, String> {
    let id = crate::parse_thread(thread_id)?;
    let s = state.store.lock().map_err(|e| e.to_string())?;
    let Some(task) = s.thread_task(id).map_err(|e| e.to_string())? else {
        return Ok(serde_json::json!({}));
    };
    let read = |prefix: &str| -> Result<serde_json::Value, String> {
        Ok(s.setting(&format!("{prefix}:{task}"))
            .map_err(|e| e.to_string())?
            .map(|v| serde_json::from_str(&v))
            .transpose()
            .map_err(|e| e.to_string())?
            .unwrap_or(serde_json::Value::Null))
    };
    Ok(serde_json::json!({"review":read("review")?,"recovery":read("recovery")?}))
}
#[tauri::command]
pub async fn apply_recovery(
    thread_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let id = crate::parse_thread(thread_id.clone())?;
    let mut task = task_for(id, &state)?;
    if task.status == TaskStatus::Running
        || state
            .store
            .lock()
            .map_err(|e| e.to_string())?
            .threads()
            .map_err(|e| e.to_string())?
            .iter()
            .any(|t| t.id == id && matches!(t.status.as_str(), "running" | "inProgress"))
    {
        return Err("worker は実行中です。".into());
    }
    {
        let s = state.store.lock().map_err(|e| e.to_string())?;
        if s.setting(&format!("recovery_consumed:{}", task.id))
            .map_err(|e| e.to_string())?
            .as_deref()
            == Some("true")
        {
            return Err("この診断は適用済みです。新しい証跡で診断してください。".into());
        }
        let r: RecoveryPlan = serde_json::from_str(
            &s.setting(&format!("recovery:{}", task.id))
                .map_err(|e| e.to_string())?
                .ok_or("診断がありません。")?,
        )
        .map_err(|e| e.to_string())?;
        if !matches!(r.action, hub_planner::RecoveryAction::RetryWorker) {
            return Err("診断は既存 worker の再試行を指定していません。".into());
        }
        task.max_attempts = task
            .attempts
            .checked_add(1)
            .ok_or("attempt counter exhausted")?;
        s.save_task(&task).map_err(|e| e.to_string())?;
        s.set_setting(
            &format!("review:{}", task.id),
            &serde_json::to_string(&FinalReview {
                verdict: hub_planner::Verdict::Rework,
                findings: vec![r.reason],
                rework_instruction: Some(r.instruction),
            })
            .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        s.set_setting(&format!("recovery_consumed:{}", task.id), "true")
            .map_err(|e| e.to_string())?;
    }
    apply_rework(thread_id, state).await
}
