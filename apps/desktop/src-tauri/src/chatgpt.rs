//! ChatGPT browser transport. This module never imports or invokes Codex.
use crate::{parse_thread, AppState};
use axum::{
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use hub_core::{HubThreadId, ProjectId, ThreadMapping};
use hub_db::Store;
use hub_events::EventBus;
use protocol_types::{AgentEvent, Message, ProviderThread, ProviderTurn, ThreadSnapshot};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::oneshot;
const BRIDGE_PORT: u16 = 8791;
#[derive(Clone, Serialize, Deserialize)]
pub struct BrowserModel {
    pub id: String,
    pub label: String,
    pub efforts: Vec<String>,
}
struct Pending {
    command: Value,
    tx: oneshot::Sender<Result<Value, String>>,
    created: Instant,
}
pub struct Browser {
    token: String,
    store: Arc<Mutex<Store>>,
    bus: EventBus,
    queue: Mutex<VecDeque<Pending>>,
    waiting: Mutex<HashMap<String, Pending>>,
    models: Mutex<Vec<BrowserModel>>,
    last_seen: Mutex<Option<Instant>>,
    error: Mutex<Option<String>>,
}
impl Browser {
    pub fn new(store: Arc<Mutex<Store>>, bus: EventBus) -> Result<Arc<Self>, String> {
        let token = {
            let s = store.lock().map_err(|e| e.to_string())?;
            let get = |key: &str| -> Result<String, String> {
                if let Some(v) = s.setting(key).map_err(|e| e.to_string())? {
                    return Ok(v);
                }
                let v = format!(
                    "{}{}",
                    hub_core::TaskId::default(),
                    hub_core::TaskId::default()
                )
                .replace('-', "");
                s.set_setting(key, &v).map_err(|e| e.to_string())?;
                Ok(v)
            };
            get("chatgpt_bridge_token")?
        };
        Ok(Arc::new(Self {
            token,
            store,
            bus,
            queue: Mutex::new(VecDeque::new()),
            waiting: Mutex::new(HashMap::new()),
            models: Mutex::new(vec![]),
            last_seen: Mutex::new(None),
            error: Mutex::new(None),
        }))
    }
    pub fn start(self: &Arc<Self>) {
        {
            let port = BRIDGE_PORT;
            let b = self.clone();
            tauri::async_runtime::spawn(async move {
                let router = Router::new()
                    .route("/poll", post(poll))
                    .route("/result", post(result))
                    .route("/snapshot", post(snapshot))
                    .layer(DefaultBodyLimit::max(2 * 1024 * 1024))
                    .with_state(b.clone());
                match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
                    Ok(listener) => {
                        if let Err(e) = axum::serve(listener, router).await {
                            *b.error.lock().unwrap() = Some(e.to_string());
                        }
                    }
                    Err(e) => {
                        *b.error.lock().unwrap() = Some(format!("ChatGPT接続ポート {port}: {e}"))
                    }
                }
            });
        }
    }
    fn connected(&self) -> bool {
        self.last_seen
            .lock()
            .unwrap()
            .is_some_and(|v| v.elapsed() < Duration::from_secs(10))
    }
    pub fn is_thread(&self, id: HubThreadId) -> Result<bool, String> {
        Ok(self
            .store
            .lock()
            .map_err(|e| e.to_string())?
            .threads()
            .map_err(|e| e.to_string())?
            .iter()
            .any(|t| t.id == id && t.provider == "chatgpt"))
    }
    pub fn models(&self) -> Vec<BrowserModel> {
        if self.connected() {
            self.models.lock().unwrap().clone()
        } else {
            vec![]
        }
    }
    async fn command(&self, mut command: Value) -> Result<Value, String> {
        if !self.connected() {
            return Err(
                "Chrome拡張を接続し、ChatGPTを開いてください。Codexへの切り替えは行いません。"
                    .into(),
            );
        }
        let id = hub_core::TaskId::default().to_string();
        command["id"] = json!(id);
        let (tx, rx) = oneshot::channel();
        self.queue.lock().unwrap().push_back(Pending {
            command,
            tx,
            created: Instant::now(),
        });
        let answer = tokio::time::timeout(Duration::from_secs(90), rx).await;
        self.queue.lock().unwrap().retain(|p| p.command["id"] != id);
        self.waiting.lock().unwrap().remove(&id);
        answer.map_err(|_|"ブラウザーからの確認がありません。送信済みかChatGPT画面を確認してください。自動再送はしません。".to_string())?.map_err(|_|"ブラウザー接続が終了しました。".to_string())?
    }
    fn mapping(&self, id: HubThreadId) -> Result<ThreadMapping, String> {
        self.store
            .lock()
            .map_err(|e| e.to_string())?
            .threads()
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|t| t.id == id && t.provider == "chatgpt")
            .ok_or("ChatGPTセッションが見つかりません。".into())
    }
    fn emit(&self, t: &ThreadMapping, turn: &str, kind: &str, text: &str, item: Option<String>) {
        let _ = self.bus.publish(AgentEvent {
            thread_id: Some(t.provider_thread_id.clone()),
            turn_id: Some(turn.into()),
            item_id: item,
            kind: kind.into(),
            text: text.into(),
            details: None,
        });
    }
    pub async fn send(&self, id: HubThreadId, text: String) -> Result<(), String> {
        let mut t = self.mapping(id)?;
        {
            let s = self.store.lock().map_err(|e| e.to_string())?;
            if !s
                .projects()
                .map_err(|e| e.to_string())?
                .iter()
                .any(|p| p.id == t.project_id)
                || s.archived_threads()
                    .map_err(|e| e.to_string())?
                    .contains(&id.to_string())
            {
                return Err(
                    "プロジェクトを登録し、アーカイブから復元してから送信してください。".into(),
                );
            }
        }
        if matches!(t.status.as_str(), "running" | "inProgress" | "dispatching") {
            return Err("ChatGPTの応答完了を待つか、停止してから送信してください。".into());
        }
        let (model, effort, conversation) = {
            let s = self.store.lock().unwrap();
            (
                s.setting(&format!("thread_model:{id}"))
                    .map_err(|e| e.to_string())?
                    .ok_or("モデルが未選択です。")?,
                s.setting(&format!("thread_reasoning:{id}"))
                    .map_err(|e| e.to_string())?
                    .filter(|s| !s.is_empty()),
                s.setting(&format!("chatgpt_conversation:{id}"))
                    .map_err(|e| e.to_string())?,
            )
        };
        let turn = hub_core::TaskId::default().to_string();
        {
            let s = self.store.lock().unwrap();
            t.status = "dispatching".into();
            s.save_thread(&t).map_err(|e| e.to_string())?;
            s.set_setting(&format!("chatgpt_turn:{id}"), &turn)
                .map_err(|e| e.to_string())?;
        }
        let prompt = text.clone();
        let outcome=self.command(json!({"kind":"send","session":id.to_string(),"conversation":conversation,"model":model,"effort":effort,"text":prompt,"turn":turn})).await;
        match outcome {
            Ok(_) => {
                self.emit(&t, &turn, "turn_started", "inProgress", None);
                Ok(())
            }
            Err(e) => {
                t.status = "failed".into();
                self.store
                    .lock()
                    .unwrap()
                    .save_thread(&t)
                    .map_err(|e| e.to_string())?;
                self.emit(&t, &turn, "error", &e, None);
                Err(e)
            }
        }
    }
    pub async fn stop(&self, id: HubThreadId) -> Result<(), String> {
        let t = self.mapping(id)?;
        self.command(json!({"kind":"stop","session":id.to_string()}))
            .await?;
        let turn = self
            .store
            .lock()
            .unwrap()
            .setting(&format!("chatgpt_turn:{id}"))
            .map_err(|e| e.to_string())?
            .unwrap_or_default();
        self.emit(&t, &turn, "turn_completed", "interrupted", None);
        Ok(())
    }
    pub async fn resume(&self, id: HubThreadId) -> Result<ThreadSnapshot, String> {
        let snapshot = self.read(id)?;
        if self.connected() {
            let (conversation, turn) = {
                let s = self.store.lock().map_err(|e| e.to_string())?;
                let conversation = s
                    .setting(&format!("chatgpt_conversation:{id}"))
                    .map_err(|e| e.to_string())?;
                let turn = s
                    .setting(&format!("chatgpt_turn:{id}"))
                    .map_err(|e| e.to_string())?;
                (conversation, turn)
            };
            if let (Some(conversation), Some(turn)) = (conversation, turn) {
                self.command(json!({"kind":"resume","session":id.to_string(),"conversation":conversation,"turn":turn})).await?;
            }
        } else if snapshot.active_turn.is_some() {
            return Err("ChatGPT拡張を再接続してから状態を確認してください。".into());
        }
        self.read(id)
    }
    pub fn read(&self, id: HubThreadId) -> Result<ThreadSnapshot, String> {
        let t = self.mapping(id)?;
        let s = self.store.lock().unwrap();
        let messages = s
            .setting(&format!("chatgpt_messages:{id}"))
            .map_err(|e| e.to_string())?
            .map(|v| serde_json::from_str::<Vec<Message>>(&v))
            .transpose()
            .map_err(|e| e.to_string())?
            .unwrap_or_default();
        let active_turn = if matches!(t.status.as_str(), "running" | "inProgress" | "dispatching") {
            Some(ProviderTurn {
                thread_id: t.provider_thread_id.clone(),
                id: s
                    .setting(&format!("chatgpt_turn:{id}"))
                    .map_err(|e| e.to_string())?
                    .unwrap_or_default(),
                status: t.status,
            })
        } else {
            None
        };
        Ok(ThreadSnapshot {
            thread: ProviderThread {
                id: t.provider_thread_id,
            },
            active_turn,
            messages,
        })
    }
}
fn authorized(b: &Browser, h: &HeaderMap) -> bool {
    h.get("authorization")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v == format!("Bearer {}", b.token))
        && h.get("host").and_then(|v| v.to_str().ok()) == Some("127.0.0.1:8791")
        && h.get("origin").is_none_or(|v| {
            v.to_str()
                .is_ok_and(|s| s.starts_with("chrome-extension://"))
        })
}
async fn poll(State(b): State<Arc<Browser>>, h: HeaderMap, Json(v): Json<Value>) -> Response {
    if !authorized(&b, &h) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    *b.last_seen.lock().unwrap() = Some(Instant::now());
    if let Some(models) = v.get("models") {
        if let Ok(models) = serde_json::from_value::<Vec<BrowserModel>>(models.clone()) {
            *b.models.lock().unwrap() = models;
        }
    }
    let mut queue = b.queue.lock().unwrap();
    while queue
        .front()
        .is_some_and(|p| p.created.elapsed() > Duration::from_secs(90))
    {
        queue.pop_front();
    }
    let command = queue.pop_front().map(|p| {
        let command = p.command.clone();
        b.waiting
            .lock()
            .unwrap()
            .insert(command["id"].as_str().unwrap().into(), p);
        command
    });
    Json(json!({"command":command})).into_response()
}
async fn result(State(b): State<Arc<Browser>>, h: HeaderMap, Json(v): Json<Value>) -> Response {
    if !authorized(&b, &h) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if let Some(p) = b
        .waiting
        .lock()
        .unwrap()
        .remove(v["id"].as_str().unwrap_or(""))
    {
        let _ = p.tx.send(if let Some(e) = v["error"].as_str() {
            Err(e.into())
        } else {
            Ok(v["result"].clone())
        });
    }
    StatusCode::NO_CONTENT.into_response()
}
async fn snapshot(State(b): State<Arc<Browser>>, h: HeaderMap, Json(v): Json<Value>) -> Response {
    if !authorized(&b, &h) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Ok(id) = parse_thread(v["session"].as_str().unwrap_or("").into()) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let Ok(t) = b.mapping(id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !matches!(t.status.as_str(), "dispatching" | "running" | "inProgress") {
        return StatusCode::CONFLICT.into_response();
    }
    let Ok(mut messages) = serde_json::from_value::<Vec<Message>>(v["messages"].clone()) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    for message in &mut messages {
        message.text = hub_policy::redact(&message.text);
    }
    let turn = v["turn"].as_str().unwrap_or("");
    let conversation = v["conversation"].as_str().unwrap_or("");
    {
        let s = b.store.lock().unwrap();
        if s.setting(&format!("chatgpt_turn:{id}"))
            .ok()
            .flatten()
            .as_deref()
            != Some(turn)
        {
            return StatusCode::CONFLICT.into_response();
        }
        if let Some(old) = s
            .setting(&format!("chatgpt_conversation:{id}"))
            .ok()
            .flatten()
        {
            if old != conversation {
                return StatusCode::CONFLICT.into_response();
            }
        }
        if !conversation.is_empty() {
            let _ = s.set_setting(&format!("chatgpt_conversation:{id}"), conversation);
        }
        let _ = s.set_setting(
            &format!("chatgpt_messages:{id}"),
            &serde_json::to_string(&messages).unwrap(),
        );
    }
    for (i, m) in messages
        .iter()
        .enumerate()
        .filter(|(_, m)| m.role == "assistant")
    {
        b.emit(
            &t,
            turn,
            "message_completed",
            &m.text,
            Some(format!("chatgpt-{i}")),
        );
    }
    if v["finished"].as_bool() == Some(true) {
        let status = if v["error"].as_str().is_some() {
            "failed"
        } else {
            "completed"
        };
        b.emit(&t, turn, "turn_completed", status, None);
        if let Some(e) = v["error"].as_str() {
            b.emit(&t, turn, "error", e, None);
        }
    }
    StatusCode::NO_CONTENT.into_response()
}
#[tauri::command]
pub fn chatgpt_status(state: tauri::State<AppState>) -> Value {
    let b = &state.browser;
    json!({"connected":b.connected(),"models":b.models(),"token":b.token,"bridge_url":format!("http://127.0.0.1:{BRIDGE_PORT}"),"error":b.error.lock().unwrap().clone(),"extension_path":concat!(env!("CARGO_MANIFEST_DIR"),"/../../../extensions/chatgpt")})
}
#[tauri::command]
pub async fn chatgpt_create(
    project_id: String,
    model: String,
    reasoning: Option<String>,
    text: String,
    state: tauri::State<'_, AppState>,
) -> Result<ThreadMapping, String> {
    let available = state.browser.models();
    if !available
        .iter()
        .any(|m| m.id == model && reasoning.as_ref().is_none_or(|e| m.efforts.contains(e)))
    {
        return Err("ChatGPTで確認済みのモデルを選択してください。".into());
    }
    let project_id = ProjectId(project_id.parse().map_err(|_| "invalid project ID")?);
    let t = ThreadMapping {
        id: HubThreadId::default(),
        project_id,
        provider: "chatgpt".into(),
        provider_thread_id: format!("chatgpt:{}", hub_core::TaskId::default()),
        title: text.chars().take(80).collect(),
        status: "idle".into(),
    };
    {
        let s = state.store.lock().unwrap();
        if !s
            .projects()
            .map_err(|e| e.to_string())?
            .iter()
            .any(|p| p.id == project_id)
        {
            return Err("プロジェクトが見つかりません。".into());
        }
        s.save_thread(&t).map_err(|e| e.to_string())?;
        s.set_setting(&format!("thread_model:{}", t.id), &model)
            .map_err(|e| e.to_string())?;
        if let Some(e) = reasoning {
            s.set_setting(&format!("thread_reasoning:{}", t.id), &e)
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(t)
}
#[tauri::command]
pub async fn chatgpt_send(
    thread_id: String,
    text: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    state.browser.send(parse_thread(thread_id)?, text).await
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, Arc<Browser>, ThreadMapping) {
        let dir = tempfile::tempdir().unwrap();
        assert!(std::process::Command::new("git")
            .args(["init", "-q"])
            .arg(dir.path())
            .status()
            .unwrap()
            .success());
        std::fs::write(dir.path().join("sample.txt"), "before").unwrap();
        let store = Arc::new(Mutex::new(Store::open(&dir.path().join("hub.db")).unwrap()));
        let project = store.lock().unwrap().register_project(dir.path()).unwrap();
        let t = ThreadMapping {
            id: HubThreadId::default(),
            project_id: project.id,
            provider: "chatgpt".into(),
            provider_thread_id: "chatgpt:fixture".into(),
            title: "fixture".into(),
            status: "idle".into(),
        };
        store.lock().unwrap().save_thread(&t).unwrap();
        let bus = EventBus::new(store.clone());
        let b = Browser::new(store, bus).unwrap();
        (dir, b, t)
    }
    #[tokio::test]
    async fn disconnected_send_fails_without_creating_a_codex_provider() {
        let (_dir, b, t) = fixture();
        assert!(b
            .command(json!({"kind":"send"}))
            .await
            .unwrap_err()
            .contains("Codex"));
        assert_eq!(b.mapping(t.id).unwrap().provider, "chatgpt");
    }
    #[tokio::test]
    async fn bridge_requires_token_and_rejects_web_origins() {
        let (_dir, b, _) = fixture();
        let mut headers = HeaderMap::new();
        headers.insert("host", "127.0.0.1:8791".parse().unwrap());
        assert!(!authorized(&b, &headers));
        headers.insert(
            "authorization",
            format!("Bearer {}", b.token).parse().unwrap(),
        );
        assert!(authorized(&b, &headers));
        headers.insert("origin", "https://attacker.example".parse().unwrap());
        assert!(!authorized(&b, &headers));
    }
}
