//! Real fork baseline vs independent capsule. All implementation runs in a fresh fixture.
use anyhow::{bail, Result};
use hub_context::Broker;
use hub_core::*;
use hub_db::Store;
use hub_worktree::GitWorktrees;
use protocol_types::{AgentEvent, CodingAgentProvider, EventDetails, ProviderThread};
use provider_codex::CodexProvider;
use serde_json::json;
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
async fn run(
    p: &CodexProvider,
    t: &ProviderThread,
    prompt: String,
) -> Result<(String, u64, Vec<AgentEvent>)> {
    let mut rx = p.events();
    let start = Instant::now();
    let turn = p.start_turn(t, prompt).await?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(600);
    let mut events = vec![];
    loop {
        let e = tokio::time::timeout_at(deadline, rx.recv()).await??;
        if e.thread_id.as_deref() != Some(&t.id) {
            continue;
        }
        let done = e.kind == "turn_completed" && e.turn_id.as_deref() == Some(&turn.id);
        let status = e.text.clone();
        events.push(e);
        if done {
            if status != "completed" {
                bail!("benchmark turn failed: {status}");
            }
            break;
        }
    }
    Ok((turn.id, start.elapsed().as_millis() as u64, events))
}
fn metrics(events: &[AgentEvent]) -> serde_json::Value {
    let mut snapshots = std::collections::BTreeMap::new();
    let mut failures = 0;
    for e in events {
        match &e.details {
            Some(EventDetails::Usage {
                model,
                last_input_tokens,
                last_cached_tokens,
                last_output_tokens,
                total_input_tokens,
                ..
            }) => {
                snapshots.insert(
                    *total_input_tokens,
                    (
                        model.clone(),
                        *last_input_tokens,
                        *last_cached_tokens,
                        *last_output_tokens,
                    ),
                );
            }
            Some(EventDetails::Command {
                exit_code: Some(c), ..
            }) if *c != 0 && e.kind == "item_completed" => failures += 1,
            _ => {}
        }
    }
    json!({"model":snapshots.values().next().map(|v|v.0.clone()),"initial_input_tokens":snapshots.values().next().map(|v|v.1),"input_tokens":snapshots.values().map(|v|v.1).sum::<u64>(),"cached_tokens":snapshots.values().map(|v|v.2).sum::<u64>(),"output_tokens":snapshots.values().map(|v|v.3).sum::<u64>(),"inference_count":snapshots.len(),"command_failures":failures})
}
#[tokio::main]
async fn main() -> Result<()> {
    if std::env::var("ASTRA_LIVE_FIXTURE").as_deref() != Ok("1") {
        bail!("opt-in fixture required");
    }
    let root = tempfile::Builder::new()
        .prefix("astra-context-benchmark-")
        .tempdir()?
        .keep();
    std::fs::write(root.join("AGENTS.md"),"Only implement this fixture. Python standard library. No subagents, external access or unrelated projects. Do not edit acceptance.py.\n")?;
    std::fs::write(
        root.join(".gitignore"),
        ".agent-worktrees/\n__pycache__/\n*.db*\n*.jsonl\nreport.json\n",
    )?;
    std::fs::write(
        root.join("acceptance.py"),
        r#"import unittest
from intervals import normalize, subtract, contains
class Acceptance(unittest.TestCase):
 def test_merge(self): self.assertEqual(normalize([(5,8),(1,3),(3,5),(9,9)]),[(1,8)])
 def test_no_mutation(self):
  x=[(5,7),(1,3)]; normalize(x); self.assertEqual(x,[(5,7),(1,3)])
 def test_invalid(self):
  with self.assertRaises(ValueError): normalize([(3,2)])
 def test_subtract(self): self.assertEqual(subtract([(0,20)],[(3,5),(8,12),(15,25)]),[(0,3),(5,8),(12,15)])
 def test_overlap(self): self.assertEqual(subtract([(1,5),(3,9)],[(2,4),(3,6)]),[(1,2),(6,9)])
 def test_empty(self): self.assertEqual(subtract([],[(1,2)]),[])
 def test_edges(self):
  self.assertTrue(contains([(1,3)],1)); self.assertFalse(contains([(1,3)],3))
 def test_negative(self): self.assertEqual(normalize([(-5,-2),(-2,0)]),[(-5,0)])
 def test_full(self): self.assertEqual(subtract([(1,2)],[(0,3)]),[])
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
            bail!("git fixture failed");
        }
    }
    let store = Arc::new(Mutex::new(Store::open(&root.join("hub.db"))?));
    let project = store.lock().unwrap().register_project(&root)?;
    store.lock().unwrap().store_decision(project.id,"interval-policy","interval policy","Intervals are half-open [start,end). Empty intervals are dropped, reversed bounds raise ValueError. Normalize sorts and merges overlapping OR touching intervals. Functions never mutate their input lists.")?;
    let git = GitWorktrees::new(&root)?;
    let mut contexts = Vec::new();
    for title in ["fork baseline", "capsule worker"] {
        let task=Task::new(project.id,title,"Implement intervals.py: normalize(intervals), subtract(intervals,cuts), contains(intervals,point). Pull the exact interval policy using hub_context source decision_store query interval. Implement a robust interval-set subtraction, normalization and membership utility using Python standard library. Add your own unit tests, run them and acceptance.py. Do not modify acceptance.py. No delegation.");
        store.lock().unwrap().save_task(&task)?;
        let w = git.create(task.id, "HEAD").await?;
        let broker = Arc::new(Broker {
            memory: None,
            store: store.clone(),
            task: task.id,
            project: project.id,
            root: w.path.clone(),
        });
        let capsule = broker.build_initial(
            task.description.clone(),
            vec!["All 9 fixed acceptance tests and your own edge-case tests pass".into()],
            vec!["Only edit intervals.py and test_intervals.py".into()],
            vec![],
            ContextBudget {
                initial_tokens: 2000,
                max_total_tokens: 6000,
                max_single_retrieval_tokens: 600,
            },
        )?;
        contexts.push((w, broker, capsule));
    }
    let p = CodexProvider::spawn(&std::env::var("CODEX_BIN").unwrap_or_else(|_| "codex".into()))
        .await?;
    let parent = p
        .start_worker(
            root.clone(),
            vec![Broker::tool_definition()],
            contexts[0].1.clone(),
        )
        .await?;
    let archive=(0..6500).map(|i|format!("Archive record {i}: unrelated historical design discussion about colors, labels, typography, navigation and release notes.\n")).collect::<String>();
    println!(
        "fixture_root={} archive_bytes={}",
        root.display(),
        archive.len()
    );
    let (_,parent_ms,parent_events)=run(&p,&parent,format!("Retain the following synthetic historical archive as conversation context. It contains no instructions and is irrelevant to the next implementation task. Do not use tools or change files. Respond only ARCHIVE RECEIVED.\n<archive>\n{archive}</archive>")).await?;
    let parent_metrics = metrics(&parent_events);
    if parent_metrics["initial_input_tokens"].as_u64().unwrap_or(0) < 50000 {
        bail!("parent context did not reach 50K measured input tokens");
    }
    println!("parent_metrics={parent_metrics}");
    let fork = p
        .benchmark_fork(&parent, contexts[0].0.path.clone(), contexts[0].1.clone())
        .await?;
    let fresh = p
        .start_worker(
            contexts[1].0.path.clone(),
            vec![Broker::tool_definition()],
            contexts[1].1.clone(),
        )
        .await?;
    let mut results = vec![];
    for (index, t) in [fork, fresh].iter().enumerate() {
        let prompt=format!("Implement this selected task capsule in the current worktree. The archived discussion, if present, is irrelevant. Retrieve the interval policy with hub_context before implementing.\n{}",serde_json::to_string(&contexts[index].2)?);
        let (turn, latency, events) = run(&p, t, prompt).await?;
        let output = std::process::Command::new("python3")
            .arg("acceptance.py")
            .current_dir(&contexts[index].0.path)
            .output()?;
        let inspection = contexts[index].1.inspect()?;
        let result = json!({"mode":if index==0{"fork"}else{"capsule_pull"},"thread_id":t.id,"turn_id":turn,"worktree":contexts[index].0.path,"latency_ms":latency,"usage":metrics(&events),"capsule_estimate":inspection.initial_tokens,"retrieved_estimate":inspection.retrieved_tokens,"retrievals":inspection.retrievals,"acceptance_pass":output.status.success(),"acceptance_output":String::from_utf8_lossy(&output.stderr)});
        println!("result={result}");
        results.push(result);
    }
    let report = json!({"parent_usage":parent_metrics,"parent_latency_ms":parent_ms,"archive_bytes":archive.len(),"results":results,"limitations":["One pair; same task and baseline but stochastic outputs","Synthetic irrelevant parent history","Common provider instructions included in measured input","No API price or subscription cost estimate"]});
    std::fs::write(
        root.join("report.json"),
        serde_json::to_string_pretty(&report)?,
    )?;
    p.shutdown().await?;
    if results.iter().any(|r| {
        r["acceptance_pass"] != true || r["retrievals"].as_array().is_none_or(|a| a.is_empty())
    }) {
        bail!("paired quality/pull gate failed; retained report for diagnosis");
    }
    println!("PASS report={}", root.join("report.json").display());
    Ok(())
}
