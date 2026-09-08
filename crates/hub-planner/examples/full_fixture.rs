#[path = "finish_fixture.rs"]
mod finishing;
use anyhow::{bail, Context, Result};
use hub_core::*;
use hub_memory::{DurableMemoryProvider, OrgBrain, OrgBrainConfig};
use hub_planner::*;
use hub_runtime::{workers::Workers, Sessions};
use protocol_types::local::LocalModelProvider;
use provider_codex::CodexProvider;
use provider_spark::SparkProvider;
use std::{
    collections::HashMap,
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
#[tokio::main]
async fn main() -> Result<()> {
    if std::env::var("ASTRA_LIVE_FIXTURE").as_deref() != Ok("1") {
        bail!("opt-in fixture required");
    }
    let root = if let Ok(path) = std::env::var("ASTRA_REUSE_FIXTURE") {
        let p = std::path::PathBuf::from(path).canonicalize()?;
        if !p.starts_with(std::env::temp_dir().canonicalize()?)
            || !p
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("astra-full-fixture-")
        {
            bail!("invalid fixture reuse root");
        }
        p
    } else {
        tempfile::Builder::new()
            .prefix("astra-full-fixture-")
            .tempdir()?
            .keep()
    };
    println!("fixture_root={}", root.display());
    if !root.join(".git").exists() {
        std::fs::write(root.join("AGENTS.md"),"Only implement this fixture in your assigned worktree. No subagents or external projects. Use Python standard library. Do not edit acceptance.py. Never merge branches.\n")?;
        std::fs::write(
            root.join(".gitignore"),
            ".agent-worktrees/\n__pycache__/\n*.db*\n*.jsonl\n*.json\n",
        )?;
        std::fs::write(
            root.join("acceptance.py"),
            r#"import unittest
from words import count_words
from lines import count_lines
from stats import text_stats
class Acceptance(unittest.TestCase):
 def test_words(self): self.assertEqual(count_words(' hello  world\nagain '),3)
 def test_empty_words(self): self.assertEqual(count_words('  \t\n'),0)
 def test_apostrophe(self): self.assertEqual(count_words("can't stop"),2)
 def test_lines(self): self.assertEqual(count_lines('a\nb\n'),2)
 def test_empty_lines(self): self.assertEqual(count_lines(''),0)
 def test_blank_line(self): self.assertEqual(count_lines('\n'),1)
 def test_integrated(self): self.assertEqual(text_stats('a b\nc'),{'words':3,'lines':2})
 def test_empty(self): self.assertEqual(text_stats(''),{'words':0,'lines':0})
if __name__=='__main__': unittest.main()
"#,
        )?;
        for args in [
            vec!["init"],
            vec!["add", "AGENTS.md", ".gitignore", "acceptance.py"],
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
                bail!("git setup failed");
            }
        }
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
    std::io::BufReader::new(child.stdout.take().context("server stdout missing")?)
        .read_line(&mut port)?;
    let _server = Server(child);
    let config = OrgBrainConfig {
        endpoint: format!("http://127.0.0.1:{}/mcp", port.trim()),
        tenant: "fixture".into(),
        remote_project_id: "fixture-project".into(),
        auto_store: true,
    };
    let store = Arc::new(Mutex::new(hub_db::Store::open(&root.join("hub.db"))?));
    let project = store.lock().unwrap().register_project(&root)?;
    let local = Arc::new(SparkProvider::new("http://127.0.0.1:8765")?);
    local.available().await?;
    let memory = OrgBrain::new(config.clone(), "fixture-id".into(), "fixture-secret".into())?;
    if !root.join("orgbrain-fixture.json").exists() {
        let extracted=local.extract_memory("Durable fixture architecture decision: use Python whitespace splitting for count_words and str.splitlines for count_lines. Reject punctuation-based word splitting because it splits apostrophes inconsistently. This policy applies to future text_stats work.",&["policy-1: Confirmed fixture policy: count_words(text)=len(text.split()); count_lines(text)=len(text.splitlines()); text_stats returns a dict with words and lines. Rejected alternative: punctuation splitting; reason: inconsistent apostrophe handling.".into()]).await?;
        let seed = hub_memory::durable_items(
            project.id,
            TaskId::default(),
            extracted.output,
            &["policy-1".into()],
        )?;
        if seed.is_empty() {
            bail!("Spark did not extract the durable fixture policy");
        }
        memory.store(seed).await?;
    }
    drop(memory);
    let memory = Arc::new(OrgBrain::new(
        config,
        "fixture-id".into(),
        "fixture-secret".into(),
    )?);
    if memory
        .search(MemorySearchRequest {
            query: "text_stats".into(),
            limit: 8,
        })
        .await?
        .is_empty()
    {
        bail!("cross-session retrieval failed");
    }
    let bus = hub_events::EventBus::new(store.clone());
    let b = bus.clone();
    let p = Arc::new(
        CodexProvider::spawn_with_sink(
            &std::env::var("CODEX_BIN").unwrap_or_else(|_| "codex".into()),
            Arc::new(move |e| b.publish(e)),
            root.join("provider.jsonl"),
        )
        .await?,
    );
    let astra = Astra {
        reasoning: None,
        mode: AstraAccessMode::CodexIntegrated,
        provider: p.clone(),
        root: root.clone(),
        model: "gpt-6-astra".into(),
    };
    println!("fixture_root={}", root.display());
    let mut plan=astra.create_plan(PlanningRequest{goal:"Create exactly three tasks: two independent workers for words.py count_words(text) and lines.py count_lines(text), each with a separate unittest file. Third worker depends on both, implements stats.py text_stats(text) returning {'words':count_words(text),'lines':count_lines(text)}, adds test_stats.py and runs all tests and acceptance.py. Read fixture policy from OrgBrain via hub_context (source org_brain, query text_stats, token_budget 1000). Python standard library. Do not modify acceptance.py.".into(),constraints:vec!["Only fixture worktrees; no delegation or branch merges".into()],evidence:vec!["Fixed acceptance.py imports all three exact functions and tests whitespace, apostrophes, blank lines and trailing newlines. The prior policy is available through OrgBrain.".into()]}).await?;
    for step in &mut plan.steps {
        step.goal=format!("First retrieve prior policy via hub_context source org_brain query text_stats token_budget 1000. Then: {}",step.goal);
    }
    let (dag, briefs) = plan.compile(project.id)?;
    if dag.tasks.len() != 3
        || dag
            .tasks
            .values()
            .filter(|t| t.dependencies.is_empty())
            .count()
            != 2
    {
        bail!("planner did not produce the requested parallel DAG");
    }
    {
        let s = store.lock().unwrap();
        s.save_plan(
            dag.tasks.values().next().unwrap().plan_id.unwrap(),
            project.id,
            &serde_json::to_string(&plan)?,
        )?;
        for t in dag.tasks.values() {
            s.save_task(t)?;
        }
        for t in dag.tasks.values() {
            s.save_dependencies(t)?;
        }
    }
    let sessions = Arc::new(Sessions::new(store.clone(), p.clone()));
    sessions.set_memory(project.id, memory.clone())?;
    let workers = Arc::new(Workers {
        store: store.clone(),
        sessions,
        provider: p.clone(),
        local: Some(local),
        memory: Some(memory.clone()),
        bus,
        worktrees: hub_worktree::GitWorktrees::new(&root)?,
        base_sha: "HEAD".into(),
        briefs,
        outcomes: Mutex::new(HashMap::new()),
    });
    let dag = hub_scheduler::execute(dag, workers.clone(), 2).await?;
    for t in dag.tasks.values() {
        println!("task={} status={:?}", t.title, t.status);
    }
    if dag
        .tasks
        .values()
        .any(|t| t.status != TaskStatus::Completed)
    {
        bail!("full workflow worker failed");
    }
    p.shutdown().await?;
    drop(_server);
    finishing::finish(root).await
}
