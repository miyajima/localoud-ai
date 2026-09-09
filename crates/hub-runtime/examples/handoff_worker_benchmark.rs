//! Bounded comparison driver through the production Workers path; no native fork.
use anyhow::{bail, Context, Result};
use hub_db::Store;
use hub_events::EventBus;
use hub_runtime::{plan::ExecutionPlan, workers::Workers, Sessions};
use hub_scheduler::TaskRunner;
use provider_codex::CodexProvider;
use serde_json::json;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Instant,
};

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 3 || !matches!(args[1].as_str(), "preflight" | "run") {
        bail!("usage: handoff_worker_benchmark <preflight|run> <prepared-root>");
    }
    let root = PathBuf::from(&args[2]).canonicalize()?;
    let evidence = root.join("localoud/evidence");
    let binary = std::env::var("CODEX_BIN").unwrap_or_else(|_| "codex".into());
    let plan: ExecutionPlan = serde_json::from_slice(&std::fs::read(root.join("plan.json"))?)?;
    if plan.steps.len() != 1 {
        bail!("calibration requires exactly one worker");
    }
    let execution = plan.steps[0]
        .brief
        .execution
        .as_ref()
        .context("explicit execution model required")?;
    if args[1] == "preflight" {
        let provider = CodexProvider::spawn(&binary).await?;
        let result = provider
            .validate_model_reasoning(&execution.model, Some(&execution.reasoning))
            .await;
        let body = json!({"model":execution.model,"reasoning":execution.reasoning,"available":result.is_ok(),"error":result.as_ref().err().map(|e|e.to_string()),"model_calls":0});
        std::fs::write(
            evidence.join("preflight.json"),
            serde_json::to_vec_pretty(&body)?,
        )?;
        provider.shutdown().await?;
        println!("{}", body);
        return result;
    }
    if std::env::var("ASTRA_LIVE_FIXTURE").as_deref() != Ok("1") {
        bail!("explicit live fixture opt-in required");
    }
    // The marker is permanent even on failure; this driver never retries a run.
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(evidence.join("attempt-started"))?;
    let started = Instant::now();
    let work = root.join("localoud/fixture").canonicalize()?;
    let sha = String::from_utf8(
        std::process::Command::new("git")
            .arg("-C")
            .arg(&work)
            .args(["rev-parse", "HEAD"])
            .output()?
            .stdout,
    )?
    .trim()
    .to_owned();
    let store = Arc::new(Mutex::new(Store::open(&evidence.join("hub.db"))?));
    let project = store.lock().unwrap().register_project(&work)?;
    let (dag, briefs) = plan.compile(project.id)?;
    let task = dag.tasks.values().next().context("missing task")?.clone();
    store.lock().unwrap().save_plan(
        task.plan_id.context("missing plan ID")?,
        project.id,
        &serde_json::to_string(&plan)?,
    )?;
    store.lock().unwrap().save_task(&task)?;
    let bus = EventBus::new(store.clone());
    let sink = bus.clone();
    let provider = Arc::new(
        CodexProvider::spawn_with_sink(
            &binary,
            Arc::new(move |event| sink.publish(event)),
            evidence.join("provider.jsonl"),
        )
        .await?,
    );
    let workers = Workers {
        store: store.clone(),
        sessions: Arc::new(Sessions::new(store.clone(), provider.clone())),
        provider: provider.clone(),
        local: None,
        memory: None,
        bus,
        worktrees: hub_worktree::GitWorktrees::new(&work)?,
        base_sha: sha,
        briefs,
        outcomes: Mutex::new(HashMap::new()),
    };
    let result = workers.run(task.clone()).await;
    let outcome = workers.outcomes.lock().unwrap().get(&task.id).cloned();
    let inspection = store.lock().unwrap().capsule(task.id)?;
    let body = json!({"task_id":task.id,"model":execution.model,"reasoning":execution.reasoning,
        "status":if result.is_ok(){"completed"}else{"failed"},"error":result.as_ref().err().map(|e|format!("{e:#}")),
        "elapsed_ms":started.elapsed().as_millis(),"outcome":outcome,"capsule":inspection});
    std::fs::write(
        evidence.join("result.json"),
        serde_json::to_vec_pretty(&body)?,
    )?;
    provider.shutdown().await?;
    println!(
        "status={} result={}",
        body["status"],
        evidence.join("result.json").display()
    );
    result
}
