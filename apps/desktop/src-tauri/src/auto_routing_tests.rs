use super::*;
use hub_db::Store;
use hub_events::EventBus;
use hub_runtime::local_worker::LocalExecutor;
use provider_spark::{LocalProtocol, LocalServiceConfig, SparkProvider};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Fixture {
    state: AppState,
    project: ProjectId,
    _directory: tempfile::TempDir,
    server: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}
fn assessment() -> DifficultyAssessment {
    DifficultyAssessment {
        difficulty: 1,
        confidence: 0.91,
        reason: "one known text replacement".into(),
        risk: hub_core::RiskLevel::Low,
        needs_plan: false,
        estimated_scope: protocol_types::local::EstimatedScope { files: 1, loc: 1 },
    }
}
async fn fixture() -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    assert!(std::process::Command::new("git")
        .arg("init")
        .arg(directory.path())
        .output()
        .unwrap()
        .status
        .success());
    std::fs::write(directory.path().join("greeting.txt"), "unchanged").unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut request = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let count = socket.read(&mut chunk).await.unwrap();
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..count]);
                if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..end]);
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            let value = if request.starts_with(b"GET /health ") {
                serde_json::json!({"status":"ready","model":"fixture/local","quantization_bits":8})
            } else {
                serde_json::json!({"output":assessment(),"usage":{"prompt_tokens":3,"completion_tokens":2},"latency_ms":1,"model":"fixture/local","quantization_bits":8})
            };
            let body = serde_json::to_string(&value).unwrap();
            let response=format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body);
            socket.write_all(response.as_bytes()).await.unwrap();
        }
    });
    let store = Arc::new(Mutex::new(
        Store::open(&directory.path().join("hub.db")).unwrap(),
    ));
    let project = store
        .lock()
        .unwrap()
        .register_project(directory.path())
        .unwrap()
        .id;
    let local_config = LocalServiceConfig {
        endpoint,
        model_id: "fixture/local".into(),
        display_name: "Fixture".into(),
        quantization_bits: 8,
        protocol: LocalProtocol::Dedicated,
    };
    let bus = EventBus::new(store.clone());
    let local = LocalExecutor {
        store: store.clone(),
        bus: bus.clone(),
        provider: Arc::new(SparkProvider::configured(local_config.clone()).unwrap()),
        lock: tokio::sync::Mutex::new(()),
    };
    models::ensure_builtin_profiles(&store.lock().unwrap(), &local_config).unwrap();
    let providers = Arc::new(tokio::sync::RwLock::new(
        provider_api::ProviderRegistry::from_profiles(
            store.lock().unwrap().provider_profiles().unwrap(),
        )
        .unwrap(),
    ));
    let api_sessions = Arc::new(hub_runtime::api_sessions::ApiSessions::new(
        store.clone(),
        bus.clone(),
        providers.clone(),
    ));
    let command_approvals = Arc::new(hub_runtime::api_sessions::CommandApprovals::new(
        store.clone(),
        bus.clone(),
    ));
    let state = AppState {
        read_mcp: crate::read_mcp::ReadMcp::new(store.clone()),
        workflow_lock: tokio::sync::Mutex::new(()),
        local_stops: tokio::sync::Mutex::new(HashMap::new()),
        store,
        bus,
        connection: tokio::sync::Mutex::new(None),
        data_dir: directory.path().to_path_buf(),
        local,
        local_config,
        providers,
        api_sessions,
        command_approvals,
        auto_routes: Mutex::new(HashMap::new()),
        running_plans: Arc::new(tokio::sync::Mutex::new(HashSet::new())),
    };
    let config = AutoSettings::initial("fixture/local".into(), None);
    state
        .store
        .lock()
        .unwrap()
        .set_setting(
            "auto_routing_settings",
            &serde_json::to_string(&config).unwrap(),
        )
        .unwrap();
    Fixture {
        state,
        project,
        _directory: directory,
        server,
    }
}
fn prepared(f: &Fixture) -> PreparedRoute {
    let settings: AutoSettings =
        serde_json::from_str(&saved_settings(&f.state).unwrap().unwrap()).unwrap();
    PreparedRoute {
        project: f.project,
        input: RoutingInput {
            request: "fix typo".into(),
            known_files: vec!["greeting.txt".into()],
            estimated_loc: None,
        },
        settings,
        settings_revision: saved_settings(&f.state).unwrap(),
        assessment: Some(assessment()),
        created: Instant::now(),
        preview: AutoPreview {
            id: TaskId::default().to_string(),
            revision: 0,
            level: Some(1),
            risk: Some(hub_core::RiskLevel::Low),
            confidence: Some(0.91),
            estimated_scope: Some(assessment().estimated_scope),
            target: None,
            reason: "known typo".into(),
            blocked: None,
            used_fallback: false,
            needs_plan: false,
            confirm_before_run: true,
            manual_override: false,
        },
    }
}
#[tokio::test]
async fn classifier_uses_the_local_difficulty_protocol_and_records_usage() {
    let f = fixture().await;
    let route = prepared(&f);
    let result = classify(f.project, &route.input, &route.settings, &f.state)
        .await
        .unwrap();
    result.validate().unwrap();
    assert_eq!(result.difficulty, 1);
    assert_eq!(f.state.store.lock().unwrap().usage().unwrap().len(), 1);
    let history = read_difficulty_history(f.project, &f.state).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].classifier.model, "fixture/local");
    assert_eq!(history[0].quantization_bits, Some(8));
    assert_eq!(history[0].assessment.as_ref().unwrap().difficulty, 1);
    assert!(history[0].valid);
    assert!(history[0].error.is_none());

    let replacement = ModelTarget {
        provider: ModelProvider::Codex,
        profile_id: Some("codex".into()),
        model: "fixture/replacement".into(),
        reasoning: Some("low".into()),
    };
    let mut unreliable = assessment();
    unreliable.confidence = 0.5;
    let repeated = Ok(unreliable);
    record_difficulty_history(f.project, &route.input, &replacement, &repeated, &f.state).unwrap();
    let history = read_difficulty_history(f.project, &f.state).unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].classifier.model, "fixture/local");
    assert_eq!(history[1].classifier.model, "fixture/replacement");
    assert_eq!(history[0].input_sha256, history[1].input_sha256);
    assert_eq!(history[1].quantization_bits, None);
    assert!(!history[1].valid);
    assert!(history[1].error.is_some());
    assert!(f.state.connection.lock().await.is_none());
}
#[tokio::test]
async fn only_the_configured_fallback_is_used_and_never_bypasses_planning() {
    let f = fixture().await;
    let mut route = prepared(&f);
    let actual = route.settings.target(1).unwrap();
    let mut unavailable = actual.clone();
    unavailable.model = "retired/model".into();
    choose_target(&mut route, Some(unavailable.clone()), None, true, &f.state)
        .await
        .unwrap();
    assert!(route.preview.blocked.is_some());
    assert!(route.preview.target.is_none());
    route.settings.fallback = Some(actual.clone());
    choose_target(&mut route, Some(unavailable), None, true, &f.state)
        .await
        .unwrap();
    assert_eq!(route.preview.target, Some(actual.clone()));
    assert!(route.preview.used_fallback);
    route.assessment.as_mut().unwrap().risk = hub_core::RiskLevel::High;
    route.preview.level = Some(1);
    choose_target(&mut route, Some(actual), None, true, &f.state)
        .await
        .unwrap();
    assert!(route.preview.needs_plan);
    assert!(route.preview.target.is_none());
    assert!(f.state.connection.lock().await.is_none());
}
#[tokio::test]
async fn consumed_changed_or_stale_previews_cannot_be_reused() {
    let f = fixture().await;
    let mut route = prepared(&f);
    let target = route.settings.target(1).unwrap();
    choose_target(&mut route, Some(target.clone()), None, true, &f.state)
        .await
        .unwrap();
    let id = route.preview.id.clone();
    f.state
        .auto_routes
        .lock()
        .unwrap()
        .insert(id.clone(), route.clone());
    let mut changed = route.input.clone();
    changed.request = "different task".into();
    assert!(consume_route(&id, 0, f.project, &changed, &f.state)
        .await
        .is_err());
    assert_eq!(
        consume_route(&id, 0, f.project, &route.input, &f.state)
            .await
            .unwrap()
            .target,
        Some(target)
    );
    assert!(consume_route(&id, 0, f.project, &route.input, &f.state)
        .await
        .is_err());
    f.state
        .auto_routes
        .lock()
        .unwrap()
        .insert(id.clone(), route.clone());
    let mut config = route.settings.clone();
    config.confirm_before_run = !config.confirm_before_run;
    f.state
        .store
        .lock()
        .unwrap()
        .set_setting(
            "auto_routing_settings",
            &serde_json::to_string(&config).unwrap(),
        )
        .unwrap();
    assert!(consume_route(&id, 0, f.project, &route.input, &f.state)
        .await
        .unwrap_err()
        .contains("設定が変更"));
    assert_eq!(
        std::fs::read_to_string(f._directory.path().join("greeting.txt")).unwrap(),
        "unchanged"
    );
}

#[test]
fn initial_routes_use_available_routine_and_planner_models() {
    let choice = |name: &str, is_default| models::Choice {
        key: format!("codex:{name}"),
        label: name.into(),
        model: name.into(),
        profile_id: "codex".into(),
        protocol: protocol_types::providers::ProviderProtocol::CodexAppServer,
        local: false,
        reasoning: vec![],
        tools: true,
        default_reasoning: None,
        is_default,
    };
    let mut catalog = models::Catalog {
        models: vec![choice("gpt-6-astra", true), choice("gpt-5.6-luna", false)],
        warnings: vec![],
    };
    let config = initial_settings("local-model".into(), &catalog);
    assert_eq!(config.classifier.model, "local-model");
    assert_eq!(config.target(1).unwrap().provider, ModelProvider::Local);
    assert_eq!(config.target(3).unwrap().model, "gpt-5.6-luna");
    assert_eq!(config.target(5).unwrap().model, "gpt-6-astra");
    assert_eq!(config.fallback.unwrap().model, "gpt-5.6-luna");
    catalog.models = vec![choice("available-default", true)];
    let config = initial_settings("local-model".into(), &catalog);
    assert_eq!(config.target(3).unwrap().model, "available-default");
    assert_eq!(config.target(5).unwrap().model, "available-default");
    catalog.models.clear();
    assert!(initial_settings("local-model".into(), &catalog)
        .fallback
        .is_none());
}

#[test]
fn legacy_auto_settings_gain_planner_and_reviewer_defaults_once() {
    let target = |model: &str| ModelTarget {
        provider: ModelProvider::Codex,
        profile_id: Some("codex".into()),
        model: model.into(),
        reasoning: None,
    };
    let mut config = AutoSettings::initial("fixture/local".into(), Some(target("routine")));
    config.levels[4].target = target("level-five");
    config.planner_default = None;
    config.reviewer_default = None;
    let reviewer = ModelTarget {
        provider: ModelProvider::Codex,
        profile_id: Some("codex".into()),
        model: "saved-astra".into(),
        reasoning: Some("high".into()),
    };
    let (migrated, changed) = normalize_saved_settings(config, Some(reviewer.clone()));
    assert!(changed);
    assert_eq!(
        migrated.planner_default.as_ref().unwrap().model,
        "level-five"
    );
    assert_eq!(migrated.reviewer_default, Some(reviewer));
    let (unchanged, changed) = normalize_saved_settings(migrated.clone(), None);
    assert!(!changed);
    assert_eq!(unchanged, migrated);
}
