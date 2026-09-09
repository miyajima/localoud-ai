//! Read-only presentation of the retired ChatGPT/Codex phase workflow.
use crate::AppState;
use hub_core::ThreadMapping;
use protocol_types::Message;
use serde::Deserialize;
use serde_json::{json, Value};
#[derive(Deserialize)]
pub struct Workflow {
    pub root: String,
    pub chatgpt: Option<String>,
    pub codex: Option<String>,
    runs: Vec<Run>,
}
#[derive(Deserialize)]
struct Run {
    phase: String,
    model: String,
    input: String,
    messages: Vec<Message>,
}
pub fn load(store: &hub_db::Store, id: &str) -> Result<Option<Workflow>, String> {
    store
        .setting(&format!("workflow:{id}"))
        .map_err(|e| e.to_string())?
        .map(|s| serde_json::from_str(&s).map_err(|e| e.to_string()))
        .transpose()
}
pub fn project_threads(store: &hub_db::Store) -> Result<Vec<ThreadMapping>, String> {
    let mut threads = store.threads().map_err(|e| e.to_string())?;
    let mut hidden = std::collections::HashSet::new();
    for t in &mut threads {
        if let Some(w) = load(store, &t.id.to_string())? {
            for id in [w.chatgpt, w.codex].into_iter().flatten() {
                if id != w.root {
                    hidden.insert(id);
                }
            }
            t.provider = "workflow".into();
            t.status = "legacy_read_only".into();
        }
    }
    threads.retain(|t| !hidden.contains(&t.id.to_string()));
    Ok(threads)
}
#[tauri::command]
pub fn workflow_snapshot(
    thread_id: String,
    state: tauri::State<AppState>,
) -> Result<Value, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    let w = load(&store, &thread_id)?.ok_or("旧ワークフローがありません")?;
    let mut messages = vec![];
    for (i, r) in w.runs.iter().enumerate() {
        messages.push(json!({"role":"user","text":r.input,"key":format!("legacy-{i}-user"),"label":format!("{} · {}",r.phase,r.model)}));
        for (j, m) in r.messages.iter().enumerate() {
            messages.push(json!({"role":m.role,"text":m.text,"key":format!("legacy-{i}-{j}")}));
        }
    }
    messages.push(json!({"role":"assistant","text":"旧ブラウザ連携の記録です。再送・再開はできません。内蔵ChatGPTで新しいManifestを作成してください。","key":"legacy-notice"}));
    Ok(
        json!({"running":false,"status":"legacy_read_only","messages":messages,"phase":null,"model":"保存済みの記録","targets":{},"handoff":""}),
    )
}
