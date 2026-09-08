//! Complete unresolved memory/review gates on an already executed fixture, without re-running workers.
use anyhow::{bail, Context, Result};
use hub_core::*;
use hub_memory::{OrgBrain, OrgBrainConfig};
use hub_planner::*;
use hub_runtime::workers::WorkerOutcome;
use provider_codex::CodexProvider;
use provider_spark::SparkProvider;
use serde_json::{json, Value};
use std::{
    io::BufRead,
    sync::{Arc, Mutex},
};
struct Server(std::process::Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[allow(dead_code)]
#[tokio::main]
async fn main() -> Result<()> {
    if std::env::var("ASTRA_LIVE_FIXTURE").as_deref() != Ok("1") {
        bail!("opt-in fixture required");
    }
    let root = std::path::PathBuf::from(std::env::var("ASTRA_REUSE_FIXTURE")?).canonicalize()?;
    finish(root).await
}
pub async fn finish(root: std::path::PathBuf) -> Result<()> {
    if !root.starts_with(std::env::temp_dir().canonicalize()?)
        || !root
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("astra-full-fixture-")
    {
        bail!("invalid retained fixture");
    }
    let store = Arc::new(Mutex::new(hub_db::Store::open(&root.join("hub.db"))?));
    let project = store
        .lock()
        .unwrap()
        .projects()?
        .into_iter()
        .next()
        .context("project missing")?;
    let tasks = store.lock().unwrap().tasks(project.id)?;
    if tasks.len() != 3 || tasks.iter().any(|t| t.status != TaskStatus::Completed) {
        bail!("requires 3 completed workers");
    }
    let mut child = std::process::Command::new("python3")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/memory/server.py"
        ))
        .arg(root.join("orgbrain-fixture.json"))
        .stdout(std::process::Stdio::piped())
        .spawn()?;
    let mut port = String::new();
    std::io::BufReader::new(child.stdout.take().unwrap()).read_line(&mut port)?;
    let _server = Server(child);
    let memory = OrgBrain::new(
        OrgBrainConfig {
            endpoint: format!("http://127.0.0.1:{}/mcp", port.trim()),
            tenant: "fixture".into(),
            remote_project_id: "fixture-project".into(),
            auto_store: true,
        },
        "fixture-id".into(),
        "fixture-secret".into(),
    )?;
    let local = SparkProvider::new("http://127.0.0.1:8765")?;
    let mut errors = vec![];
    let task = tasks
        .iter()
        .find(|t| t.dependencies.len() == 2)
        .context("integration task missing")?;
    let outcome: WorkerOutcome = serde_json::from_str(
        &store
            .lock()
            .unwrap()
            .setting(&format!("outcome:{}", task.id))?
            .context("outcome missing")?,
    )?;
    let current_diff = hub_worktree::GitWorktrees::new(&root)?
        .diff(&outcome.worktree)
        .await?;
    if current_diff != outcome.diff {
        bail!("held fixture diff changed");
    }
    let evidence_file = root.join("final-evidence.json");
    let evidence: Vec<String> = if evidence_file.exists() {
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&evidence_file)?)?;
        if v["diff"] != current_diff || v.get("goal") != Some(&json!(task.description)) {
            bail!("evidence diff changed");
        }
        serde_json::from_value(v["evidence"].clone())?
    } else {
        let mut logs = vec![];
        for args in [
            vec!["acceptance.py"],
            vec!["-m", "unittest", "discover", "-v"],
        ] {
            let o = std::process::Command::new("python3")
                .args(&args)
                .current_dir(&outcome.worktree.path)
                .output()?;
            if !o.status.success() {
                bail!(
                    "fixed or worker tests failed: {}",
                    String::from_utf8_lossy(&o.stderr)
                );
            }
            logs.push(format!(
                "Independent python3 {} returned exit 0:\n{}",
                args.join(" "),
                String::from_utf8_lossy(&o.stderr)
            ));
        }
        let git = hub_worktree::GitWorktrees::new(&root)?;
        let original = std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args([
                "show",
                &format!("{}:acceptance.py", outcome.worktree.base_sha),
            ])
            .output()?;
        if !original.status.success()
            || original.stdout != std::fs::read(outcome.worktree.path.join("acceptance.py"))?
        {
            bail!("acceptance fixture was modified");
        }
        logs.push(format!(
            "Git provenance: {}",
            git.review_provenance(&outcome.worktree).await?
        ));
        logs.push(format!(
            "Hub control receipts: {:?}",
            store.lock().unwrap().task_control_evidence(task.id)?
        ));
        logs.extend(store.lock().unwrap().review_evidence(outcome.thread_id)?);
        logs.push(format!(
            "Hub retrieval ledger: {}",
            serde_json::to_string(&store.lock().unwrap().retrievals(task.id)?)?
        ));
        std::fs::write(
            &evidence_file,
            serde_json::to_string_pretty(
                &json!({"goal":task.description,"diff":current_diff,"evidence":logs}),
            )?,
        )?;
        logs
    };
    let review_file = root.join("final-review-scoped.json");
    let review: FinalReview = if review_file.exists() {
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&review_file)?)?;
        if v["diff"] != current_diff || v.get("goal") != Some(&json!(task.description)) {
            bail!("review diff changed");
        }
        serde_json::from_value(v["review"].clone())?
    } else {
        let p = Arc::new(
            CodexProvider::spawn(&std::env::var("CODEX_BIN").unwrap_or_else(|_| "codex".into()))
                .await?,
        );
        let astra = Astra {
            reasoning: None,
            mode: AstraAccessMode::CodexIntegrated,
            provider: p.clone(),
            root: root.clone(),
            model: "gpt-6-astra".into(),
        };
        let review = astra
            .review_result(FinalReviewRequest {
                task_id: task.id,
                goal: task.description.clone(),
                acceptance_criteria: vec![
                    "8 fixed acceptance tests and all worker tests pass".into()
                ],
                diff: current_diff.clone(),
                evidence: evidence.clone(),
            })
            .await?;
        p.shutdown().await?;
        std::fs::write(
            &review_file,
            serde_json::to_string_pretty(
                &json!({"goal":task.description,"diff":current_diff,"review":review}),
            )?,
        )?;
        review
    };
    if matches!(review.verdict, Verdict::Approve) {
        let scope = hub_policy::scope_digest(&task.description, &current_diff);
        {
            let s = store.lock().unwrap();
            s.set_setting(
                &format!("review:{}", task.id),
                &serde_json::to_string(&review)?,
            )?;
            s.set_setting(&format!("review_scope:{}", task.id), &scope)?;
        }
        match hub_runtime::memory_capture::capture(&store, &local, Some(&memory), task, &outcome)
            .await
        {
            Ok(items) => println!("reviewed completion durable_items={}", items.len()),
            Err(e) => errors.push(format!("{}: {e:#}", task.title)),
        }
    }
    let retrievals: usize = tasks
        .iter()
        .map(|t| {
            store
                .lock()
                .unwrap()
                .retrievals(t.id)
                .unwrap()
                .iter()
                .filter(|r| r.source == "org_brain" && r.token_estimate > 0)
                .count()
        })
        .sum();
    let report = json!({"root":root,"tasks":tasks,"orgbrain_retrievals":retrievals,"review":review,"test_evidence":evidence,"memory_errors":errors,"usage":store.lock().unwrap().usage()?,"orgbrain_transport":"loopback file-backed MCP fixture, not a real account","prior_attempts":["Initial query fixture did not match extracted policy; corrected to text_stats before workers ran","Worker tests absent from first final-review input; independent test output supplied","429/422 extraction failures diagnosed; compact completion facts and serialized local calls applied", "Same-parent quality review rejected misread command-count memories; fixture-only records removed, unsupported failure claims blocked, and persistence now requires approval of exact goal/diff"]});
    std::fs::write(
        root.join("full-report.json"),
        serde_json::to_string_pretty(&report)?,
    )?;
    if !errors.is_empty() || !matches!(review.verdict, Verdict::Approve) || retrievals < 3 {
        bail!("one or more retained fixture gates remain open; see full-report.json");
    }
    println!(
        "PASS full-report={}",
        root.join("full-report.json").display()
    );
    Ok(())
}
