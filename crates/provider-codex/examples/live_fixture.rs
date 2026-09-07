//! Opt-in live acceptance: operates only on a newly created /tmp Git fixture.
use anyhow::{bail, Context, Result};
use provider_codex::{CodexProvider, CodingAgentProvider};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
#[tokio::main]
async fn main() -> Result<()> {
    if std::env::var("ASTRA_LIVE_FIXTURE").as_deref() != Ok("1") {
        bail!("set ASTRA_LIVE_FIXTURE=1 to authorize this live fixture");
    }
    let root = std::env::temp_dir().join(format!(
        "astra-hub-fixture-{}",
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis()
    ));
    std::fs::create_dir(&root)?;
    std::fs::write(root.join("greeting.txt"), "hello fixture\n")?;
    std::fs::write(root.join("AGENTS.md"),"This is an isolated acceptance fixture. Do not delegate to subagents. Only change greeting.txt as requested. Do not access any other project.\n")?;
    for args in [
        vec!["init"],
        vec!["add", "greeting.txt", "AGENTS.md"],
        vec![
            "-c",
            "user.name=Hub Fixture",
            "-c",
            "user.email=fixture@localhost",
            "commit",
            "-m",
            "Fixture baseline",
        ],
    ] {
        let o = std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(args)
            .output()?;
        if !o.status.success() {
            bail!("fixture Git setup failed");
        }
    }
    println!("fixture_root={}", root.display());
    let bin = std::env::var("CODEX_BIN").unwrap_or_else(|_| "codex".into());
    let p = CodexProvider::spawn(&bin).await?;
    let mut events = p.events();
    let thread = p.start_thread(root.clone()).await?;
    println!("thread_id={}", thread.id);
    let turn=p.start_turn(&thread,"In this fixture repository only, replace the exact content of greeting.txt with hello astra followed by a newline. Do not change other files. Read greeting.txt afterward to verify it. Do not delegate. Then reply briefly with the result.".into()).await?;
    println!("turn_id={}", turn.id);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(240);
    let mut count = 0;
    let outcome = loop {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Ok(event)) => {
                if event.thread_id.as_deref() != Some(thread.id.as_str()) {
                    continue;
                }
                count += 1;
                println!("event={}", event.kind);
                if event.kind == "turn_completed"
                    && event.turn_id.as_deref() == Some(turn.id.as_str())
                {
                    break event.text;
                }
                if event.kind == "approval_required" {
                    let _ = p.interrupt_turn(&turn).await;
                    bail!("fixture needs unsupported approval UI");
                }
            }
            other => {
                let _ = p.interrupt_turn(&turn).await;
                let _ = p.shutdown().await;
                bail!("live fixture stream ended or timed out: {other:?}");
            }
        }
    };
    if outcome != "completed" {
        bail!("turn status {outcome}");
    }
    if std::fs::read_to_string(root.join("greeting.txt"))? != "hello astra\n" {
        bail!("fixture content mismatch");
    }
    let diff = std::process::Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["diff", "--name-only"])
        .output()?;
    if String::from_utf8(diff.stdout)?.trim() != "greeting.txt" {
        bail!("fixture changed unexpected tracked files");
    }
    p.shutdown().await?;
    let restored = CodexProvider::spawn(&bin).await?;
    let snapshot = restored.resume_thread(&thread, root.clone()).await?;
    if snapshot.thread.id != thread.id || snapshot.active_turn.is_some() {
        bail!("restart recovery mismatch");
    }
    snapshot
        .messages
        .iter()
        .find(|m| m.role == "assistant")
        .context("missing recovered assistant message")?;
    restored.shutdown().await?;
    println!("PASS: file mutation verified; {count} events; same thread resumed after process restart; fixture retained at {}",root.display());
    Ok(())
}
