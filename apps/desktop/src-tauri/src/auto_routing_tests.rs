use super::*;
use hub_db::Store;
use hub_events::EventBus;
use hub_runtime::local_worker::LocalExecutor;
use provider_spark::{LocalServiceConfig, SparkProvider};
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
    };
    let bus = EventBus::new(store.clone());
    let local = LocalExecutor {
        store: store.clone(),
        bus: bus.clone(),
        provider: Arc::new(SparkProvider::configured(local_config.clone()).unwrap()),
        lock: tokio::sync::Mutex::new(()),
    };
    let browser = crate::chatgpt::Browser::new(store.clone(), bus.clone()).unwrap();
    let state = AppState {
        workflow_lock: tokio::sync::Mutex::new(()),
        browser,
        local_stops: tokio::sync::Mutex::new(HashMap::new()),
        store,
        bus,
        connection: tokio::sync::Mutex::new(None),
        data_dir: directory.path().to_path_buf(),
        local,
        local_config,
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
