use provider_codex::{CodexProvider, CodingAgentProvider};
#[tokio::test]
async fn requested_effort_is_validated_and_sent_on_every_turn() -> anyhow::Result<()> {
    let provider = CodexProvider::spawn_command(
        "python3",
        &[concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/codex-events/fake_server.py"
        )],
    )
    .await?;
    assert!(provider
        .validate_model_reasoning("fixture-a", Some("high"))
        .await
        .is_err());
    assert!(provider
        .validate_model_reasoning("fixture-b", Some("ultra"))
        .await
        .is_err());
    let root = std::env::current_dir()?;
    let thread = provider
        .start_thread_with_model(root.clone(), "fixture-b")
        .await?;
    let turn = provider
        .start_turn_with_reasoning(
            &thread,
            "verify reasoning".into(),
            "fixture-b",
            Some("high"),
        )
        .await?;
    provider.interrupt_turn(&turn).await?;
    provider
        .resume_thread_with_model(&thread, root, "fixture-b")
        .await?;
    let turn = provider
        .start_turn_with_reasoning(
            &thread,
            "verify reasoning".into(),
            "fixture-b",
            Some("high"),
        )
        .await?;
    provider.interrupt_turn(&turn).await?;
    provider.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn resumed_worker_turn_explicitly_rebinds_cwd_and_write_scope() -> anyhow::Result<()> {
    let provider = CodexProvider::spawn_command(
        "python3",
        &[concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/codex-events/fake_server.py"
        )],
    )
    .await?;
    let old = tempfile::tempdir()?;
    let new = tempfile::tempdir()?;
    let thread = provider
        .start_thread_with_model(old.path().to_owned(), "fixture-b")
        .await?;
    provider
        .resume_thread_with_model(&thread, new.path().to_owned(), "fixture-b")
        .await?;
    let root = new.path().canonicalize()?;
    let turn = provider
        .start_turn_in_worktree(
            &thread,
            root.clone(),
            format!("verify isolated cwd:{}", root.display()),
            "fixture-b",
            Some("high"),
        )
        .await?;
    provider.interrupt_turn(&turn).await?;
    provider.shutdown().await?;
    Ok(())
}
