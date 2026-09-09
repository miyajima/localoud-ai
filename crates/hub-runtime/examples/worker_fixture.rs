use anyhow::{bail, Result};
use hub_core::*;
use hub_db::Store;
use hub_events::EventBus;
use hub_runtime::{
    workers::{WorkerBrief, Workers},
    Sessions,
};
use hub_scheduler::{execute, Dag};
use hub_worktree::GitWorktrees;
use provider_codex::CodexProvider;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
#[tokio::main]
async fn main() -> Result<()> {
    if std::env::var("ASTRA_LIVE_FIXTURE").as_deref() != Ok("1") {
        bail!("set ASTRA_LIVE_FIXTURE=1 for the opt-in live fixture");
    }
    let root = tempfile::Builder::new()
        .prefix("astra-workers-fixture-")
        .tempdir()?
        .keep();
    std::fs::write(
        root.join("README.md"),
        "Fixture project. Standard-library Python only. No network or subagents.\n",
    )?;
    std::fs::write(
        root.join(".gitignore"),
        "__pycache__/\n*.db*\n*.jsonl\n.agent-worktrees/\n",
    )?;
    std::fs::write(root.join("AGENTS.md"),"Only work within this fixture worktree. No subagents. Use unittest to verify changes. Do not access other projects or merge branches.\n")?;
    for args in [
        vec!["init"],
        vec!["add", "README.md", ".gitignore", "AGENTS.md"],
        vec![
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@localhost",
            "commit",
            "-m",
            "baseline",
        ],
    ] {
        if !std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(args)
            .output()?
            .status
            .success()
        {
            bail!("fixture Git setup failed");
        }
    }
    let sha = String::from_utf8(
        std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["rev-parse", "HEAD"])
            .output()?
            .stdout,
    )?
    .trim()
    .to_owned();
    let store = Arc::new(Mutex::new(Store::open(&root.join("hub.db"))?));
    let project = store.lock().unwrap().register_project(&root)?;
    store.lock().unwrap().store_decision(project.id,"slug-policy","slug punctuation policy","Convert ASCII letters to lowercase. Replace each run of characters outside ASCII a-z and 0-9 with one hyphen. Strip leading and trailing hyphens. Empty input returns empty string.")?;
    let bus = EventBus::new(store.clone());
    let b = bus.clone();
    let provider = Arc::new(
        CodexProvider::spawn_with_sink(
            &std::env::var("CODEX_BIN").unwrap_or_else(|_| "codex".into()),
            Arc::new(move |e| b.publish(e)),
            root.join("provider.jsonl"),
        )
        .await?,
    );
    let sessions = Arc::new(Sessions::new(store.clone(), provider.clone()));
    let a=Task::new(project.id,"Slug worker","First use hub_context with source decision_store and query slug to retrieve the exact slug policy. Then create strings.py with slugify(text) following that retrieved policy and test_strings.py using unittest. Include punctuation, repeated separators, uppercase, and empty input cases. Run the tests. Do not edit other files.");
    let b=Task::new(project.id,"Clamp worker","First use hub_context with source repo, path README.md, query fixture, token_budget 200 to read project context. Create numbers_util.py with clamp(value, low, high): bound value to the inclusive range, raise ValueError when low > high. Add test_numbers.py with unittest covering below/inside/above/equal bounds and invalid bounds. Run tests. Only edit those two files.");
    let mut c=Task::new(project.id,"Integration worker","Dependencies supply strings.py and numbers_util.py plus tests. Create integration.py with format_score(name, score) returning slugify(name) + ':' + str(clamp(score,0,100)). Add test_integration.py including format_score('Hello, World!',120) == 'hello-world:100'. Run python3 -m unittest discover -v. Do not modify dependency files.");
    c.dependencies = vec![a.id, b.id];
    let tasks = vec![a, b, c];
    {
        let s = store.lock().unwrap();
        for t in &tasks {
            s.save_task(t)?;
        }
        for t in &tasks {
            s.save_dependencies(t)?;
        }
    }
    let briefs=tasks.iter().map(|t|(t.id,WorkerBrief{execution:None,handoff:None,acceptance_criteria:vec!["Implement the requested behavior and run unittest successfully".into()],constraints:vec!["No parent conversation is available. Use hub_context for missing evidence.".into()],context_items:vec![],budget:ContextBudget{initial_tokens:2000,max_total_tokens:4000,max_single_retrieval_tokens:600}})).collect();
    let workers = Arc::new(Workers {
        memory: None,
        store: store.clone(),
        sessions,
        provider: provider.clone(),
        local: None,
        bus,
        worktrees: GitWorktrees::new(&root)?,
        base_sha: sha,
        briefs,
        outcomes: Mutex::new(HashMap::new()),
    });
    println!("fixture_root={}", root.display());
    let dag = execute(Dag::new(tasks)?, workers.clone(), 2).await?;
    for task in dag.tasks.values() {
        println!("task={} status={:?}", task.title, task.status);
    }
    for result in workers.outcomes.lock().unwrap().values() {
        println!("outcome={}", serde_json::to_string(result)?);
    }
    println!(
        "usage={}",
        serde_json::to_string(&store.lock().unwrap().usage()?)?
    );
    provider.shutdown().await?;
    if !dag
        .tasks
        .values()
        .all(|t| t.status == TaskStatus::Completed)
    {
        bail!("worker DAG did not complete; inspect retained fixture");
    }
    if root.join("strings.py").exists() || root.join("numbers_util.py").exists() {
        bail!("workers changed parent workspace");
    }
    let s = store.lock().unwrap();
    let total_retrievals = dag
        .tasks
        .keys()
        .map(|id| s.retrievals(*id).map(|r| r.len()))
        .collect::<Result<Vec<_>>>()?
        .iter()
        .sum::<usize>();
    if total_retrievals < 2 {
        bail!("worker pull retrieval was not demonstrated");
    }
    println!("PASS: independent threads/worktrees, dependency integration, {total_retrievals} logged context retrievals; inspect/run tests in the integration worktree for independent acceptance");
    Ok(())
}
