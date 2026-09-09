//! Read-only Astra review of a frozen, supplied benchmark evidence bundle.
use anyhow::{bail, Context, Result};
use provider_codex::CodexProvider;
use serde_json::{json, Value};
use std::{
    io::Write,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Instant,
};

#[tokio::main]
async fn main() -> Result<()> {
    if std::env::var("ASTRA_LIVE_FIXTURE").as_deref() != Ok("1") {
        bail!("explicit live fixture opt-in required");
    }
    let root =
        PathBuf::from(std::env::args().nth(1).context("fixture root required")?).canonicalize()?;
    let lane = std::env::args().nth(2).context("lane required")?;
    if !matches!(lane.as_str(), "native" | "localoud") {
        bail!("unknown benchmark lane");
    }
    let input: Value = serde_json::from_slice(&std::fs::read(
        root.join(format!("{lane}-review-request.json")),
    )?)?;
    let model = input["model"].as_str().context("review model required")?;
    let effort = input["effort"].as_str().context("review effort required")?;
    let prompt = input["prompt"].as_str().context("review prompt required")?;
    let schema = input["schema"].clone();
    let candidate = input["candidate_dir"]
        .as_str()
        .context("neutral review directory required")?;
    if !matches!(candidate, "candidate-a" | "candidate-b") {
        bail!("unknown review directory");
    }
    let out = root.join(format!("{lane}-review"));
    std::fs::create_dir(&out).context("preserve any earlier review attempt")?;
    let file = Arc::new(Mutex::new(
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(out.join("events.jsonl"))?,
    ));
    let sink = Arc::new(move |event: protocol_types::AgentEvent| {
        let mut file = file.lock().unwrap();
        writeln!(file, "{}", serde_json::to_string(&event)?)?;
        file.flush()?;
        Ok(())
    });
    let p = CodexProvider::spawn_with_sink(
        &std::env::var("CODEX_BIN").unwrap_or_else(|_| "codex".into()),
        sink,
        out.join("wire.jsonl"),
    )
    .await?;
    let start = Instant::now();
    let result = p
        .structured_read_only_with_reasoning(
            root.join(candidate).canonicalize()?,
            model,
            Some(effort),
            prompt.to_owned(),
            schema,
        )
        .await;
    let elapsed_ms = start.elapsed().as_millis();
    let record = match &result {
        Ok(review) => {
            json!({"status":"completed","model":model,"effort":effort,"elapsed_ms":elapsed_ms,"review":review})
        }
        Err(error) => {
            json!({"status":"failed","model":model,"effort":effort,"elapsed_ms":elapsed_ms,"error":error.to_string()})
        }
    };
    std::fs::write(out.join("result.json"), serde_json::to_vec_pretty(&record)?)?;
    p.shutdown().await?;
    result?;
    println!(
        "review completed report={}",
        out.join("result.json").display()
    );
    Ok(())
}
