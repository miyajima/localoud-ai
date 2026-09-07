use provider_codex::{CodexProvider, CodingAgentProvider};
#[tokio::test]
async fn lifecycle_stream_steer_interrupt_resume() -> anyhow::Result<()> {
    let script = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/codex-events/fake_server.py"
    );
    let p = CodexProvider::spawn_command("python3", &[script]).await?;
    let mut events = p.events();
    let root = std::env::current_dir()?;
    let t = p.start_thread(root.clone()).await?;
    let turn = p.start_turn(&t, "fixture".into()).await?;
    let event = tokio::time::timeout(std::time::Duration::from_secs(3), events.recv()).await??;
    assert_eq!(event.text, "fixture output");
    p.steer_turn(&turn, "stay in scope".into()).await?;
    assert_eq!(p.read_thread(&t).await?.active_turn.unwrap().id, turn.id);
    p.interrupt_turn(&turn).await?;
    assert!(p.resume_thread(&t, root).await?.active_turn.is_none());
    p.shutdown().await?;
    Ok(())
}
