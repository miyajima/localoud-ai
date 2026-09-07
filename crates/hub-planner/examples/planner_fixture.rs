use anyhow::{bail, Result};
use hub_core::*;
use hub_planner::*;
use provider_codex::CodexProvider;
use std::sync::Arc;
#[tokio::main]
async fn main() -> Result<()> {
    if std::env::var("ASTRA_LIVE_FIXTURE").as_deref() != Ok("1") {
        bail!("opt-in fixture required");
    }
    let d = tempfile::Builder::new()
        .prefix("astra-planner-fixture-")
        .tempdir()?
        .keep();
    std::fs::write(d.join("AGENTS.md"),"This fixture is for read-only planning and review. Do not execute commands, edit files, or delegate.\n")?;
    if !std::process::Command::new("git")
        .arg("init")
        .arg(&d)
        .output()?
        .status
        .success()
    {
        bail!("git init failed");
    }
    let p = Arc::new(
        CodexProvider::spawn(&std::env::var("CODEX_BIN").unwrap_or_else(|_| "codex".into()))
            .await?,
    );
    let astra = Astra {
        mode: AstraAccessMode::CodexIntegrated,
        provider: p.clone(),
        root: d.clone(),
        model: "gpt-6-astra".into(),
    };
    println!("fixture_root={}", d.display());
    let plan=astra.create_plan(PlanningRequest{goal:"Plan a tiny Python standard-library text statistics utility. words.py counts whitespace-separated words, lines.py counts splitlines, stats.py integrates both. Each module has unittest coverage. Two independent module workers and one integration worker are appropriate. Planning only.".into(),constraints:vec!["No network, no merges, only fixture files".into()],evidence:vec!["Repository is initially empty except AGENTS.md. Python standard library only.".into()]}).await?;
    plan.compile(ProjectId::default())?;
    std::fs::write(d.join("plan.json"), serde_json::to_string_pretty(&plan)?)?;
    println!("plan_steps={}", plan.steps.len());
    let review = astra
        .review_result(FinalReviewRequest {
            task_id: TaskId::default(),
            goal: "Return sum of two values".into(),
            acceptance_criteria: vec!["add(2,3)==5".into()],
            diff: "+def add(a,b):\n+    return a-b\n".into(),
            evidence: vec!["Fixed acceptance failed: expected 5, got -1".into()],
        })
        .await?;
    if !matches!(review.verdict, Verdict::Rework) {
        bail!("known defect was not rejected");
    }
    println!("review={}", serde_json::to_string(&review)?);
    let recovery = astra
        .diagnose_failure(FailureDiagnosisRequest {
            task_id: TaskId::default(),
            goal: "Return sum of two values".into(),
            attempts: 2,
            failures: vec![
                "Attempt 1 subtracts instead of adding".into(),
                "Attempt 2 still subtracts; fixed test fails".into(),
            ],
            architecture_question: None,
        })
        .await?;
    println!("recovery={}", serde_json::to_string(&recovery)?);
    let disabled = Astra {
        mode: AstraAccessMode::Disabled,
        provider: p.clone(),
        root: d,
        model: "gpt-6-astra".into(),
    };
    assert!(disabled
        .create_plan(PlanningRequest {
            goal: "manual fallback".into(),
            constraints: vec![],
            evidence: vec![]
        })
        .await
        .is_err());
    p.shutdown().await?;
    println!(
        "PASS structured plan, defect review, repeated-failure diagnosis, disabled-mode boundary"
    );
    Ok(())
}
