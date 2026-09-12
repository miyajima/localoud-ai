use hub_core::*;
use hub_db::Store;
use hub_events::EventBus;
use hub_runtime::{plan::ExecutionPlan, workers::Workers, Sessions};
use hub_scheduler::TaskRunner;
use provider_codex::CodexProvider;
use std::sync::{Arc, Mutex};

fn plan() -> ExecutionPlan {
    let sections = [
        "purpose",
        "constraints",
        "decisions",
        "current_state",
        "unresolved",
        "completion",
    ];
    let sources: Vec<_> = sections.iter().enumerate().map(|(i, s)| serde_json::json!({
        "id":s,"role":"user","reference":format!("turn:{i}"),"text":format!("{s}: temporary task context; keep src/a.b.py"),"call_id":null
    })).collect();
    let groups: Vec<_> = sections.iter().map(|s| serde_json::json!({"id":s,"section":s,"source_ids":[s],"depends_on":[],"corrects":[]})).collect();
    serde_json::from_value(serde_json::json!({"title":"handoff fixture", "steps":[{
        "key":"one","title":"one","goal":"Current task: inspect only", "dependencies":[],
        "brief":{"execution":{"model":"fixture-b","reasoning":"high"},"acceptance_criteria":["inspection complete"],"constraints":["No external actions"],"context_items":[],
            "budget":{"initial_tokens":8000,"max_total_tokens":10000,"max_single_retrieval_tokens":500},
            "handoff":{"sources":sources,"groups":groups,"outcomes":[],"max_bytes":12000}}
    }]})).unwrap()
}

#[test]
fn plan_rejects_full_capsule_overflow_and_accepts_legacy_json() {
    let mut p = plan();
    p.steps[0].brief.budget.initial_tokens = 100;
    assert!(p
        .compile(ProjectId::default())
        .err()
        .unwrap()
        .to_string()
        .contains("initial capsule exceeds"));
    p.steps[0].brief.handoff = None;
    let serialized = serde_json::to_string(&p).unwrap();
    assert!(!serialized.contains("\"handoff\""));
    let _: ExecutionPlan = serde_json::from_str(&serialized).unwrap();
}

#[tokio::test]
async fn actual_worker_receives_persisted_handoff_through_plan_dispatch() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    std::fs::write(root.path().join("README.md"), "frozen fixture\n")?;
    for args in [
        vec!["init"],
        vec!["add", "README.md"],
        vec![
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@localhost",
            "commit",
            "-m",
            "baseline",
        ],
    ] {
        assert!(std::process::Command::new("git")
            .arg("-C")
            .arg(root.path())
            .args(args)
            .output()?
            .status
            .success());
    }
    let sha = String::from_utf8(
        std::process::Command::new("git")
            .arg("-C")
            .arg(root.path())
            .args(["rev-parse", "HEAD"])
            .output()?
            .stdout,
    )?
    .trim()
    .to_owned();
    let data = tempfile::tempdir()?;
    let capture = data.path().join("prompt.json");
    let store = Arc::new(Mutex::new(Store::open(&data.path().join("hub.db"))?));
    let project = store.lock().unwrap().register_project(root.path())?;
    let (dag, briefs) = plan().compile(project.id)?;
    let task = dag.tasks.values().next().unwrap().clone();
    store.lock().unwrap().save_plan(
        task.plan_id.unwrap(),
        project.id,
        &serde_json::to_string(&plan())?,
    )?;
    store.lock().unwrap().save_task(&task)?;
    let provider = Arc::new(
        CodexProvider::spawn_command(
            "python3",
            &[
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../fixtures/codex-events/handoff_server.py"
                ),
                capture.to_str().unwrap(),
            ],
        )
        .await?,
    );
    let workers = Workers {
        store: store.clone(),
        sessions: Arc::new(Sessions::new(store.clone(), provider.clone())),
        provider: provider.clone(),
        local: None,
        memory: None,
        bus: EventBus::new(store.clone()),
        worktrees: hub_worktree::GitWorktrees::new(root.path())?,
        base_sha: sha,
        briefs,
        outcomes: Mutex::new(Default::default()),
    };
    workers.run(task.clone()).await?;
    let request: serde_json::Value = serde_json::from_slice(&std::fs::read(&capture)?)?;
    assert_eq!(request["model"], "fixture-b");
    assert_eq!(request["effort"], "high");
    assert_eq!(request["fixture_thread_start"]["model"], "fixture-b");
    assert_eq!(
        request["fixture_thread_start"]["dynamicTools"][0]["name"],
        "hub_context"
    );
    let prompt = request["input"][0]["text"].as_str().unwrap();
    let capsule = store.lock().unwrap().capsule(task.id)?.unwrap().0;
    let envelope = prompt
        .split_once("<a2a-message protocol-version=\"1.0\">\n")
        .and_then(|(_, value)| value.split_once("\n</a2a-message>"))
        .map(|(value, _)| value)
        .expect("worker prompt must contain one A2A v1 envelope");
    let message: protocol_types::a2a::Message = serde_json::from_str(envelope)?;
    message.validate()?;
    let expected_task_id = task.id.to_string();
    assert_eq!(message.task_id.as_deref(), Some(expected_task_id.as_str()));
    assert_eq!(
        message.parts[0].media_type.as_deref(),
        Some(protocol_types::a2a::CONTEXT_CAPSULE_MEDIA_TYPE)
    );
    assert_eq!(message.parts[0].data, Some(serde_json::to_value(&capsule)?));
    assert!(prompt.contains("not authorization"));
    let selection: hub_context::handoff::Selection = serde_json::from_str(&capsule.items[0].text)?;
    assert_eq!(selection.sources.len(), 6);
    assert_eq!(selection.sources[3].reference, "turn:3");
    assert_eq!(workers.outcomes.lock().unwrap().len(), 1);
    let thread = workers.outcomes.lock().unwrap()[&task.id].thread_id;
    assert_eq!(
        store
            .lock()
            .unwrap()
            .setting(&format!("thread_model:{thread}"))?
            .as_deref(),
        Some("fixture-b")
    );
    assert_eq!(
        store
            .lock()
            .unwrap()
            .setting(&format!("thread_reasoning:{thread}"))?
            .as_deref(),
        Some("high")
    );
    let mut bad_briefs = workers.briefs.clone();
    bad_briefs
        .get_mut(&task.id)
        .unwrap()
        .execution
        .as_mut()
        .unwrap()
        .reasoning = "low".into();
    let incompatible = Workers {
        briefs: bad_briefs,
        ..workers
    };
    assert!(incompatible
        .run(task)
        .await
        .unwrap_err()
        .to_string()
        .contains("refusing to resume"));

    provider.shutdown().await?;
    Ok(())
}
