use protocol_types::composer::{CollaborationMode, InputMention, TurnOptions};
use provider_codex::{CodexProvider, CodingAgentProvider};
use std::collections::BTreeMap;
#[tokio::test]
async fn native_modes_mentions_goal_and_question_answers_reach_the_wire() -> anyhow::Result<()> {
    let p = CodexProvider::spawn_command(
        "python3",
        &[concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/codex-events/composer_server.py"
        )],
    )
    .await?;
    let root = std::env::current_dir()?;
    let catalog = p.composer_catalog(&root).await?;
    assert!(catalog
        .entries
        .iter()
        .any(|e| e.kind == "skill" && e.name == "fixture-skill"));
    assert!(catalog
        .entries
        .iter()
        .any(|e| e.path == "plugin://fixture-plugin@fixture-market"));
    assert!(catalog.modes.contains(&"plan".into()));
    let thread = p.start_thread_with_model(root, "fixture-model").await?;
    let mentions = vec![
        InputMention::Skill {
            name: "fixture-skill".into(),
            path: "/fixture/SKILL.md".into(),
        },
        InputMention::Plugin {
            name: "fixture-plugin".into(),
            path: "plugin://fixture-plugin@fixture-market".into(),
        },
        InputMention::File {
            name: "readme.md".into(),
            path: "/fixture/readme.md".into(),
        },
        InputMention::Agent {
            name: "explorer".into(),
        },
    ];
    let mut events = p.events();
    p.start_turn_with_options(
        &thread,
        "Plan this change".into(),
        "fixture-model",
        Some("high"),
        TurnOptions {
            mode: Some(CollaborationMode::Plan),
            mentions,
            goal: None,
        },
    )
    .await?;
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if events.recv().await.unwrap().kind == "input_required" {
                break;
            }
        }
    })
    .await?;
    let questions = p.pending_questions(&thread.id).await;
    assert_eq!(questions.len(), 1);
    let answers = BTreeMap::from([("scope".into(), vec!["Small".into()])]);
    assert!(p
        .answer_question("wrong-thread", &questions[0].request_id, answers.clone())
        .await
        .is_err());
    p.answer_question(&thread.id, &questions[0].request_id, answers.clone())
        .await?;
    assert!(p
        .answer_question(&thread.id, &questions[0].request_id, answers)
        .await
        .is_err());
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if events.recv().await.unwrap().kind == "turn_completed" {
                break;
            }
        }
    })
    .await?;
    let turn = p
        .start_turn_with_options(
            &thread,
            "Finish fixture".into(),
            "fixture-model",
            Some("high"),
            TurnOptions {
                mode: Some(CollaborationMode::Default),
                mentions: vec![],
                goal: Some("Finish fixture".into()),
            },
        )
        .await?;
    assert_eq!(p.goal_state(&thread.id).await?["goal"]["status"], "active");
    assert_eq!(p.pause_goal(&thread.id).await?["goal"]["status"], "paused");
    p.interrupt_turn(&turn).await?;
    p.shutdown().await?;
    Ok(())
}
