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
    assert!(p.archive_thread(&t).await.is_err());
    p.interrupt_turn(&turn).await?;
    p.archive_thread(&t).await?;
    assert!(p.read_thread(&t).await?.active_turn.is_none());
    assert!(p.resume_thread(&t, root.clone()).await.is_err());
    p.unarchive_thread(&t).await?;
    assert!(p.resume_thread(&t, root).await?.active_turn.is_none());
    p.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn selected_model_is_sent_and_resumed_without_default_substitution() -> anyhow::Result<()> {
    let p = CodexProvider::spawn_command(
        "python3",
        &[concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/codex-events/fake_server.py"
        )],
    )
    .await?;
    let root = std::env::current_dir()?;
    assert!(p
        .start_thread_with_model(root.clone(), "unavailable")
        .await
        .is_err());
    let thread = p.start_thread_with_model(root.clone(), "fixture-b").await?;
    p.resume_thread_with_model(&thread, root, "fixture-b")
        .await?;
    p.shutdown().await?;
    Ok(())
}
