use anyhow::{bail, Result};
use hub_db::Store;
use hub_events::EventBus;
use hub_runtime::local_worker::LocalExecutor;
use protocol_types::local::{LocalModelProvider, RoutingInput};
use provider_spark::SparkProvider;
use std::sync::{Arc, Mutex};
#[tokio::main]
async fn main() -> Result<()> {
    let provider = Arc::new(SparkProvider::new("http://127.0.0.1:8765")?);
    println!("health={}", provider.health().await?);
    let dir = tempfile::tempdir()?;
    let output = std::process::Command::new("git")
        .arg("init")
        .arg(dir.path())
        .output()?;
    if !output.status.success() {
        bail!("fixture init failed");
    }
    std::fs::write(dir.path().join("greeting.txt"), "helllo world\n")?;
    let store = Arc::new(Mutex::new(Store::open(&dir.path().join("hub.db"))?));
    let project = store.lock().unwrap().register_project(dir.path())?;
    let executor = LocalExecutor {
        store: store.clone(),
        bus: EventBus::new(store.clone()),
        provider: provider.clone(),
        lock: tokio::sync::Mutex::new(()),
    };
    let input = RoutingInput {
        request: "Fix the typo helllo to hello in greeting.txt. Preserve everything else.".into(),
        known_files: vec!["greeting.txt".into()],
        estimated_loc: Some(1),
    };
    let route = provider.classify_task(&input).await?;
    executor.record_usage(&route.usage, None)?;
    println!("route={}", serde_json::to_string(&route)?);
    let thread = executor.create(project.id, "Local typo fixture".into())?;
    let result = executor
        .run(thread.id, input.request.clone(), input.known_files)
        .await?;
    if std::fs::read_to_string(dir.path().join("greeting.txt"))? != "hello world\n" {
        bail!("local task acceptance failed");
    }
    let summary = provider
        .summarize_progress(&[
            "Changed greeting.txt: helllo -> hello. Exact bytes verified.".into(),
        ])
        .await?;
    executor.record_usage(&summary.usage, None)?;
    println!("result={}", serde_json::to_string(&result)?);
    println!("summary={}", serde_json::to_string(&summary)?);
    println!(
        "usage_records={}",
        serde_json::to_string(&store.lock().unwrap().usage()?)?
    );
    println!("PASS: real Spark 8-bit route, edit, first-pass review, and progress summary; no Codex invocation");
    Ok(())
}
