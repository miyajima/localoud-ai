use crate::AppState;
use hub_core::*;
use hub_memory::{DurableMemoryProvider, OrgBrain, OrgBrainConfig};
use std::sync::Arc;
fn entry(project: ProjectId, field: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new("dev.locloud.orgbrain", &format!("{project}:{field}"))
        .map_err(|e| e.to_string())
}
pub fn provider(
    project: ProjectId,
    state: &AppState,
) -> Result<Option<Arc<dyn DurableMemoryProvider>>, String> {
    let value = state
        .store
        .lock()
        .map_err(|e| e.to_string())?
        .setting(&format!("orgbrain:{project}"))
        .map_err(|e| e.to_string())?;
    let Some(value) = value else { return Ok(None) };
    let config: OrgBrainConfig = serde_json::from_str(&value).map_err(|e| e.to_string())?;
    let id = entry(project, "client-id")?
        .get_password()
        .map_err(|_| "OrgBrain の Client ID を Keychain から取得できません。")?;
    let secret = entry(project, "client-secret")?
        .get_password()
        .map_err(|_| "OrgBrain の Client Secret を Keychain から取得できません。")?;
    Ok(Some(Arc::new(
        OrgBrain::new(config, id, secret).map_err(|e| e.to_string())?,
    )))
}
#[tauri::command]
pub fn orgbrain_settings(
    project_id: String,
    state: tauri::State<AppState>,
) -> Result<Option<OrgBrainConfig>, String> {
    let project = ProjectId(project_id.parse().map_err(|_| "invalid project ID")?);
    state
        .store
        .lock()
        .map_err(|e| e.to_string())?
        .setting(&format!("orgbrain:{project}"))
        .map_err(|e| e.to_string())?
        .map(|v| serde_json::from_str(&v).map_err(|e| e.to_string()))
        .transpose()
}
#[tauri::command]
pub async fn set_orgbrain_settings(
    project_id: String,
    config: OrgBrainConfig,
    client_id: String,
    client_secret: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let project = ProjectId(project_id.parse().map_err(|_| "invalid project ID")?);
    if !state
        .store
        .lock()
        .map_err(|e| e.to_string())?
        .projects()
        .map_err(|e| e.to_string())?
        .iter()
        .any(|p| p.id == project)
    {
        return Err("project not registered".into());
    }
    let id = if client_id.is_empty() {
        entry(project, "client-id")?
            .get_password()
            .map_err(|_| "Client ID が必要です。")?
    } else {
        client_id
    };
    let secret = if client_secret.is_empty() {
        entry(project, "client-secret")?
            .get_password()
            .map_err(|_| "Client Secret が必要です。")?
    } else {
        client_secret
    };
    let memory = Arc::new(
        OrgBrain::new(config.clone(), id.clone(), secret.clone()).map_err(|e| e.to_string())?,
    );
    entry(project, "client-id")?
        .set_password(&id)
        .map_err(|_| "Keychain に Client ID を保存できません。")?;
    entry(project, "client-secret")?
        .set_password(&secret)
        .map_err(|_| "Keychain に Client Secret を保存できません。")?;
    state
        .store
        .lock()
        .map_err(|e| e.to_string())?
        .set_setting(
            &format!("orgbrain:{project}"),
            &serde_json::to_string(&config).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    if let Some(c) = state.connection.lock().await.as_ref() {
        c.sessions
            .set_memory(project, memory)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
#[tauri::command]
pub async fn search_memory(
    project_id: String,
    query: String,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<MemoryItem>, String> {
    let project = ProjectId(project_id.parse().map_err(|_| "invalid project ID")?);
    provider(project, &state)?
        .ok_or("OrgBrain はこのプロジェクトに未設定です。")?
        .search(MemorySearchRequest { query, limit: 8 })
        .await
        .map_err(|e| e.to_string())
}
#[derive(serde::Serialize)]
pub struct Capture {
    pub candidates: Vec<DurableMemoryItem>,
    pub saved: bool,
}
#[tauri::command]
pub fn memory_candidates(
    thread_id: String,
    state: tauri::State<AppState>,
) -> Result<Capture, String> {
    let id = crate::parse_thread(thread_id)?;
    let s = state.store.lock().map_err(|e| e.to_string())?;
    let Some(task) = s.thread_task(id).map_err(|e| e.to_string())? else {
        return Ok(Capture {
            candidates: vec![],
            saved: false,
        });
    };
    let candidates = s
        .setting(&format!("memory_candidates:{task}"))
        .map_err(|e| e.to_string())?
        .map(|v| serde_json::from_str(&v))
        .transpose()
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    let saved = s
        .setting(&format!("memory_saved:{task}"))
        .map_err(|e| e.to_string())?
        .as_deref()
        == Some("true");
    Ok(Capture { candidates, saved })
}
#[tauri::command]
pub async fn extract_memory(
    thread_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<DurableMemoryItem>, String> {
    let id = crate::parse_thread(thread_id)?;
    let (task, mut outcome, root) = {
        let s = state.store.lock().map_err(|e| e.to_string())?;
        let task_id = s
            .thread_task(id)
            .map_err(|e| e.to_string())?
            .ok_or("Plan worker を選択してください。")?;
        let mapping = s
            .threads()
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|t| t.id == id)
            .ok_or("thread missing")?;
        let task = s
            .tasks(mapping.project_id)
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|t| t.id == task_id)
            .ok_or("task missing")?;
        if task.status == TaskStatus::Running {
            return Err("worker が停止してから抽出してください。".into());
        }
        let outcome: hub_runtime::workers::WorkerOutcome = serde_json::from_str(
            &s.setting(&format!("outcome:{task_id}"))
                .map_err(|e| e.to_string())?
                .ok_or("worker result missing")?,
        )
        .map_err(|e| e.to_string())?;
        let root = s
            .projects()
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|p| p.id == task.project_id)
            .ok_or("project missing")?
            .root;
        (task, outcome, root)
    };
    outcome.diff = hub_worktree::GitWorktrees::new(&root)
        .map_err(|e| e.to_string())?
        .diff(&outcome.worktree)
        .await
        .map_err(|e| e.to_string())?;
    let memory = provider(task.project_id, &state)?;
    hub_runtime::memory_capture::capture(
        &state.store,
        state.local.provider.as_ref(),
        memory.as_deref(),
        &task,
        &outcome,
    )
    .await
    .map_err(|e| format!("{e:#}"))
}
