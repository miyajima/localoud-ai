use crate::AppState;
use tauri_plugin_dialog::DialogExt;

#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathKind {
    Project,
    CodexBinary,
}

#[tauri::command]
pub async fn choose_path(app: tauri::AppHandle, kind: PathKind) -> Result<Option<String>, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let dialog = app.dialog().file();
    match kind {
        PathKind::Project => dialog
            .set_title("プロジェクトのフォルダを選択")
            .pick_folder(move |path| {
                let _ = tx.send(path);
            }),
        PathKind::CodexBinary => {
            dialog
                .set_title("Codex 実行ファイルを選択")
                .pick_file(move |path| {
                    let _ = tx.send(path);
                })
        }
    }
    rx.await
        .map_err(|_| "ファイル選択が終了しました。".to_string())?
        .map(|path| {
            path.into_path()
                .map(|p| p.to_string_lossy().into_owned())
                .map_err(|e| e.to_string())
        })
        .transpose()
}

#[tauri::command]
pub fn rename_thread(
    thread_id: String,
    title: String,
    state: tauri::State<AppState>,
) -> Result<(), String> {
    state
        .store
        .lock()
        .map_err(|e| e.to_string())?
        .rename_thread(crate::parse_thread(thread_id)?, &title)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn open_web_link(app: tauri::AppHandle, url: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let parsed = tauri::Url::parse(&url).map_err(|e| e.to_string())?;
    if !matches!(parsed.scheme(), "https" | "http")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err("HTTP または HTTPS のリンクのみ開けます。".into());
    }
    app.opener()
        .open_url(parsed.as_str(), None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn archived_threads(state: tauri::State<AppState>) -> Result<Vec<String>, String> {
    state
        .store
        .lock()
        .map_err(|e| e.to_string())?
        .archived_threads()
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn manage_session(
    thread_id: String,
    action: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let _workflow = state.workflow_lock.lock().await;
    let id = crate::parse_thread(thread_id)?;
    manage_sessions(&[id], &action, &state).await
}

async fn manage_sessions(
    roots: &[hub_core::HubThreadId],
    action: &str,
    state: &AppState,
) -> Result<(), String> {
    if !matches!(action, "archive" | "restore" | "delete") {
        return Err("不明な操作です。".into());
    }
    let running = state.running_plans.lock().await;
    let (mut ids, all) = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        let all = store.threads().map_err(|e| e.to_string())?;
        let mut ids = roots.to_vec();
        let mut i = 0;
        while i < ids.len() {
            let id = ids[i];
            let thread = all
                .iter()
                .find(|t| t.id == id)
                .ok_or("セッションが見つかりません。")?;
            if running.contains(&thread.project_id) {
                return Err("計画の実行が終了してから操作してください。".into());
            }
            if let Some(w) = crate::workflow::load(&store, &id.to_string())? {
                for leg in [w.chatgpt, w.codex].into_iter().flatten() {
                    let leg = crate::parse_thread(leg)?;
                    if !ids.contains(&leg) {
                        ids.push(leg);
                    }
                }
            }
            for child in &all {
                if store
                    .setting(&format!("autonomous_parent:{}", child.id))
                    .map_err(|e| e.to_string())?
                    .as_deref()
                    == Some(id.to_string().as_str())
                    && !ids.contains(&child.id)
                {
                    ids.push(child.id);
                }
            }
            i += 1;
        }
        if all.iter().any(|t| {
            ids.contains(&t.id)
                && matches!(
                    t.status.as_str(),
                    "queued"
                        | "running"
                        | "inProgress"
                        | "dispatching"
                        | "integrating"
                        | "stopping"
                )
        }) {
            return Err("実行完了後に操作してください。".into());
        }
        store.check_threads_idle(&ids).map_err(|e| e.to_string())?;
        (ids, all)
    };
    // Children first. Keep the visible root available if any provider operation fails.
    ids.reverse();
    for id in ids {
        let thread = all
            .iter()
            .find(|t| t.id == id)
            .ok_or("セッションが見つかりません。")?;
        if thread.provider == "codex" && action != "delete" {
            state
                .sessions()
                .await?
                .set_archived(id, action == "archive")
                .await
                .map_err(|e| {
                    format!(
                        "Codexとのアーカイブ同期に失敗しました（{}）: {e:#}",
                        thread.title
                    )
                })?;
        } else {
            let mut store = state.store.lock().map_err(|e| e.to_string())?;
            match action {
                "archive" => store.set_thread_archived(id, true),
                "restore" => store.set_thread_archived(id, false),
                "delete" => store.delete_thread(id),
                _ => unreachable!(),
            }
            .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn sync_archived_sessions(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<String>, String> {
    let _workflow = state.workflow_lock.lock().await;
    let pending = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        let archived = store.archived_threads().map_err(|e| e.to_string())?;
        let mut pending = vec![];
        for thread in store.threads().map_err(|e| e.to_string())? {
            if archived.contains(&thread.id.to_string())
                && store
                    .setting(&format!("codex_archive_synced:{}", thread.id))
                    .map_err(|e| e.to_string())?
                    .as_deref()
                    != Some("true")
                && thread.provider == "codex"
            {
                pending.push(thread.id);
            }
        }
        pending
    };
    let mut failures = vec![];
    for id in pending {
        if let Err(e) = manage_sessions(&[id], "archive", &state).await {
            failures.push(e);
        }
    }
    Ok(failures)
}
#[tauri::command]
pub async fn unregister_project(
    project_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let _workflow = state.workflow_lock.lock().await;
    let id = hub_core::ProjectId(project_id.parse().map_err(|_| "invalid project ID")?);
    let running = state.running_plans.lock().await;
    if running.contains(&id) {
        return Err("計画の実行が終了してからプロジェクトを削除してください。".into());
    }
    state
        .store
        .lock()
        .map_err(|e| e.to_string())?
        .unregister_project(id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn manage_project(
    app: tauri::AppHandle,
    project_id: String,
    action: String,
    name: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<Option<String>, String> {
    use tauri_plugin_opener::OpenerExt;
    let id = hub_core::ProjectId(project_id.parse().map_err(|_| "invalid project ID")?);
    let _workflow = state.workflow_lock.lock().await;
    let project = state
        .store
        .lock()
        .map_err(|e| e.to_string())?
        .projects()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|p| p.id == id)
        .ok_or("プロジェクトが見つかりません。")?;
    match action.as_str() {
        "rename" => state
            .store
            .lock()
            .map_err(|e| e.to_string())?
            .rename_project(id, name.as_deref().ok_or("名前が必要です。")?)
            .map_err(|e| e.to_string())?,
        "reveal" => app
            .opener()
            .open_path(project.root.to_string_lossy(), None::<&str>)
            .map_err(|e| e.to_string())?,
        "archive" => {
            if state.running_plans.lock().await.contains(&id) {
                return Err("計画の実行完了後に操作してください。".into());
            }
            let ids = state
                .store
                .lock()
                .map_err(|e| e.to_string())?
                .threads()
                .map_err(|e| e.to_string())?
                .into_iter()
                .filter(|t| t.project_id == id)
                .map(|t| t.id)
                .collect::<Vec<_>>();
            manage_sessions(&ids, "archive", &state).await?;
        }
        "worktree" => {
            let tree = hub_worktree::GitWorktrees::new(&project.root)
                .map_err(|e| e.to_string())?
                .create(hub_core::TaskId::default(), "HEAD")
                .await
                .map_err(|e| e.to_string())?;
            let path = tree.path.to_string_lossy().to_string();
            state
                .store
                .lock()
                .map_err(|e| e.to_string())?
                .register_project(&tree.path)
                .map_err(|e| e.to_string())?;
            return Ok(Some(path));
        }
        _ => return Err("不明な操作です。".into()),
    }
    Ok(None)
}
