//! Serialized per-thread mutations with durable Hub/provider mapping.
pub mod memory_capture;
pub mod plan;
use anyhow::{anyhow, bail, Result};
use hub_core::{HubThreadId, ProjectId, ThreadMapping};
use hub_db::Store;
use protocol_types::{CodingAgentProvider, ProviderThread, ProviderTurn, ThreadSnapshot};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex as StdMutex},
};
use tokio::sync::Mutex;

#[derive(Default)]
struct ThreadState {
    active: Option<ProviderTurn>,
    reconciled: bool,
}
pub struct Sessions {
    store: Arc<StdMutex<Store>>,
    provider: Arc<dyn CodingAgentProvider>,
    memories: StdMutex<HashMap<ProjectId, Arc<dyn hub_memory::DurableMemoryProvider>>>,
    actors: Mutex<HashMap<HubThreadId, Arc<Mutex<ThreadState>>>>,
}
impl Sessions {
    pub fn new(store: Arc<StdMutex<Store>>, provider: Arc<dyn CodingAgentProvider>) -> Self {
        Self {
            store,
            provider,
            actors: Mutex::new(HashMap::new()),
            memories: StdMutex::new(HashMap::new()),
        }
    }
    pub fn set_memory(
        &self,
        project: ProjectId,
        memory: Arc<dyn hub_memory::DurableMemoryProvider>,
    ) -> Result<()> {
        self.memories
            .lock()
            .map_err(|_| anyhow!("memory registry poisoned"))?
            .insert(project, memory);
        Ok(())
    }
    async fn actor(&self, id: HubThreadId) -> Arc<Mutex<ThreadState>> {
        self.actors
            .lock()
            .await
            .entry(id)
            .or_insert_with(|| Arc::new(Mutex::new(ThreadState::default())))
            .clone()
    }
    fn mapping(&self, id: HubThreadId) -> Result<ThreadMapping> {
        self.store
            .lock()
            .map_err(|_| anyhow!("database lock poisoned"))?
            .threads()?
            .into_iter()
            .find(|t| t.id == id)
            .ok_or_else(|| anyhow!("unknown Hub thread"))
    }
    fn project_root(&self, id: ProjectId) -> Result<std::path::PathBuf> {
        self.store
            .lock()
            .map_err(|_| anyhow!("database lock poisoned"))?
            .projects()?
            .into_iter()
            .find(|p| p.id == id)
            .map(|p| p.root)
            .ok_or_else(|| anyhow!("unregistered project"))
    }
    pub async fn diff(&self, id: HubThreadId) -> Result<String> {
        repository_diff(&self.store, id).await
    }
    pub async fn create(&self, project: ProjectId, title: String) -> Result<ThreadMapping> {
        self.create_with_model(project, title, None).await
    }
    pub async fn create_with_model(
        &self,
        project: ProjectId,
        title: String,
        model: Option<&str>,
    ) -> Result<ThreadMapping> {
        let root = self.project_root(project)?;
        let thread = match model {
            Some(model) => self.provider.start_thread_with_model(root, model).await?,
            None => self.provider.start_thread(root).await?,
        };
        let mapping = ThreadMapping {
            id: HubThreadId::default(),
            project_id: project,
            provider: "codex".into(),
            provider_thread_id: thread.id,
            title,
            status: "idle".into(),
        };
        self.store
            .lock()
            .map_err(|_| anyhow!("database lock poisoned"))?
            .save_thread(&mapping)?;
        if let Some(model) = model {
            self.store
                .lock()
                .map_err(|_| anyhow!("database lock poisoned"))?
                .set_setting(&format!("thread_model:{}", mapping.id), model)?;
        }
        self.actor(mapping.id).await.lock().await.reconciled = true;
        Ok(mapping)
    }
    pub async fn create_worker(
        &self,
        task: &hub_core::Task,
        worktree: &hub_core::Worktree,
        handler: Arc<dyn protocol_types::AgentTool>,
        tools: Vec<protocol_types::ToolDefinition>,
    ) -> Result<ThreadMapping> {
        self.create_worker_with_options(task, worktree, handler, tools, None, None)
            .await
    }
    pub async fn create_worker_with_options(
        &self,
        task: &hub_core::Task,
        worktree: &hub_core::Worktree,
        handler: Arc<dyn protocol_types::AgentTool>,
        tools: Vec<protocol_types::ToolDefinition>,
        model: Option<&str>,
        effort: Option<&str>,
    ) -> Result<ThreadMapping> {
        if model.is_none() && effort.is_some() {
            bail!("Worker reasoning requires an explicit model");
        }
        let thread = match model {
            Some(model) => {
                self.provider
                    .start_worker_with_model(worktree.path.clone(), tools, handler, model, effort)
                    .await?
            }
            None => {
                self.provider
                    .start_worker(worktree.path.clone(), tools, handler)
                    .await?
            }
        };
        let mapping = ThreadMapping {
            id: HubThreadId::default(),
            project_id: task.project_id,
            provider: "codex".into(),
            provider_thread_id: thread.id,
            title: task.title.clone(),
            status: "idle".into(),
        };
        {
            let store = self
                .store
                .lock()
                .map_err(|_| anyhow!("database lock poisoned"))?;
            store.save_thread(&mapping)?;
            store.bind_worker_thread(mapping.id, task.id, hub_core::WorkerId::default())?;
            if let Some(model) = model {
                store.set_setting(&format!("thread_model:{}", mapping.id), model)?;
                if let Some(effort) = effort {
                    store.set_setting(&format!("thread_reasoning:{}", mapping.id), effort)?;
                }
            }
        }
        self.actor(mapping.id).await.lock().await.reconciled = true;
        Ok(mapping)
    }
    pub async fn resume(&self, id: HubThreadId) -> Result<ThreadSnapshot> {
        let actor = self.actor(id).await;
        let mut state = actor.lock().await;
        state.reconciled = false;
        let mut m = self.mapping(id)?;
        let root = self
            .store
            .lock()
            .map_err(|_| anyhow!("database lock poisoned"))?
            .thread_root(id)?;
        let task = self
            .store
            .lock()
            .map_err(|_| anyhow!("database lock poisoned"))?
            .thread_task(id)?;
        if let Some(task) = task {
            let has_capsule = self
                .store
                .lock()
                .map_err(|_| anyhow!("database lock poisoned"))?
                .capsule(task)?
                .is_some();
            if has_capsule {
                let memory = self
                    .memories
                    .lock()
                    .map_err(|_| anyhow!("memory registry poisoned"))?
                    .get(&m.project_id)
                    .cloned();
                self.provider
                    .attach_worker(
                        &ProviderThread {
                            id: m.provider_thread_id.clone(),
                        },
                        Arc::new(hub_context::Broker {
                            memory,
                            store: self.store.clone(),
                            task,
                            project: m.project_id,
                            root: root.clone(),
                        }),
                    )
                    .await?;
            }
        }
        let pinned = self
            .store
            .lock()
            .map_err(|_| anyhow!("database lock poisoned"))?
            .setting(&format!("thread_model:{id}"))?;
        let thread = ProviderThread {
            id: m.provider_thread_id.clone(),
        };
        let result = match pinned {
            Some(model) => {
                self.provider
                    .resume_thread_with_model(&thread, root, &model)
                    .await?
            }
            None => self.provider.resume_thread(&thread, root).await?,
        };
        state.active = result.active_turn.clone();
        state.reconciled = true;
        m.status = if state.active.is_some() {
            "running"
        } else {
            "idle"
        }
        .into();
        self.store
            .lock()
            .map_err(|_| anyhow!("database lock poisoned"))?
            .save_thread(&m)?;
        Ok(result)
    }
    pub async fn start(&self, id: HubThreadId, text: String) -> Result<ProviderTurn> {
        self.start_with_options(id, text, protocol_types::composer::TurnOptions::default())
            .await
    }
    pub async fn start_with_options(
        &self,
        id: HubThreadId,
        text: String,
        options: protocol_types::composer::TurnOptions,
    ) -> Result<ProviderTurn> {
        options.validate()?;
        if text.trim().is_empty() {
            bail!("task input is empty");
        }
        let actor = self.actor(id).await;
        let mut state = actor.lock().await;
        if !state.reconciled {
            bail!("resume the thread to reconcile provider state first");
        }
        let m = self.mapping(id)?;
        if state.active.is_some() {
            // Re-read under the same actor lock; a completed event may arrive before
            // or after turn/start returns. Provider state resolves that race.
            let current = self
                .provider
                .read_thread(&ProviderThread {
                    id: m.provider_thread_id.clone(),
                })
                .await?;
            state.active = current.active_turn;
            if state.active.is_some() {
                bail!("thread already has an active turn; use steer");
            }
        }
        if !options.is_empty() {
            let store = self
                .store
                .lock()
                .map_err(|_| anyhow!("database lock poisoned"))?;
            anyhow::ensure!(
                store.setting(&format!("thread_model:{id}"))?.is_some(),
                "この既存タスクにはモデル指定がありません。新規タスクを作成してください。"
            );
        }
        let requested_mode = options.mode;
        let intent = self
            .store
            .lock()
            .map_err(|_| anyhow!("database lock poisoned"))?
            .record_turn_intent(id)?;
        state.reconciled = false;
        let (pinned_model, pinned_effort) = {
            let store = self
                .store
                .lock()
                .map_err(|_| anyhow!("database lock poisoned"))?;
            (
                store.setting(&format!("thread_model:{id}"))?,
                store.setting(&format!("thread_reasoning:{id}"))?,
            )
        };
        let thread = ProviderThread {
            id: m.provider_thread_id.clone(),
        };
        let started = match pinned_model {
            Some(model) => {
                self.provider
                    .start_turn_with_options(
                        &thread,
                        text,
                        &model,
                        pinned_effort.as_deref(),
                        options,
                    )
                    .await
            }
            None => self.provider.start_turn(&thread, text).await,
        };
        match started {
            Ok(turn) => {
                self.store
                    .lock()
                    .map_err(|_| anyhow!("database lock poisoned"))?
                    .resolve_turn_intent(&intent, Some(&turn.id), &turn.status)?;
                if let Some(mode) = requested_mode {
                    self.store
                        .lock()
                        .map_err(|_| anyhow!("database lock poisoned"))?
                        .set_setting(
                            &format!("thread_mode:{id}"),
                            &serde_json::to_string(&mode)?,
                        )?;
                }
                state.active = Some(turn.clone());
                state.reconciled = true;
                // Lifecycle status is journaled by the event sink. Do not overwrite
                // a completion notification that raced ahead of this reply.
                Ok(turn)
            }
            Err(e) => {
                self.store
                    .lock()
                    .map_err(|_| anyhow!("database lock poisoned"))?
                    .resolve_turn_intent(&intent, None, "unknown")?;
                Err(e)
            }
        }
    }
    pub async fn steer(&self, id: HubThreadId, text: String) -> Result<()> {
        if text.trim().is_empty() {
            bail!("steering input is empty");
        }
        let actor = self.actor(id).await;
        let mut state = actor.lock().await;
        if !state.reconciled {
            bail!("resume before steering");
        }
        let turn = state
            .active
            .clone()
            .ok_or_else(|| anyhow!("no active turn"))?;
        self.store
            .lock()
            .map_err(|_| anyhow!("database lock poisoned"))?
            .record_command(id, "turn/steer")?;
        state.reconciled = false;
        self.provider.steer_turn(&turn, text).await?;
        state.reconciled = true;
        Ok(())
    }
    pub async fn interrupt(&self, id: HubThreadId) -> Result<()> {
        let actor = self.actor(id).await;
        let mut state = actor.lock().await;
        if !state.reconciled {
            bail!("resume before interrupting");
        }
        let turn = state
            .active
            .clone()
            .ok_or_else(|| anyhow!("no active turn"))?;
        self.store
            .lock()
            .map_err(|_| anyhow!("database lock poisoned"))?
            .record_command(id, "turn/interrupt")?;
        state.reconciled = false;
        self.provider.interrupt_turn(&turn).await?;
        // Interrupt acknowledgement does not prove turn completion.
        let snapshot = self
            .provider
            .read_thread(&ProviderThread { id: turn.thread_id })
            .await?;
        state.active = snapshot.active_turn;
        state.reconciled = true;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use provider_codex::CodexProvider;
    #[tokio::test]
    async fn mappings_survive_restart_and_start_is_serialized() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let init = std::process::Command::new("git")
            .arg("init")
            .arg(dir.path())
            .output()?;
        assert!(init.status.success());
        let store = Arc::new(StdMutex::new(Store::open(&dir.path().join("hub.db"))?));
        let project = store.lock().unwrap().register_project(dir.path())?;
        let script = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/codex-events/fake_server.py"
        );
        let provider = Arc::new(CodexProvider::spawn_command("python3", &[script]).await?);
        let sessions = Sessions::new(store.clone(), provider.clone());
        let thread = sessions.create(project.id, "Fixture".into()).await?;
        let (a, b) = tokio::join!(
            sessions.start(thread.id, "one".into()),
            sessions.start(thread.id, "two".into())
        );
        assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
        drop(sessions);
        let restored = Sessions::new(store.clone(), provider.clone());
        assert!(restored
            .start(thread.id, "not reconciled".into())
            .await
            .is_err());
        assert!(restored.resume(thread.id).await?.active_turn.is_some());
        restored.interrupt(thread.id).await?;
        assert!(restored.resume(thread.id).await?.active_turn.is_none());
        assert_eq!(
            store.lock().unwrap().threads()?[0].provider_thread_id,
            thread.provider_thread_id
        );
        provider.shutdown().await?;
        Ok(())
    }
}

pub mod local_worker;

pub async fn repository_diff(store: &std::sync::Mutex<Store>, id: HubThreadId) -> Result<String> {
    let worker = {
        let s = store
            .lock()
            .map_err(|_| anyhow!("database lock poisoned"))?;
        if let Some(task) = s.thread_task(id)? {
            if let Some(w) = s.worktree_for_task(task)? {
                let p = s
                    .threads()?
                    .into_iter()
                    .find(|t| t.id == id)
                    .ok_or_else(|| anyhow!("thread missing"))?
                    .project_id;
                let root = s
                    .projects()?
                    .into_iter()
                    .find(|p2| p2.id == p)
                    .ok_or_else(|| anyhow!("project missing"))?
                    .root;
                Some((root, w))
            } else {
                None
            }
        } else {
            None
        }
    };
    if let Some((root, w)) = worker {
        return hub_worktree::GitWorktrees::new(&root)?.diff(&w).await;
    }
    let root = store
        .lock()
        .map_err(|_| anyhow!("database lock poisoned"))?
        .thread_root(id)?;
    if root.canonicalize()? != root {
        bail!("registered project root changed; register it again");
    }
    let mut command = tokio::process::Command::new("git");
    command
        .arg("-C")
        .arg(root)
        .args(["diff", "--no-ext-diff", "--no-textconv", "HEAD", "--"])
        .kill_on_drop(true);
    let result =
        tokio::time::timeout(std::time::Duration::from_secs(10), command.output()).await??;
    if !result.status.success() {
        bail!(
            "Git diff failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    if result.stdout.len() > 4_000_000 {
        bail!("diff exceeds 4 MB; inspect it in the repository");
    }
    Ok(String::from_utf8(result.stdout)?)
}
pub mod workers;
