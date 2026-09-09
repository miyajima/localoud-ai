//! One independent Localoud worker for pairing with a real native spawn_agent.
//! Requires a pre-created disposable fixture; never starts the native control.
use anyhow::{bail, Context, Result};
use hub_context::Broker;
use hub_core::Task;
use hub_db::Store;
use protocol_types::{CodingAgentProvider, EventDetails};
use provider_codex::CodexProvider;
use serde_json::json;
use std::{
    io::Write,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[tokio::main]
async fn main() -> Result<()> {
    if std::env::var("ASTRA_LIVE_FIXTURE").as_deref() != Ok("1") {
        bail!("explicit live fixture opt-in required");
    }
    let root =
        PathBuf::from(std::env::args().nth(1).context("fixture root required")?).canonicalize()?;
    let setup: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("setup.json"))?)?;
    let work = root.join("localoud").canonicalize()?;
    if work.join("intervals.py").exists() || root.join("localoud-events.jsonl").exists() {
        bail!("preserve existing attempt; create a new fixture instead");
    }
    let model = setup["model"].as_str().context("model required")?;
    let effort = setup["effort"].as_str().context("effort required")?;
    let prompt = setup["task"].as_str().context("task required")?.to_string();
    let store = Arc::new(Mutex::new(Store::open(&root.join("hub.db"))?));
    let project = store.lock().unwrap().register_project(&work)?;
    let task = Task::new(project.id, "native subagent comparison", &prompt);
    store.lock().unwrap().save_task(&task)?;
    let broker = Arc::new(Broker {
        memory: None,
        store,
        task: task.id,
        project: project.id,
        root: work.clone(),
    });
    let model_config = format!("model=\"{model}\"");
    let effort_config = format!("model_reasoning_effort=\"{effort}\"");
    let p = CodexProvider::spawn_command(
        &std::env::var("CODEX_BIN").unwrap_or_else(|_| "codex".into()),
        &[
            "app-server",
            "--listen",
            "stdio://",
            "-c",
            &model_config,
            "-c",
            &effort_config,
        ],
    )
    .await?;
    p.validate_model_reasoning(model, Some(effort)).await?;
    let thread = p
        .start_worker(work.clone(), vec![Broker::tool_definition()], broker)
        .await?;
    let mut rx = p.events();
    let mut journal = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(root.join("localoud-events.jsonl"))?;
    let start = Instant::now();
    let turn = p
        .start_turn_in_worktree(&thread, work, prompt, model, Some(effort))
        .await?;
    std::fs::write(
        root.join("localoud-start.json"),
        serde_json::to_vec_pretty(
            &json!({"thread_id":thread.id,"turn_id":turn.id,"model":model,"effort":effort}),
        )?,
    )?;
    println!("thread_id={} turn_id={}", thread.id, turn.id);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(600);
    let mut usage = Vec::new();
    loop {
        let e = match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(event) => event?,
            Err(_) => {
                p.interrupt_turn(&turn).await?;
                bail!("attempt timed out; retained evidence");
            }
        };
        if e.thread_id.as_deref() != Some(&thread.id) {
            continue;
        }
        writeln!(journal, "{}", serde_json::to_string(&e)?)?;
        journal.flush()?;
        if let Some(EventDetails::Usage { .. }) = &e.details {
            usage.push(e.details.clone());
        }
        if e.kind == "turn_completed" && e.turn_id.as_deref() == Some(&turn.id) {
            let result = json!({"thread_id":thread.id,"turn_id":turn.id,"status":e.text,"elapsed_ms":start.elapsed().as_millis(),"usage_snapshots":usage});
            std::fs::write(
                root.join("localoud-result.json"),
                serde_json::to_vec_pretty(&result)?,
            )?;
            p.shutdown().await?;
            if e.text != "completed" {
                bail!("worker ended with {}", e.text);
            }
            println!(
                "completed report={}",
                root.join("localoud-result.json").display()
            );
            break;
        }
    }
    Ok(())
}
