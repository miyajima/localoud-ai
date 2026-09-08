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
