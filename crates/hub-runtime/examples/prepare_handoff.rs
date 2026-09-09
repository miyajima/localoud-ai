//! Offline exporter using the same selection and capsule preflight as Workers.
use anyhow::{bail, Context, Result};
use hub_core::Task;
use hub_runtime::plan::ExecutionPlan;

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 2 {
        bail!("usage: prepare_handoff <execution-plan.json>");
    }
    let plan: ExecutionPlan = serde_json::from_slice(&std::fs::read(&args[1])?)?;
    let project = hub_core::ProjectId::default();
    plan.compile(project)?;
    let step = plan.steps.first().context("missing step")?;
    let task = Task::new(project, &step.title, &step.goal);
    let item = step
        .brief
        .prepare_handoff(&task)?
        .context("first step has no handoff")?;
    println!("{}", item.text);
    Ok(())
}
