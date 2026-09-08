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
    let running = state.running_plans.lock().await;
    let mut store = state.store.lock().map_err(|e| e.to_string())?;
    let thread = store
        .threads()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|t| t.id == id)
        .ok_or("セッションが見つかりません。")?;
    if running.contains(&thread.project_id) {
        return Err("計画の実行が終了してから操作してください。".into());
    }
    let mut ids = vec![id];
    if let Some(w) = crate::workflow::load(&store, &id.to_string())? {
        for leg in [w.chatgpt, w.codex].into_iter().flatten() {
            let leg = crate::parse_thread(leg)?;
            if !ids.contains(&leg) {
                ids.push(leg);
            }
        }
    }
    let all = store.threads().map_err(|e| e.to_string())?;
    if ids.iter().any(|id| {
        all.iter().any(|t| {
            t.id == *id && matches!(t.status.as_str(), "running" | "inProgress" | "dispatching")
        })
    }) {
        return Err("実行完了後に操作してください".into());
    }
    store.check_threads_idle(&ids).map_err(|e| e.to_string())?;
    // Children first; the visible root is removed only after its legs succeed.
    ids.reverse();
    for id in ids {
        match action.as_str() {
            "archive" => store.set_thread_archived(id, true),
            "restore" => store.set_thread_archived(id, false),
            "delete" => store.delete_thread(id),
            _ => return Err("不明な操作です。".into()),
        }
        .map_err(|e| e.to_string())?;
    }
    Ok(())
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
