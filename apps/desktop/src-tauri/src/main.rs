#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod astra;
mod auto_routing;
mod autonomous;
mod composer;
mod embedded_browser;
mod legacy_browser;
mod manifest;
mod memory;
mod models;
mod plans;
mod read_mcp;
mod workflow;
mod workspace_ui;
use hub_core::{ExecutorKind, ExecutorPreference};
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
use tokio::sync::RwLock as AsyncRwLock;
struct Connection {
    provider: Arc<CodexProvider>,
    sessions: Arc<Sessions>,
}
struct AppState {
    workflow_lock: AsyncMutex<()>,
    read_mcp: Arc<read_mcp::ReadMcp>,
    local_stops: AsyncMutex<std::collections::HashMap<HubThreadId, Arc<tokio::sync::Notify>>>,
    store: Arc<Mutex<Store>>,
    bus: EventBus,
    connection: AsyncMutex<Option<Connection>>,
    data_dir: PathBuf,
    local: LocalExecutor,
    local_config: provider_spark::LocalServiceConfig,
    providers: Arc<AsyncRwLock<provider_api::ProviderRegistry>>,
    api_sessions: Arc<hub_runtime::api_sessions::ApiSessions>,
    command_approvals: Arc<hub_runtime::api_sessions::CommandApprovals>,
    auto_routes: Mutex<std::collections::HashMap<String, auto_routing::PreparedRoute>>,
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
    let running = state.running_plans.lock().await;
    if !running.is_empty() {
        return Err("計画の実行が終了してから接続設定を変更してください。".into());
    }
    let mut connection = state.connection.lock().await;
    if state
        .store
        .lock()
        .map_err(|e| e.to_string())?
        .threads()
        .map_err(|e| e.to_string())?
        .iter()
        .any(|t| matches!(t.status.as_str(), "running" | "inProgress"))
    {
        return Err("実行中のタスクが終了してから接続設定を変更してください。".into());
    }
    if let Some(c) = connection.as_ref() {
        c.provider.shutdown().await.map_err(|e| e.to_string())?;
    }
    *connection = None;
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
    let store = state.store.lock().map_err(|e| e.to_string())?;
    let threads = workflow::project_threads(&store)?;
    let mut visible = Vec::new();
    for thread in threads {
        if store
            .setting(&format!("autonomous_parent:{}", thread.id))
            .map_err(|e| e.to_string())?
            .is_none()
        {
            visible.push(thread);
        }
    }
    Ok(visible)
}
#[derive(serde::Serialize)]
struct RoutedTask {
    thread: ThreadMapping,
    route: RouteReport,
    target: hub_router::automatic::ModelTarget,
    started: bool,
}
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateTaskRequest {
    project_id: String,
    text: String,
    known_files: Vec<String>,
    preference: ExecutorPreference,
    profile_id: Option<String>,
    model: Option<String>,
    reasoning: Option<String>,
    auto_route_id: Option<String>,
    auto_route_revision: Option<u64>,
    #[serde(default)]
    reviewed_write: bool,
}
#[tauri::command]
async fn create_routed_task(
    request: CreateTaskRequest,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<RoutedTask, String> {
    use hub_router::automatic::{ModelProvider, ModelTarget};
    let CreateTaskRequest {
        project_id,
        text,
        known_files,
        preference,
        profile_id,
        model,
        reasoning,
        auto_route_id,
        auto_route_revision,
        reviewed_write,
    } = request;
    if matches!(
        preference,
        ExecutorPreference::Codex | ExecutorPreference::Api
    ) && model.as_ref().is_none_or(|m| m.trim().is_empty())
    {
        return Err("モデルを選択してください。".into());
    }
    let project = ProjectId(project_id.parse().map_err(|_| "invalid project ID")?);
    let input = RoutingInput {
        request: text.clone(),
        known_files,
        estimated_loc: None,
    };
    let mut resolved_auto_level = None;
    let (report, target) = if preference == ExecutorPreference::Auto {
        let preview = auto_routing::consume_route(
            auto_route_id
                .as_deref()
                .ok_or("先に Auto の判定を行ってください。")?,
            auto_route_revision.ok_or("判定の版がありません。")?,
            project,
            &input,
            &state,
        )
        .await?;
        resolved_auto_level = preview.level;
        let target = preview.target.clone().ok_or("振り分け先がありません。")?;
        let decision = RoutingDecision {
            executor: if target.provider == ModelProvider::Local {
                ExecutorKind::Spark
            } else if target.provider == ModelProvider::Api {
                ExecutorKind::Api
            } else {
                ExecutorKind::Codex
            },
            complexity: match preview.level {
                Some(1 | 2) => hub_core::Complexity::Trivial,
                Some(4 | 5) => hub_core::Complexity::Deep,
                _ => hub_core::Complexity::Normal,
            },
            risk: preview.risk.unwrap_or(hub_core::RiskLevel::Medium),
            needs_plan: false,
            needs_final_astra_review: false,
            estimated_scope: preview.estimated_scope.clone().unwrap_or(EstimatedScope {
                files: input.known_files.len() as u32,
                loc: 0,
            }),
            confidence: preview.confidence.unwrap_or(0.0),
            reason: format!(
                "Auto: {} · 難易度 {}{} → {} / reasoning={}\n{}",
                preview.agent_name.as_deref().unwrap_or("Agent"),
                preview
                    .level
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "未判定".into()),
                if preview.manual_override {
                    "（手動調整）"
                } else {
                    ""
                },
                target.model,
                target.reasoning.as_deref().unwrap_or("既定"),
                preview.reason
            ),
        };
        (
            RouteReport {
                decision,
                usage: None,
                fallback: preview.used_fallback,
            },
            target,
        )
    } else if preference == ExecutorPreference::Api {
        let target = ModelTarget {
            provider: ModelProvider::Api,
            profile_id: Some(profile_id.ok_or("API provider profileを選択してください。")?),
            model: model.ok_or("モデルを選択してください。")?,
            reasoning,
        };
        models::validate_target(&target, &state).await?;
        (
            RouteReport {
                decision: RoutingDecision {
                    executor: ExecutorKind::Api,
                    complexity: hub_core::Complexity::Normal,
                    risk: hub_core::RiskLevel::Medium,
                    needs_plan: false,
                    needs_final_astra_review: false,
                    estimated_scope: EstimatedScope {
                        files: input.known_files.len() as u32,
                        loc: 0,
                    },
                    confidence: 1.0,
                    reason: "明示指定したAPI provider/modelを使用".into(),
                },
                usage: None,
                fallback: false,
            },
            target,
        )
    } else {
        let report = RoutingPolicy::default()
            .route(&input, preference, state.local.provider.as_ref())
            .await
            .map_err(|e| format!("{e:#}"))?;
        let provider = match report.decision.executor {
            ExecutorKind::Spark => ModelProvider::Local,
            ExecutorKind::Codex => ModelProvider::Codex,
            ExecutorKind::Api => unreachable!(),
            ExecutorKind::Astra => return Err("操作を「計画する」に切り替えてください。".into()),
        };
        let target = ModelTarget {
            provider,
            profile_id: Some(
                match provider {
                    ModelProvider::Local => "spark",
                    ModelProvider::Codex => "codex",
                    ModelProvider::Api => unreachable!(),
                }
                .into(),
            ),
            model: model.unwrap_or_else(|| state.local_config.model_id.clone()),
            reasoning,
        };
        models::validate_target(&target, &state).await?;
        (report, target)
    };
    if report.decision.executor == ExecutorKind::Spark && !reviewed_write {
        return Err(
            "ローカル実装には対象ファイルを1〜2件指定してください。書き込みは専用worktreeと別セッションレビューを通します。"
                .into(),
        );
    }
    if reviewed_write {
        if input.known_files.is_empty() {
            return Err(
                "実装対象のファイルを指定するか、「ChatGPTで計画」から範囲を確認してください。"
                    .into(),
            );
        }
        let root = state
            .store
            .lock()
            .map_err(|e| e.to_string())?
            .projects()
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|candidate| candidate.id == project)
            .ok_or("project not found")?
            .root;
        let base_revision = autonomous::project_head(&root).await?;
        let level = resolved_auto_level.unwrap_or_else(|| match report.decision.complexity {
            hub_core::Complexity::Trivial => 1,
            hub_core::Complexity::Normal => 3,
            hub_core::Complexity::Deep => 5,
        });
        let risk = match report.decision.risk {
            hub_core::RiskLevel::Low => "low",
            hub_core::RiskLevel::Medium => "medium",
            hub_core::RiskLevel::High => "high",
        };
        let manifest = manifest::TaskManifest {
            version: 1,
            manifest_id: format!("direct-{}", uuid::Uuid::new_v4()),
            project_id: project.to_string(),
            base_revision,
            request: text.clone(),
            acceptance: vec![
                "The requested behavior is implemented within the selected files.".into(),
                "Relevant verification is run and its exact result is recorded.".into(),
                "A separate reviewer session passes the unchanged artifact.".into(),
            ],
            scope: input.known_files.clone(),
            steps: vec![autonomous::PlanStep {
                key: "implementation".into(),
                title: text.chars().take(70).collect(),
                goal: text.clone(),
                dependencies: vec![],
                level,
                target_override: Some(target.clone()),
                owned_paths: input.known_files.clone(),
                acceptance: vec![
                    "Implement the request only in the selected files.".into(),
                    "Run the relevant checks and report their exact outcome.".into(),
                ],
                risk: Some(risk.into()),
                estimated_loc: Some(report.decision.estimated_scope.loc),
            }],
        };
        let _lock = state.workflow_lock.lock().await;
        let thread = autonomous::import_task(manifest, app, state.clone()).await?;
        return Ok(RoutedTask {
            thread,
            route: report,
            target,
            started: true,
        });
    }
    let title = text.chars().take(70).collect();
    let thread = match report.decision.executor {
        ExecutorKind::Spark => state
            .local
            .create(project, title)
            .map_err(|e| e.to_string())?,
        ExecutorKind::Codex => {
            let sessions = state.sessions().await?;
            sessions.create_with_model(project, title, Some(&target.model)).await.map_err(|e|format!("{e:#}"))?
        },
        ExecutorKind::Api => state
            .api_sessions
            .create(
                project,
                title,
                protocol_types::providers::ModelTarget {
                    profile_id: target.profile_id.clone().ok_or("API provider profileがありません。")?,
                    model_id: target.model.clone(),
                    effort: target.reasoning.clone(),
                },
            )
            .await
            .map_err(|e| format!("{e:#}"))?,
        ExecutorKind::Astra => {
            return Err(
                "この依頼は計画が必要です。入力欄の操作を「計画する」に切り替え、モデルを選択してください。".into(),
            )
        }
    };
    if let Some(effort) = &target.reasoning {
        state
            .store
            .lock()
            .map_err(|e| e.to_string())?
            .set_setting(&format!("thread_reasoning:{}", thread.id), effort)
            .map_err(|e| e.to_string())?;
    }
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
        target,
        started: false,
    })
}
#[tauri::command]
async fn run_local(
    _thread_id: String,
    _text: String,
    _known_files: Vec<String>,
    _state: tauri::State<'_, AppState>,
) -> Result<LocalResult, String> {
    Err("旧来の直接ローカル書き込みは無効です。対象ファイルを指定して新しいタスクを開始すると、専用worktreeと別セッションレビューを通して実行します。".into())
}
#[tauri::command]
fn model_usage(
    thread_id: Option<String>,
    state: tauri::State<AppState>,
) -> Result<Vec<serde_json::Value>, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    let rows = match thread_id {
        Some(id) => store.thread_usage(parse_thread(id)?),
        None => store.usage(),
    }.map_err(|e| e.to_string())?;
    let projects = store.usage_projects().map_err(|e| e.to_string())?;
    rows.into_iter().map(|row| {
        let mut value = serde_json::to_value(&row).map_err(|e| e.to_string())?;
        let project = projects.get(&row.id);
        value["project_id"] = serde_json::json!(project.map(|p| &p.0));
        value["project_name"] = serde_json::json!(project.map(|p| &p.1));
        Ok(value)
    }).collect()
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
    let id = parse_thread(thread_id.clone())?;
    if legacy_browser::is_thread(&state, id)? {
        return legacy_browser::read(&state, id);
    }
    if thread_provider(id, &state)?.starts_with("api:") {
        return state
            .api_sessions
            .resume(id)
            .await
            .map_err(|e| format!("{e:#}"));
    }
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
    let id = parse_thread(thread_id)?;
    if thread_provider(id, &state)?.starts_with("api:") {
        return state
            .api_sessions
            .start(id, text, protocol_types::composer::TurnOptions::default())
            .await
            .map_err(|e| format!("{e:#}"));
    }
    state
        .sessions()
        .await?
        .start(id, text)
        .await
        .map_err(|e| format!("{e:#}"))
}
#[tauri::command]
async fn steer_turn(
    thread_id: String,
    text: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let id = parse_thread(thread_id)?;
    if thread_provider(id, &state)?.starts_with("api:") {
        return Err(
            "APIセッションの実行中ターンは変更できません。停止後に次のターンを送信してください。"
                .into(),
        );
    }
    state
        .sessions()
        .await?
        .steer(id, text)
        .await
        .map_err(|e| format!("{e:#}"))
}
#[tauri::command]
async fn interrupt_turn(
    thread_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let local_id = parse_thread(thread_id.clone())?;
    if let Some(stop) = state.local_stops.lock().await.get(&local_id) {
        stop.notify_one();
        return Ok(());
    }
    let id = parse_thread(thread_id.clone())?;
    if legacy_browser::is_thread(&state, id)? {
        return Err("旧ブラウザ連携は廃止済みです".into());
    }
    if thread_provider(id, &state)?.starts_with("api:") {
        return state
            .api_sessions
            .interrupt(id)
            .await
            .map_err(|e| format!("{e:#}"));
    }
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
fn thread_provider(id: HubThreadId, state: &AppState) -> Result<String, String> {
    state
        .store
        .lock()
        .map_err(|e| e.to_string())?
        .threads()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|thread| thread.id == id)
        .map(|thread| thread.provider)
        .ok_or_else(|| "タスクが見つかりません。".into())
}

#[tauri::command]
async fn decide_api_tool_approval(
    approval_id: String,
    approve: bool,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    state
        .command_approvals
        .decide(&approval_id, approve)
        .await
        .map_err(|error| format!("{error:#}"))
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let store = Arc::new(Mutex::new(Store::open(&dir.join("hub.db"))?));
            let bus = EventBus::new(store.clone());
            let command_approvals = Arc::new(
                hub_runtime::api_sessions::CommandApprovals::new(store.clone(), bus.clone()),
            );
            let mut events = bus.subscribe();
            let handle = app.handle().clone();
            let journal_store = store.clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    match events.recv().await {
                        Ok(event) => {
                            if event.event.kind == "message_completed" {
                                if let Ok(store) = journal_store.lock() {
                                    if let Ok(Some(mapping)) = store.threads().map(|threads| {
                                        threads.into_iter().find(|mapping| {
                                            mapping.provider == "codex"
                                                && event.event.thread_id.as_deref()
                                                    == Some(&mapping.provider_thread_id)
                                        })
                                    }) {
                                        let segment = store
                                            .active_provider_segment(mapping.id)
                                            .ok()
                                            .flatten()
                                            .map(|segment| segment.id);
                                        let _ = store.append_provider_message(
                                            &hub_db::ProviderMessageRecord {
                                                id: hub_core::TaskId::default().to_string(),
                                                thread_id: mapping.id,
                                                segment_id: segment,
                                                provider_turn_id: event.event.turn_id.clone(),
                                                role: protocol_types::providers::TranscriptRole::Assistant,
                                                content: vec![
                                                    protocol_types::providers::ContentBlock::Text {
                                                        text: event.event.text.clone(),
                                                    },
                                                ],
                                                provider_state: None,
                                            },
                                        );
                                    }
                                }
                            }
                            let _ = handle.emit("hub-event", event);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                            let _ = handle.emit("hub-stream-gap", count);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            });
            let local_config = models::read_local(
                &*store
                    .lock()
                    .map_err(|e| std::io::Error::other(e.to_string()))?,
            )
            .map_err(std::io::Error::other)?;
            models::ensure_builtin_profiles(
                &*store
                    .lock()
                    .map_err(|e| std::io::Error::other(e.to_string()))?,
                &local_config,
            )
            .map_err(std::io::Error::other)?;
            let providers = Arc::new(AsyncRwLock::new(
                provider_api::ProviderRegistry::from_profiles(
                    store
                        .lock()
                        .map_err(|e| std::io::Error::other(e.to_string()))?
                        .provider_profiles()?,
                )?,
            ));
            let api_sessions = Arc::new(hub_runtime::api_sessions::ApiSessions::new(
                store.clone(),
                bus.clone(),
                providers.clone(),
            ));
            let local = LocalExecutor {
                store: store.clone(),
                bus: bus.clone(),
                provider: Arc::new(SparkProvider::configured(local_config.clone())?),
                lock: AsyncMutex::new(()),
            };
            legacy_browser::retire(
                &*store
                    .lock()
                    .map_err(|e| std::io::Error::other(e.to_string()))?,
            )
            .map_err(std::io::Error::other)?;
            let read_mcp = read_mcp::ReadMcp::new(store.clone());
            read_mcp.start();
            app.manage(AppState {
                read_mcp,
                workflow_lock: AsyncMutex::new(()),
                local_stops: AsyncMutex::new(std::collections::HashMap::new()),
                store,
                bus,
                connection: AsyncMutex::new(None),
                data_dir: dir,
                local,
                local_config,
                providers,
                api_sessions,
                command_approvals,
                auto_routes: Mutex::new(std::collections::HashMap::new()),
                running_plans: Arc::new(AsyncMutex::new(std::collections::HashSet::new())),
            });
            Ok(())
        })
        .invoke_handler(|invoke: tauri::ipc::Invoke<tauri::Wry>| {
            if invoke.message.webview_ref().label() != "main" {
                invoke
                    .resolver
                    .reject("Remote browser has no Localoud command access");
                return true;
            }
            let handler: Box<dyn Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool> =
                Box::new(tauri::generate_handler![
                    read_mcp::mcp_settings,
                    read_mcp::set_mcp_settings,
                    read_mcp::mcp_copy_token,
                    read_mcp::manifest_format,
                    manifest::manifest_import_clipboard,
                    manifest::manifest_preview_clipboard,
                    manifest::manifest_import_text,
                    embedded_browser::browser_layout,
                    embedded_browser::browser_reload,
                    embedded_browser::browser_back,
                    embedded_browser::browser_home,
                    autonomous::autonomous_snapshot,
                    autonomous::autonomous_activity,
                    autonomous::autonomous_stop,
                    autonomous::autonomous_resume,
                    workflow::workflow_snapshot,
                    auto_routing::auto_settings,
                    auto_routing::set_auto_settings,
                    auto_routing::preview_auto_route,
                    auto_routing::revise_auto_route,
                    models::thread_reasoning,
                    models::thread_model_targets,
                    models::set_thread_model_target,
                    composer::composer_catalog,
                    composer::composer_files,
                    composer::send_composed_turn,
                    composer::composer_thread_state,
                    composer::answer_composer_question,
                    composer::pause_composer_goal,
                    composer::prompt_history,
                    composer::remember_prompt,
                    composer::clear_prompt_history,
                    composer::completion_settings,
                    composer::set_completion_settings,
                    composer::complete_prompt,
                    models::available_models,
                    models::provider_profiles,
                    models::set_provider_profile,
                    models::run_provider_tool_canary,
                    models::local_model_settings,
                    models::set_local_model_settings,
                    models::thread_models,
                    workspace_ui::choose_path,
                    workspace_ui::open_web_link,
                    workspace_ui::rename_thread,
                    workspace_ui::archived_threads,
                    workspace_ui::sync_archived_sessions,
                    workspace_ui::manage_session,
                    workspace_ui::unregister_project,
                    workspace_ui::manage_project,
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
                    decide_api_tool_approval,
                    event_history,
                    repo_diff
                ]);
            handler(invoke)
        })
        .run(tauri::generate_context!())
        .expect("failed to run Localoud AI");
}
