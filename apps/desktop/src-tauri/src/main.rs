#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod astra;
mod memory;
mod plans;
use hub_core::{ExecutorKind, ExecutorPreference, ModelUsageRecord};
use hub_core::{HubThreadId, Project, ProjectId, ThreadMapping};
use hub_db::Store;
use hub_events::{EventBus, JournalEvent};
use hub_router::RoutingPolicy;
use hub_runtime::{
    local_worker::{LocalExecutor, LocalResult},
    Sessions,
};
use protocol_types::local::*;
use protocol_types::{ProviderTurn, ThreadSnapshot};
use provider_codex::CodexProvider;
use provider_spark::SparkProvider;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tauri::{Emitter, Manager};
use tokio::sync::Mutex as AsyncMutex;
struct Connection {
    provider: Arc<CodexProvider>,
    sessions: Arc<Sessions>,
}
struct AppState {
    store: Arc<Mutex<Store>>,
    bus: EventBus,
    connection: AsyncMutex<Option<Connection>>,
    data_dir: PathBuf,
    local: LocalExecutor,
    running_plans: Arc<AsyncMutex<std::collections::HashSet<ProjectId>>>,
}
impl AppState {
    async fn sessions(&self) -> Result<Arc<Sessions>, String> {
        let mut connection = self.connection.lock().await;
        if let Some(c) = connection.as_ref() {
            if c.provider.is_alive() {
                return Ok(c.sessions.clone());
            }
        }
        *connection = None;
        let bus = self.bus.clone();
        let binary = self
            .store
            .lock()
            .map_err(|e| e.to_string())?
            .setting("codex_binary")
            .map_err(|e| e.to_string())?
            .unwrap_or_else(|| std::env::var("CODEX_BIN").unwrap_or_else(|_| "codex".into()));
        let provider = Arc::new(
            CodexProvider::spawn_with_sink(
                &binary,
                Arc::new(move |e| bus.publish(e)),
                self.data_dir.join("codex-events.jsonl"),
            )
            .await
            .map_err(|e| format!("{e:#}"))?,
        );
        let sessions = Arc::new(Sessions::new(self.store.clone(), provider.clone()));
        let projects = self
            .store
            .lock()
            .map_err(|e| e.to_string())?
            .projects()
            .map_err(|e| e.to_string())?;
        for p in projects {
            match memory::provider(p.id, self) {
                Ok(Some(memory)) => sessions
                    .set_memory(p.id, memory)
                    .map_err(|e| e.to_string())?,
                Ok(None) => {}
                Err(e) => {
                    self.bus
                        .publish(protocol_types::AgentEvent {
                            details: None,
                            thread_id: None,
                            turn_id: None,
                            item_id: None,
                            kind: "memory_unavailable".into(),
                            text: e,
                        })
                        .map_err(|e| e.to_string())?;
                }
            }
        }
        *connection = Some(Connection {
            provider,
            sessions: sessions.clone(),
        });
        Ok(sessions)
    }
}
#[tauri::command]
fn codex_binary(state: tauri::State<AppState>) -> Result<String, String> {
    Ok(state
        .store
        .lock()
        .map_err(|e| e.to_string())?
        .setting("codex_binary")
        .map_err(|e| e.to_string())?
        .unwrap_or_else(|| std::env::var("CODEX_BIN").unwrap_or_else(|_| "codex".into())))
}
#[tauri::command]
async fn set_codex_binary(path: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let path = Path::new(&path);
    if !path.is_absolute() || !path.is_file() {
        return Err("Codex 実行ファイルの絶対パスを指定してください。".into());
    }
    let mut connection = state.connection.lock().await;
    if connection.is_some() {
        return Err("接続設定の変更はアプリを再起動した直後に行ってください。".into());
    }
    state
        .store
        .lock()
        .map_err(|e| e.to_string())?
        .set_setting("codex_binary", &path.to_string_lossy())
        .map_err(|e| e.to_string())?;
    *connection = None;
    Ok(())
}
#[tauri::command]
fn projects(state: tauri::State<AppState>) -> Result<Vec<Project>, String> {
    state
        .store
        .lock()
        .map_err(|e| format!("{e:#}"))?
        .projects()
        .map_err(|e| format!("{e:#}"))
}
#[tauri::command]
fn register_project(path: String, state: tauri::State<AppState>) -> Result<Project, String> {
    state
        .store
        .lock()
        .map_err(|e| format!("{e:#}"))?
        .register_project(Path::new(&path))
        .map_err(|e| format!("{e:#}"))
}
#[tauri::command]
fn threads(state: tauri::State<AppState>) -> Result<Vec<ThreadMapping>, String> {
    state
        .store
        .lock()
        .map_err(|e| format!("{e:#}"))?
        .threads()
        .map_err(|e| format!("{e:#}"))
}
#[derive(serde::Serialize)]
struct RoutedTask {
    thread: ThreadMapping,
    route: RouteReport,
}
#[tauri::command]
async fn create_routed_task(
    project_id: String,
    text: String,
    known_files: Vec<String>,
    preference: ExecutorPreference,
    state: tauri::State<'_, AppState>,
) -> Result<RoutedTask, String> {
    let project = ProjectId(project_id.parse().map_err(|_| "invalid project ID")?);
    let input = RoutingInput {
        request: text.clone(),
        known_files,
        estimated_loc: None,
    };
    let report = RoutingPolicy::default()
        .route(&input, preference, state.local.provider.as_ref())
        .await
        .map_err(|e| format!("{e:#}"))?;
    if let Some(usage) = &report.usage {
        state
            .local
            .record_usage(usage, None)
            .map_err(|e| e.to_string())?;
    }
    let title = text.chars().take(70).collect();
    let thread = match report.decision.executor {
        ExecutorKind::Spark => state
            .local
            .create(project, title)
            .map_err(|e| e.to_string())?,
        ExecutorKind::Codex => state
            .sessions()
            .await?
            .create(project, title)
            .await
            .map_err(|e| format!("{e:#}"))?,
        ExecutorKind::Astra => {
            return Err(
                "この依頼は計画が必要です。Plan の「Astra で計画」を選択してください。Astra が無効の場合は計画 JSON を編集できます。".into(),
            )
        }
    };
    state
        .bus
        .publish(protocol_types::AgentEvent {
            details: None,
            thread_id: Some(thread.provider_thread_id.clone()),
            turn_id: None,
            item_id: None,
            kind: "routing".into(),
            text: format!("{:?}: {}", report.decision.executor, report.decision.reason),
        })
        .map_err(|e| e.to_string())?;
    Ok(RoutedTask {
        thread,
        route: report,
    })
}
#[tauri::command]
async fn run_local(
    thread_id: String,
    text: String,
    known_files: Vec<String>,
    state: tauri::State<'_, AppState>,
) -> Result<LocalResult, String> {
    state
        .local
        .run(parse_thread(thread_id)?, text, known_files)
        .await
        .map_err(|e| format!("{e:#}"))
}
#[tauri::command]
fn model_usage(state: tauri::State<AppState>) -> Result<Vec<ModelUsageRecord>, String> {
    state
        .store
        .lock()
        .map_err(|e| e.to_string())?
        .usage()
        .map_err(|e| e.to_string())
}
#[tauri::command]
async fn summarize_task(
    thread_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<ProgressSummary, String> {
    let id = parse_thread(thread_id)?;
    let events = state
        .store
        .lock()
        .map_err(|e| e.to_string())?
        .event_history(id, 0)
        .map_err(|e| e.to_string())?;
    let selected: Vec<String> = events
        .iter()
        .rev()
        .filter_map(|(_, body)| serde_json::from_str::<protocol_types::AgentEvent>(body).ok())
        .filter(|e| !matches!(e.kind.as_str(), "message_delta" | "tool_output"))
        .take(12)
        .map(|e| {
            format!(
                "{}: {}",
                e.kind,
                e.text.chars().take(800).collect::<String>()
            )
        })
        .collect();
    let result = state
        .local
        .provider
        .summarize_progress(&selected)
        .await
        .map_err(|e| format!("{e:#}"))?;
    state
        .local
        .record_usage(&result.usage, None)
        .map_err(|e| e.to_string())?;
    Ok(result.output)
}
#[tauri::command]
async fn review_task(
    thread_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<ReviewResult, String> {
    let id = parse_thread(thread_id)?;
    let mapping = state
        .store
        .lock()
        .map_err(|e| e.to_string())?
        .threads()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|t| t.id == id)
        .ok_or("unknown thread")?;
    let diff = hub_runtime::repository_diff(&state.store, id)
        .await
        .map_err(|e| format!("{e:#}"))?;
    if diff.len() > 24000 {
        return Err("差分がローカルレビューの上限を超えています。".into());
    }
    let result = state
        .local
        .provider
        .review_diff(&mapping.title, &diff)
        .await
        .map_err(|e| format!("{e:#}"))?;
    state
        .local
        .record_usage(&result.usage, None)
        .map_err(|e| e.to_string())?;
    Ok(result.output)
}
#[tauri::command]
async fn create_task(
    project_id: String,
    title: String,
    state: tauri::State<'_, AppState>,
) -> Result<ThreadMapping, String> {
    let project = ProjectId(project_id.parse().map_err(|_| "invalid project ID")?);
    state
        .sessions()
        .await?
        .create(project, title)
        .await
        .map_err(|e| format!("{e:#}"))
}
#[tauri::command]
async fn resume_task(
    thread_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<ThreadSnapshot, String> {
    state
        .sessions()
        .await?
        .resume(parse_thread(thread_id)?)
        .await
        .map_err(|e| format!("{e:#}"))
}
#[tauri::command]
async fn send_turn(
    thread_id: String,
    text: String,
    state: tauri::State<'_, AppState>,
) -> Result<ProviderTurn, String> {
    state
        .sessions()
        .await?
        .start(parse_thread(thread_id)?, text)
        .await
        .map_err(|e| format!("{e:#}"))
}
#[tauri::command]
async fn steer_turn(
    thread_id: String,
    text: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    state
        .sessions()
        .await?
        .steer(parse_thread(thread_id)?, text)
        .await
        .map_err(|e| format!("{e:#}"))
}
#[tauri::command]
async fn interrupt_turn(
    thread_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    state
        .sessions()
        .await?
        .interrupt(parse_thread(thread_id)?)
        .await
        .map_err(|e| format!("{e:#}"))
}
#[tauri::command]
async fn repo_diff(thread_id: String, state: tauri::State<'_, AppState>) -> Result<String, String> {
    hub_runtime::repository_diff(&state.store, parse_thread(thread_id)?)
        .await
        .map_err(|e| format!("{e:#}"))
}
#[tauri::command]
fn event_history(
    thread_id: String,
    after: i64,
    state: tauri::State<AppState>,
) -> Result<Vec<JournalEvent>, String> {
    state
        .store
        .lock()
        .map_err(|e| format!("{e:#}"))?
        .event_history(parse_thread(thread_id)?, after)
        .map_err(|e| format!("{e:#}"))?
        .into_iter()
        .map(|(sequence, body)| {
            serde_json::from_str(&body)
                .map(|event| JournalEvent { sequence, event })
                .map_err(|e| format!("{e:#}"))
        })
        .collect()
}
fn parse_thread(id: String) -> Result<HubThreadId, String> {
    id.parse()
        .map(HubThreadId)
        .map_err(|_| "invalid thread ID".into())
}
fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let store = Arc::new(Mutex::new(Store::open(&dir.join("hub.db"))?));
            let bus = EventBus::new(store.clone());
            let mut events = bus.subscribe();
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    match events.recv().await {
                        Ok(event) => {
                            let _ = handle.emit("hub-event", event);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                            let _ = handle.emit("hub-stream-gap", count);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            });
            let local = LocalExecutor {
                store: store.clone(),
                bus: bus.clone(),
                provider: Arc::new(SparkProvider::new("http://127.0.0.1:8765")?),
                lock: AsyncMutex::new(()),
            };
            app.manage(AppState {
                store,
                bus,
                connection: AsyncMutex::new(None),
                data_dir: dir,
                local,
                running_plans: Arc::new(AsyncMutex::new(std::collections::HashSet::new())),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            memory::extract_memory,
            memory::orgbrain_settings,
            memory::set_orgbrain_settings,
            memory::search_memory,
            memory::memory_candidates,
            astra::worker_insights,
            astra::astra_settings,
            astra::set_astra_settings,
            astra::create_astra_plan,
            astra::final_review,
            astra::diagnose_task,
            astra::apply_rework,
            astra::apply_recovery,
            plans::task_graph,
            plans::inspect_context,
            plans::run_plan,
            plans::resume_plan,
            codex_binary,
            set_codex_binary,
            projects,
            register_project,
            threads,
            create_task,
            create_routed_task,
            run_local,
            model_usage,
            summarize_task,
            review_task,
            resume_task,
            send_turn,
            steer_turn,
            interrupt_turn,
            event_history,
            repo_diff
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Astra Hub");
}
