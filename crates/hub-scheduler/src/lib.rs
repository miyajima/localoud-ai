use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;
use hub_core::{Task, TaskId, TaskStatus};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use tokio::task::JoinSet;

pub struct Dag {
    pub tasks: HashMap<TaskId, Task>,
}
impl Dag {
    pub fn new(tasks: Vec<Task>) -> Result<Self> {
        if tasks.is_empty() {
            bail!("plan has no tasks");
        }
        let count = tasks.len();
        let project = tasks[0].project_id;
        let tasks: HashMap<_, _> = tasks.into_iter().map(|t| (t.id, t)).collect();
        if tasks.len() != count {
            bail!("duplicate task ID");
        }
        for t in tasks.values() {
            if t.project_id != project {
                bail!("a DAG cannot cross project boundaries");
            }
            let mut seen = HashSet::new();
            for dep in &t.dependencies {
                if *dep == t.id || !tasks.contains_key(dep) || !seen.insert(dep) {
                    bail!("invalid task dependency");
                }
            }
        }
        let mut completed = HashSet::new();
        while completed.len() < count {
            let next: Vec<_> = tasks
                .values()
                .filter(|t| {
                    !completed.contains(&t.id)
                        && t.dependencies.iter().all(|d| completed.contains(d))
                })
                .map(|t| t.id)
                .collect();
            if next.is_empty() {
                bail!("task dependency cycle");
            }
            completed.extend(next);
        }
        Ok(Self { tasks })
    }
    pub fn refresh(&mut self) {
        let statuses: HashMap<_, _> = self.tasks.iter().map(|(id, t)| (*id, t.status)).collect();
        for task in self.tasks.values_mut() {
            if matches!(task.status, TaskStatus::Pending | TaskStatus::Blocked) {
                if task
                    .dependencies
                    .iter()
                    .any(|d| matches!(statuses[d], TaskStatus::Failed | TaskStatus::Cancelled))
                {
                    task.status = TaskStatus::Blocked;
                } else if task
                    .dependencies
                    .iter()
                    .all(|d| statuses[d] == TaskStatus::Completed)
                {
                    task.status = TaskStatus::Ready;
                }
            }
        }
    }
    pub fn transition(&mut self, id: TaskId, next: TaskStatus) -> Result<()> {
        let task = self
            .tasks
            .get_mut(&id)
            .ok_or_else(|| anyhow!("unknown task"))?;
        if !task.status.can_transition(next) {
            bail!(
                "invalid task state transition {:?} -> {:?}",
                task.status,
                next
            );
        }
        task.status = next;
        Ok(())
    }
}
#[async_trait]
pub trait TaskRunner: Send + Sync {
    async fn run(&self, task: Task) -> Result<()>;
    async fn changed(&self, task: &Task) -> Result<()>;
}
pub async fn execute(mut dag: Dag, runner: Arc<dyn TaskRunner>, concurrency: usize) -> Result<Dag> {
    if concurrency == 0 || concurrency > 8 {
        bail!("concurrency must be between 1 and 8");
    }
    let mut jobs = JoinSet::new();
    loop {
        dag.refresh();
        let ready: Vec<_> = dag
            .tasks
            .values()
            .filter(|t| t.status == TaskStatus::Ready)
            .map(|t| t.id)
            .take(concurrency - jobs.len())
            .collect();
        for id in ready {
            dag.transition(id, TaskStatus::Running)?;
            dag.tasks.get_mut(&id).unwrap().attempts += 1;
            let task = dag.tasks[&id].clone();
            runner.changed(&task).await?;
            let r = runner.clone();
            jobs.spawn(async move {
                let result = r.run(task).await;
                (id, result)
            });
        }
        if jobs.is_empty() {
            break;
        }
        let (id, result) = jobs.join_next().await.context_join()?;
        if result.is_ok() {
            dag.transition(id, TaskStatus::Reviewing)?;
            runner.changed(&dag.tasks[&id]).await?;
            dag.transition(id, TaskStatus::Completed)?;
        } else {
            dag.transition(id, TaskStatus::Failed)?;
        }
        runner.changed(&dag.tasks[&id]).await?;
    }
    for task in dag.tasks.values() {
        runner.changed(task).await?;
    }
    Ok(dag)
}
trait JoinContext<T> {
    fn context_join(self) -> Result<T>;
}
impl<T> JoinContext<T> for Option<std::result::Result<T, tokio::task::JoinError>> {
    fn context_join(self) -> Result<T> {
        self.ok_or_else(|| anyhow!("scheduler lost active job"))?
            .map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hub_core::ProjectId;
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[test]
    fn cycles_and_missing_dependencies_rejected() {
        let mut a = Task::new(ProjectId::default(), "a", "a");
        let mut b = Task::new(a.project_id, "b", "b");
        a.dependencies.push(b.id);
        b.dependencies.push(a.id);
        assert!(Dag::new(vec![a, b]).is_err());
    }
    struct Runner {
        active: AtomicUsize,
        peak: AtomicUsize,
        finished: std::sync::Mutex<HashSet<TaskId>>,
    }
    #[async_trait]
    impl TaskRunner for Runner {
        async fn run(&self, task: Task) -> Result<()> {
            assert!(task
                .dependencies
                .iter()
                .all(|d| self.finished.lock().unwrap().contains(d)));
            let n = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(n, Ordering::SeqCst);
            tokio::task::yield_now().await;
            self.active.fetch_sub(1, Ordering::SeqCst);
            self.finished.lock().unwrap().insert(task.id);
            Ok(())
        }
        async fn changed(&self, _: &Task) -> Result<()> {
            Ok(())
        }
    }
    #[tokio::test]
    async fn independent_tasks_overlap_and_dependencies_wait() -> Result<()> {
        let project = ProjectId::default();
        let a = Task::new(project, "a", "a");
        let b = Task::new(project, "b", "b");
        let mut c = Task::new(project, "c", "c");
        c.dependencies = vec![a.id, b.id];
        let r = Arc::new(Runner {
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            finished: std::sync::Mutex::new(HashSet::new()),
        });
        let d = execute(Dag::new(vec![a, b, c])?, r.clone(), 2).await?;
        assert!(d.tasks.values().all(|t| t.status == TaskStatus::Completed));
        assert_eq!(r.peak.load(Ordering::SeqCst), 2);
        Ok(())
    }
}
