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
