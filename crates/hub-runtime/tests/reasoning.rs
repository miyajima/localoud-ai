use hub_db::Store;
use hub_runtime::Sessions;
use provider_codex::CodexProvider;
use std::sync::{Arc, Mutex};
#[tokio::test]
async fn reasoning_survives_a_new_session_manager_and_database_reopen() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    assert!(std::process::Command::new("git")
        .arg("init")
        .arg(dir.path())
        .output()?
        .status
        .success());
    let database = dir.path().join("hub.db");
    let store = Arc::new(Mutex::new(Store::open(&database)?));
    let project = store.lock().unwrap().register_project(dir.path())?;
    let provider = Arc::new(
        CodexProvider::spawn_command(
            "python3",
            &[concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../fixtures/codex-events/fake_server.py"
            )],
        )
        .await?,
    );
    let sessions = Sessions::new(store.clone(), provider.clone());
    let thread = sessions
        .create_with_model(project.id, "Reasoning fixture".into(), Some("fixture-b"))
        .await?;
    store
        .lock()
        .unwrap()
        .set_setting(&format!("thread_reasoning:{}", thread.id), "high")?;
    sessions.start(thread.id, "verify reasoning".into()).await?;
    sessions.interrupt(thread.id).await?;
    drop(sessions);
    drop(store);
    let store = Arc::new(Mutex::new(Store::open(&database)?));
    let resumed = Sessions::new(store, provider.clone());
    resumed.resume(thread.id).await?;
    resumed.start(thread.id, "verify reasoning".into()).await?;
    resumed.interrupt(thread.id).await?;
    provider.shutdown().await?;
    Ok(())
}
