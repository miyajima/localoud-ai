use crate::AppState;
use hub_context::Broker;
use hub_core::*;
use hub_runtime::{
    plan::{ExecutionPlan, PlanRuntime},
    workers::Workers,
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

#[tauri::command]
pub fn task_graph(project_id: String, state: tauri::State<AppState>) -> Result<Vec<Task>, String> {
    let id = ProjectId(project_id.parse().map_err(|_| "invalid project ID")?);
    state
        .store
        .lock()
        .map_err(|e| e.to_string())?
        .tasks(id)
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub fn inspect_context(
    thread_id: String,
    state: tauri::State<AppState>,
) -> Result<Option<ContextInspection>, String> {
    let id = crate::parse_thread(thread_id)?;
    let (task, project, root) = {
        let s = state.store.lock().map_err(|e| e.to_string())?;
        let Some(task) = s.thread_task(id).map_err(|e| e.to_string())? else {
            return Ok(None);
        };
        if s.capsule(task).map_err(|e| e.to_string())?.is_none() {
            return Ok(None);
        }
        let thread = s
            .threads()
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|t| t.id == id)
            .ok_or("thread not found")?;
        (
            task,
            thread.project_id,
            s.thread_root(id).map_err(|e| e.to_string())?,
        )
    };
    Broker {
        memory: None,
        store: state.store.clone(),
        task,
        project,
        root,
    }
    .inspect()
    .map(Some)
    .map_err(|e| e.to_string())
}
#[tauri::command]
#[allow(unreachable_code)]
pub async fn run_plan(
    project_id: String,
    plan: ExecutionPlan,
    concurrency: usize,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<Task>, String> {
    let _ = (&project_id, &plan, concurrency, &state);
    return Err("旧Plan実行は別セッションレビューを保証できないため無効です。「ChatGPTで計画」から新しいManifest実行を開始してください。".into());
    let project = ProjectId(project_id.parse().map_err(|_| "invalid project ID")?);
    let (dag, briefs) = plan.compile(project).map_err(|e| e.to_string())?;
    if !(1..=8).contains(&concurrency) {
        return Err("concurrency must be 1–8".into());
    }
    let mut running = state.running_plans.lock().await;
    if running.contains(&project) {
        return Err("このプロジェクトの計画は実行中です。".into());
    }
    let sessions = state.sessions().await?;
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
    let git = hub_worktree::GitWorktrees::new(&root).map_err(|e| e.to_string())?;
    let output = tokio::process::Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["rev-parse", "--verify", "HEAD"])
        .output()
        .await
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err("計画の実行には初期コミットが必要です。".into());
    }
    let base_sha = String::from_utf8(output.stdout)
        .map_err(|e| e.to_string())?
        .trim()
        .to_owned();
    let tasks: Vec<_> = dag.tasks.values().cloned().collect();
    {
        let s = state.store.lock().map_err(|e| e.to_string())?;
        s.save_plan(
            tasks[0].plan_id.ok_or("missing plan ID")?,
            project,
            &serde_json::to_string(&plan).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        for t in &tasks {
            s.save_task(t).map_err(|e| e.to_string())?;
            s.save_dependencies(t).map_err(|e| e.to_string())?;
        }
        s.set_setting(
            &format!("plan_runtime:{project}"),
            &serde_json::to_string(&PlanRuntime {
                plan_id: tasks[0].plan_id.ok_or("missing plan ID")?,
                base_sha: base_sha.clone(),
                concurrency,
                briefs: briefs.clone(),
            })
            .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        s.set_setting(
            &format!("plan:{project}"),
            &serde_json::to_string(&plan).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    }
    let memory = crate::memory::provider(project, &state)?;
    running.insert(project);
    let active = state.running_plans.clone();
    let bus = state.bus.clone();
    let runner = Arc::new(Workers {
        memory,
        store: state.store.clone(),
        sessions,
        provider,
        local: Some(state.local.provider.clone()),
        bus: bus.clone(),
        worktrees: git,
        base_sha,
        briefs,
        outcomes: Mutex::new(HashMap::new()),
    });
    tauri::async_runtime::spawn(async move {
        let result = hub_scheduler::execute(dag, runner, concurrency).await;
        active.lock().await.remove(&project);
        let text = match result {
            Ok(dag) => format!(
                "{} tasks finished; {} require attention",
                dag.tasks.len(),
                dag.tasks
                    .values()
                    .filter(|t| t.status != TaskStatus::Completed)
                    .count()
            ),
            Err(e) => format!("Plan failed: {e:#}"),
        };
        let _ = bus.publish(protocol_types::AgentEvent {
            details: None,
            thread_id: None,
            turn_id: None,
            item_id: None,
            kind: "plan_finished".into(),
            text,
        });
    });
    Ok(tasks)
}

#[tauri::command]
#[allow(unreachable_code)]
pub async fn resume_plan(
    project_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<Task>, String> {
    let _ = (&project_id, &state);
    return Err("旧Plan実行は再開できません。保存済み成果物を確認し、新しいManifestで未完了範囲を開始してください。".into());
    let project = ProjectId(project_id.parse().map_err(|_| "invalid project ID")?);
    let mut running = state.running_plans.lock().await;
    if running.contains(&project) {
        return Err("このプロジェクトの計画は実行中です。".into());
    }
    let (runtime, tasks, root, outcomes) = {
        let s = state.store.lock().map_err(|e| e.to_string())?;
        let runtime: PlanRuntime = serde_json::from_str(
            &s.setting(&format!("plan_runtime:{project}"))
                .map_err(|e| e.to_string())?
                .ok_or("再開可能な計画がありません。")?,
        )
        .map_err(|e| e.to_string())?;
        let tasks = s.tasks(project).map_err(|e| e.to_string())?;
        let root = s
            .projects()
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|p| p.id == project)
            .ok_or("project missing")?
            .root;
        let mut outcomes = HashMap::new();
        for id in runtime.briefs.keys() {
            if let Some(raw) = s
                .setting(&format!("outcome:{id}"))
                .map_err(|e| e.to_string())?
            {
                outcomes.insert(
                    *id,
                    serde_json::from_str::<hub_runtime::workers::WorkerOutcome>(&raw)
                        .map_err(|e| e.to_string())?,
                );
            }
        }
        (runtime, tasks, root, outcomes)
    };
    let git = hub_worktree::GitWorktrees::new(&root).map_err(|e| e.to_string())?;
    for outcome in outcomes.values() {
        if git
            .diff(&outcome.worktree)
            .await
            .map_err(|e| e.to_string())?
            != outcome.diff
        {
            return Err(
                "完了後の差分が変更されています。レビューしてから再開してください。".into(),
            );
        }
    }
    let dag = runtime
        .recover(tasks, &outcomes)
        .map_err(|e| e.to_string())?;
    let tasks = dag.tasks.values().cloned().collect();
    let sessions = state.sessions().await?;
    let provider = state
        .connection
        .lock()
        .await
        .as_ref()
        .ok_or("provider unavailable")?
        .provider
        .clone();
    let memory = crate::memory::provider(project, &state)?;
    let bus = state.bus.clone();
    let runner = Arc::new(Workers {
        memory,
        store: state.store.clone(),
        sessions,
        provider,
        local: Some(state.local.provider.clone()),
        bus: bus.clone(),
        worktrees: git,
        base_sha: runtime.base_sha,
        briefs: runtime.briefs,
        outcomes: Mutex::new(outcomes),
    });
    running.insert(project);
    let active = state.running_plans.clone();
    tauri::async_runtime::spawn(async move {
        let result = hub_scheduler::execute(dag, runner, runtime.concurrency).await;
        active.lock().await.remove(&project);
        let text = match result {
            Ok(dag) => format!(
                "Plan reconciled: {} completed / {} total",
                dag.tasks
                    .values()
                    .filter(|t| t.status == TaskStatus::Completed)
                    .count(),
                dag.tasks.len()
            ),
            Err(e) => format!("Plan recovery failed: {e:#}"),
        };
        let _ = bus.publish(protocol_types::AgentEvent {
            details: None,
            thread_id: None,
            turn_id: None,
            item_id: None,
            kind: "plan_finished".into(),
            text,
        });
    });
    Ok(tasks)
}
