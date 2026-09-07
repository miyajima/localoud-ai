use crate::workers::WorkerBrief;
use anyhow::{bail, Context, Result};
use hub_core::*;
use hub_scheduler::Dag;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanStep {
    pub key: String,
    pub title: String,
    pub goal: String,
    pub dependencies: Vec<String>,
    pub brief: WorkerBrief,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionPlan {
    pub title: String,
    pub steps: Vec<PlanStep>,
}
impl ExecutionPlan {
    pub fn compile(&self, project: ProjectId) -> Result<(Dag, HashMap<TaskId, WorkerBrief>)> {
        if self.title.trim().is_empty() || self.steps.is_empty() || self.steps.len() > 32 {
            bail!("plan needs a title and 1–32 steps");
        }
        let plan = PlanId::default();
        let mut ids = HashMap::new();
        for s in &self.steps {
            if s.key.is_empty()
                || s.title.trim().is_empty()
                || s.goal.trim().is_empty()
                || s.brief.acceptance_criteria.is_empty()
            {
                bail!("each step needs a key, title, goal and acceptance criteria");
            }
            s.brief.budget.validate().map_err(anyhow::Error::msg)?;
            if ids.insert(s.key.clone(), TaskId::default()).is_some() {
                bail!("duplicate step key");
            }
        }
        let mut tasks = Vec::new();
        let mut briefs = HashMap::new();
        for s in &self.steps {
            let mut t = Task::new(project, &s.title, &s.goal);
            t.id = ids[&s.key];
            t.plan_id = Some(plan);
            t.dependencies = s
                .dependencies
                .iter()
                .map(|d| ids.get(d).copied().context("unknown dependency key"))
                .collect::<Result<_>>()?;
            briefs.insert(t.id, s.brief.clone());
            tasks.push(t);
        }
        Ok((Dag::new(tasks)?, briefs))
    }
}

/// Durable dispatch state; task IDs are preserved across a Hub restart.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanRuntime {
    pub plan_id: PlanId,
    pub base_sha: String,
    pub concurrency: usize,
    pub briefs: HashMap<TaskId, WorkerBrief>,
}
impl PlanRuntime {
    pub fn recover(
        &self,
        mut tasks: Vec<Task>,
        outcomes: &HashMap<TaskId, crate::workers::WorkerOutcome>,
    ) -> Result<Dag> {
        if !(1..=8).contains(&self.concurrency)
            || !matches!(self.base_sha.len(), 40 | 64)
            || !self.base_sha.bytes().all(|b| b.is_ascii_hexdigit())
        {
            bail!("invalid persisted dispatch settings");
        }
        tasks.retain(|t| t.plan_id == Some(self.plan_id));
        if tasks.len() != self.briefs.len()
            || tasks.iter().any(|t| !self.briefs.contains_key(&t.id))
        {
            bail!("persisted plan task set changed");
        }
        for t in &mut tasks {
            if let Some(outcome) = outcomes.get(&t.id) {
                if outcome.task_id != t.id
                    || outcome.worktree.task_id != t.id
                    || outcome.worktree.base_sha != self.base_sha
                {
                    bail!("outcome provenance mismatch");
                }
                t.status = TaskStatus::Completed;
            } else if t.status == TaskStatus::Completed {
                bail!("completed task outcome is missing");
            } else if matches!(t.status, TaskStatus::Running | TaskStatus::Reviewing) {
                t.status = TaskStatus::Pending;
            }
        }
        Dag::new(tasks)
    }
}

#[cfg(test)]
mod recovery_tests {
    use super::*;
    #[test]
    fn restart_preserves_ids_and_completed_dependencies() -> Result<()> {
        let project = ProjectId::default();
        let plan_id = PlanId::default();
        let mut a = Task::new(project, "done", "done");
        a.plan_id = Some(plan_id);
        a.status = TaskStatus::Completed;
        let mut b = Task::new(project, "interrupted", "interrupted");
        b.plan_id = Some(plan_id);
        b.status = TaskStatus::Running;
        b.dependencies = vec![a.id];
        let brief = WorkerBrief {
            acceptance_criteria: vec!["test".into()],
            constraints: vec![],
            context_items: vec![],
            budget: ContextBudget {
                initial_tokens: 1000,
                max_total_tokens: 4000,
                max_single_retrieval_tokens: 1000,
            },
        };
        let runtime = PlanRuntime {
            plan_id,
            base_sha: "a".repeat(40),
            concurrency: 2,
            briefs: HashMap::from([(a.id, brief.clone()), (b.id, brief)]),
        };
        let outcome = crate::workers::WorkerOutcome {
            task_id: a.id,
            thread_id: HubThreadId::default(),
            worktree: Worktree {
                id: WorktreeId::default(),
                task_id: a.id,
                path: "/tmp/fixture".into(),
                base_sha: runtime.base_sha.clone(),
            },
            diff: String::new(),
            latency_ms: 1,
            command_failures: 0,
        };
        assert!(runtime
            .recover(vec![a.clone(), b.clone()], &HashMap::new())
            .is_err());
        let mut dag = runtime.recover(
            vec![a.clone(), b.clone()],
            &HashMap::from([(a.id, outcome)]),
        )?;
        dag.refresh();
        assert_eq!(dag.tasks[&a.id].status, TaskStatus::Completed);
        assert_eq!(dag.tasks[&b.id].status, TaskStatus::Ready);
        assert_eq!(dag.tasks[&b.id].dependencies, vec![a.id]);
        Ok(())
    }
}
