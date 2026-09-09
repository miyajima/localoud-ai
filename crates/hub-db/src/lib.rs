use anyhow::{bail, Context, Result};
use hub_core::{HubThreadId, Project, ProjectId, ThreadMapping};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

pub struct Store {
    conn: Connection,
}
impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version > 4 {
            bail!("database version {version} is newer than this application");
        }
        if version == 0 {
            conn.execute_batch(concat!(
                "BEGIN;",
                include_str!("../../../migrations/001_initial.sql"),
                "COMMIT;"
            ))?;
        }
        if version < 2 {
            conn.execute_batch(concat!(
                "BEGIN;",
                include_str!("../../../migrations/002_context.sql"),
                "COMMIT;"
            ))?;
        }
        if version < 3 {
            conn.execute_batch(concat!(
                "BEGIN;",
                include_str!("../../../migrations/003_usage.sql"),
                "COMMIT;"
            ))?;
        }
        if version < 4 {
            conn.execute_batch(concat!(
                "BEGIN;",
                include_str!("../../../migrations/004_lifecycle.sql"),
                "COMMIT;"
            ))?;
        }
        Ok(Self { conn })
    }
    pub fn register_project(&self, path: &Path) -> Result<Project> {
        let root = path.canonicalize().context("project path does not exist")?;
        if !root.is_dir() {
            bail!("project must be a directory");
        }
        let result = std::process::Command::new("git")
            .args(["-C"])
            .arg(&root)
            .args(["rev-parse", "--show-toplevel"])
            .output()?;
        if !result.status.success() {
            bail!("project must be a Git repository");
        }
        let git_root =
            std::path::PathBuf::from(String::from_utf8(result.stdout)?.trim()).canonicalize()?;
        if git_root != root {
            bail!("register the repository root: {}", git_root.display());
        }
        self.conn.execute(
            "UPDATE projects SET registered=1 WHERE root=?1",
            [root.to_string_lossy().as_ref()],
        )?;
        if let Some(p) = self.projects()?.into_iter().find(|p| p.root == root) {
            return Ok(p);
        }
        let project = Project {
            id: ProjectId::default(),
            name: root
                .file_name()
                .context("missing directory name")?
                .to_string_lossy()
                .into(),
            root,
        };
        self.conn.execute(
            "INSERT INTO projects(id,name,root) VALUES (?1,?2,?3)",
            params![
                project.id.to_string(),
                project.name,
                project.root.to_string_lossy()
            ],
        )?;
        Ok(project)
    }
    pub fn projects(&self) -> Result<Vec<Project>> {
        let mut q = self
            .conn
            .prepare("SELECT id,name,root FROM projects WHERE registered=1 ORDER BY name")?;
        let rows = q.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        rows.map(|r| {
            let (id, name, root) = r?;
            Ok(Project {
                id: ProjectId(id.parse()?),
                name,
                root: root.into(),
            })
        })
        .collect()
    }
    pub fn rename_project(&self, id: ProjectId, name: &str) -> Result<()> {
        let name=name.trim();
        if name.is_empty() || name.chars().count()>120 { bail!("プロジェクト名は1〜120文字で入力してください。"); }
        if self.conn.execute("UPDATE projects SET name=?2 WHERE id=?1 AND registered=1",params![id.to_string(),name])? != 1 { bail!("プロジェクトが見つかりません。"); }
        Ok(())
    }
    pub fn archive_project_threads(&mut self, id: ProjectId) -> Result<()> {
        let ids=self.threads()?.into_iter().filter(|t|t.project_id==id).map(|t|t.id).collect::<Vec<_>>();
        self.check_threads_idle(&ids)?;
        if self.threads()?.iter().any(|t|t.project_id==id && matches!(t.status.as_str(),"queued"|"integrating"|"stopping")) { bail!("実行完了後に操作してください。"); }
        let tx=self.conn.transaction()?;
        tx.execute("UPDATE provider_threads SET archived=1 WHERE project_id=?1",[id.to_string()])?;
        tx.commit()?; Ok(())
    }
    /// Removes only the app registration; repository files and task records remain intact.
    pub fn unregister_project(&self, id: ProjectId) -> Result<()> {
        let running: bool = self.conn.query_row("SELECT EXISTS(SELECT 1 FROM provider_threads WHERE project_id=?1 AND status IN ('running','inProgress','dispatching'))", [id.to_string()], |r| r.get(0))?;
        if running {
            bail!("実行中のセッションがあるため、プロジェクトを削除できません。");
        }
        if self.conn.execute(
            "UPDATE projects SET registered=0 WHERE id=?1 AND registered=1",
            [id.to_string()],
        )? != 1
        {
            bail!("プロジェクトが見つかりません。");
        }
        Ok(())
    }
    pub fn archived_threads(&self) -> Result<Vec<String>> {
        let mut q = self
            .conn
            .prepare("SELECT id FROM provider_threads WHERE archived=1")?;
        let rows = q.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
    pub fn set_thread_archived(&self, id: HubThreadId, archived: bool) -> Result<()> {
        self.check_thread_idle(id)?;
        self.conn.execute(
            "UPDATE provider_threads SET archived=?2 WHERE id=?1",
            params![id.to_string(), archived],
        )?;
        Ok(())
    }
    pub fn check_threads_idle(&self, ids: &[HubThreadId]) -> Result<()> {
        for id in ids {
            self.check_thread_idle(*id)?;
        }
        Ok(())
    }
    fn check_thread_idle(&self, id: HubThreadId) -> Result<()> {
        let status: String = self
            .conn
            .query_row(
                "SELECT status FROM provider_threads WHERE id=?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?
            .context("セッションが見つかりません。")?;
        let pending: bool = self.conn.query_row("SELECT EXISTS(SELECT 1 FROM turns WHERE thread_id=?1 AND status IN ('running','inProgress','dispatching'))", [id.to_string()], |r| r.get(0))?;
        if pending || matches!(status.as_str(), "running" | "inProgress" | "dispatching") {
            bail!("実行が終了してから操作してください。");
        }
        Ok(())
    }
    /// Delete this app's transcript and mapping, never the provider's remote conversation.
    pub fn delete_thread(&mut self, id: HubThreadId) -> Result<()> {
        self.check_thread_idle(id)?;
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM provider_usage_snapshots WHERE provider_thread_id=(SELECT provider_thread_id FROM provider_threads WHERE id=?1)", [id.to_string()])?;
        tx.execute("DELETE FROM events WHERE thread_id=?1", [id.to_string()])?;
        tx.execute("DELETE FROM turns WHERE thread_id=?1", [id.to_string()])?;
        for prefix in [
            "thread_model:",
            "thread_reasoning:",
            "chatgpt_messages:",
            "chatgpt_conversation:",
            "chatgpt_turn:",
            "workflow:",
        ] {
            tx.execute(
                "DELETE FROM settings WHERE key=?1",
                [format!("{prefix}{id}")],
            )?;
        }
        tx.execute("DELETE FROM provider_threads WHERE id=?1", [id.to_string()])?;
        tx.commit()?;
        Ok(())
    }
    pub fn save_thread(&self, t: &ThreadMapping) -> Result<()> {
        self.conn.execute("INSERT INTO provider_threads(id,project_id,provider,provider_thread_id,title,status) VALUES (?1,?2,?3,?4,?5,?6) ON CONFLICT(id) DO UPDATE SET status=excluded.status,updated_at=CURRENT_TIMESTAMP",params![t.id.to_string(),t.project_id.to_string(),t.provider,t.provider_thread_id,t.title,t.status])?;
        Ok(())
    }
    /// A user's Manifest is consumed exactly once, atomically with its workflow state.
    /// A restart after this commit must reconcile the task, never replay the clipboard.
    pub fn accept_manifest(
        &mut self,
        manifest_id: &str,
        manifest: &str,
        thread: &ThreadMapping,
        snapshot_key: &str,
        snapshot: &str,
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        let key = format!("manifest_receipt:{manifest_id}");
        if tx.query_row("SELECT 1 FROM settings WHERE key=?1", [&key], |_| Ok(())).optional()?.is_some() {
            bail!("このManifestは取り込み済みです。保存されたタスクを確認してください。");
        }
        tx.execute("INSERT INTO settings(key,value) VALUES (?1,?2)", params![key, manifest])?;
        tx.execute("INSERT INTO provider_threads(id,project_id,provider,provider_thread_id,title,status) VALUES (?1,?2,?3,?4,?5,?6) ON CONFLICT(id) DO UPDATE SET status=excluded.status,updated_at=CURRENT_TIMESTAMP", params![thread.id.to_string(),thread.project_id.to_string(),thread.provider,thread.provider_thread_id,thread.title,thread.status])?;
        tx.execute("INSERT INTO settings(key,value) VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![snapshot_key, snapshot])?;
        tx.commit()?;
        Ok(())
    }
    pub fn rename_thread(&self, id: HubThreadId, title: &str) -> Result<()> {
        let title = title.trim();
        if title.is_empty() || title.chars().count() > 120 || title.chars().any(char::is_control) {
            bail!("タスク名は改行を含まない1〜120文字で入力してください。");
        }
        if self.conn.execute(
            "UPDATE provider_threads SET title=?1 WHERE id=?2",
            params![title, id.to_string()],
        )? != 1
        {
            bail!("タスクが見つかりません。");
        }
        Ok(())
    }
    pub fn save_plan(&self, id: hub_core::PlanId, project: ProjectId, body: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO plans(id,project_id,body) VALUES (?1,?2,?3)",
            params![id.to_string(), project.to_string(), body],
        )?;
        Ok(())
    }
    pub fn save_task(&self, t: &hub_core::Task) -> Result<()> {
        self.conn.execute("INSERT INTO tasks(id,project_id,plan_id,status,body) VALUES (?1,?2,?3,?4,?5) ON CONFLICT(id) DO UPDATE SET status=excluded.status,body=excluded.body",params![t.id.to_string(),t.project_id.to_string(),t.plan_id.map(|id|id.to_string()),serde_json::to_string(&t.status)?,serde_json::to_string(t)?])?;
        Ok(())
    }
    pub fn tasks(&self, project: ProjectId) -> Result<Vec<hub_core::Task>> {
        let mut q = self
            .conn
            .prepare("SELECT body FROM tasks WHERE project_id=?1 ORDER BY rowid")?;
        let rows = q.query_map([project.to_string()], |r| r.get::<_, String>(0))?;
        rows.map(|r| Ok(serde_json::from_str(&r?)?)).collect()
    }
    pub fn save_dependencies(&self, t: &hub_core::Task) -> Result<()> {
        for dep in &t.dependencies {
            self.conn.execute(
                "INSERT OR IGNORE INTO task_dependencies VALUES (?1,?2)",
                params![t.id.to_string(), dep.to_string()],
            )?;
        }
        Ok(())
    }
    pub fn thread_task(&self, thread: HubThreadId) -> Result<Option<hub_core::TaskId>> {
        let id: Option<String> = self.conn.query_row(
            "SELECT task_id FROM provider_threads WHERE id=?1",
            [thread.to_string()],
            |r| r.get(0),
        )?;
        id.map(|id| Ok(hub_core::TaskId(id.parse()?))).transpose()
    }
    pub fn thread_root(&self, thread: HubThreadId) -> Result<std::path::PathBuf> {
        let root:String=self.conn.query_row("SELECT COALESCE(w.path,p.root) FROM provider_threads t JOIN projects p ON p.id=t.project_id LEFT JOIN worktrees w ON w.task_id=t.task_id WHERE t.id=?1",[thread.to_string()],|r|r.get(0))?;
        Ok(root.into())
    }
    pub fn save_worktree(&self, w: &hub_core::Worktree) -> Result<()> {
        self.conn.execute(
            "INSERT INTO worktrees(id,task_id,path,base_sha) VALUES (?1,?2,?3,?4)",
            params![
                w.id.to_string(),
                w.task_id.to_string(),
                w.path.to_string_lossy(),
                w.base_sha
            ],
        )?;
        Ok(())
    }
    pub fn worktree_for_task(&self, task: hub_core::TaskId) -> Result<Option<hub_core::Worktree>> {
        use rusqlite::OptionalExtension;
        let row = self
            .conn
            .query_row(
                "SELECT id,path,base_sha FROM worktrees WHERE task_id=?1",
                [task.to_string()],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?;
        row.map(|(id, path, base_sha)| {
            Ok(hub_core::Worktree {
                id: hub_core::WorktreeId(id.parse()?),
                task_id: task,
                path: path.into(),
                base_sha,
            })
        })
        .transpose()
    }
    pub fn bind_worker_thread(
        &self,
        thread: HubThreadId,
        task: hub_core::TaskId,
        worker: hub_core::WorkerId,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE provider_threads SET task_id=?2 WHERE id=?1",
            params![thread.to_string(), task.to_string()],
        )?;
        self.conn.execute(
            "INSERT INTO workers(id,task_id,status) VALUES (?1,?2,'running')",
            params![worker.to_string(), task.to_string()],
        )?;
        Ok(())
    }
    pub fn save_capsule(&self, c: &hub_core::ContextCapsule, initial: u32) -> Result<()> {
        self.conn.execute(
            "INSERT INTO context_capsules(id,task_id,body,initial_tokens) VALUES (?1,?2,?3,?4)",
            params![
                c.id.to_string(),
                c.task_id.to_string(),
                serde_json::to_string(c)?,
                initial
            ],
        )?;
        for item in &c.items {
            self.conn.execute("INSERT INTO context_capsule_items(id,capsule_id,source,reference,token_estimate) VALUES (?1,?2,?3,?4,?5)",params![hub_core::TaskId::default().to_string(),c.id.to_string(),item.source,item.reference,item.token_estimate])?;
        }
        Ok(())
    }
    pub fn capsule(
        &self,
        task: hub_core::TaskId,
    ) -> Result<Option<(hub_core::ContextCapsule, u32)>> {
        use rusqlite::OptionalExtension;
        let row = self
            .conn
            .query_row(
                "SELECT body,initial_tokens FROM context_capsules WHERE task_id=?1",
                [task.to_string()],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?)),
            )
            .optional()?;
        row.map(|(body, tokens)| Ok((serde_json::from_str(&body)?, tokens)))
            .transpose()
    }
    pub fn retrievals(&self, task: hub_core::TaskId) -> Result<Vec<hub_core::ContextRetrieval>> {
        let mut q=self.conn.prepare("SELECT id,source,query,token_budget,token_estimate,result_ref FROM context_retrievals WHERE task_id=?1 ORDER BY rowid")?;
        let rows = q.query_map([task.to_string()], |r| {
            Ok(hub_core::ContextRetrieval {
                id: r.get(0)?,
                task_id: task,
                source: r.get(1)?,
                query: r.get(2)?,
                token_budget: r.get(3)?,
                token_estimate: r.get(4)?,
                result_ref: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
    pub fn record_retrieval_denial(&self, r: &hub_core::ContextRetrieval) -> Result<()> {
        self.conn.execute("INSERT INTO context_retrievals(id,task_id,source,query,token_budget,token_estimate,result_ref) VALUES (?1,?2,?3,?4,?5,0,?6)",params![r.id,r.task_id.to_string(),r.source,r.query,r.token_budget,r.result_ref])?;
        Ok(())
    }
    pub fn commit_retrieval(&mut self, r: &hub_core::ContextRetrieval, result: &str) -> Result<()> {
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (body, initial): (String, u32) = tx.query_row(
            "SELECT body,initial_tokens FROM context_capsules WHERE task_id=?1",
            [r.task_id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let capsule: hub_core::ContextCapsule = serde_json::from_str(&body)?;
        let used: u64 = tx.query_row(
            "SELECT COALESCE(SUM(token_estimate),0) FROM context_retrievals WHERE task_id=?1",
            [r.task_id.to_string()],
            |row| row.get(0),
        )?;
        if r.token_budget > capsule.budget.max_single_retrieval_tokens
            || r.token_estimate > r.token_budget
            || u64::from(initial) + used + u64::from(r.token_estimate)
                > u64::from(capsule.budget.max_total_tokens)
        {
            bail!("context retrieval budget exhausted");
        }
        tx.execute("INSERT INTO context_retrievals(id,task_id,source,query,token_budget,token_estimate,result_ref,result_json) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",params![r.id,r.task_id.to_string(),r.source,r.query,r.token_budget,r.token_estimate,r.result_ref,result])?;
        tx.commit()?;
        Ok(())
    }
    pub fn store_decision(
        &self,
        project: ProjectId,
        id: &str,
        statement: &str,
        reason: &str,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO decisions(id,project_id,statement,reason) VALUES (?1,?2,?3,?4)",
            params![id, project.to_string(), statement, reason],
        )?;
        Ok(())
    }
    pub fn search_decisions(
        &self,
        project: ProjectId,
        query: &str,
    ) -> Result<Vec<(String, String)>> {
        let mut q=self.conn.prepare("SELECT id,statement||char(10)||reason FROM decisions WHERE project_id=?1 AND (instr(lower(statement),lower(?2))>0 OR instr(lower(reason),lower(?2))>0) LIMIT 8")?;
        let rows = q.query_map(params![project.to_string(), query], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
    pub fn search_session(&self, project: ProjectId, query: &str) -> Result<Vec<(String, String)>> {
        let mut q=self.conn.prepare("SELECT cast(e.id as text),json_extract(e.body,'$.text') FROM events e JOIN provider_threads t ON e.thread_id=t.id WHERE t.project_id=?1 AND e.kind IN ('message_completed','summary','review') AND instr(lower(json_extract(e.body,'$.text')),lower(?2))>0 ORDER BY e.id DESC LIMIT 8")?;
        let rows = q.query_map(params![project.to_string(), query], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
    pub fn record_codex_usage(
        &mut self,
        thread: &str,
        turn: &str,
        details: &protocol_types::EventDetails,
    ) -> Result<()> {
        if let protocol_types::EventDetails::Usage {
            model,
            last_input_tokens,
            last_cached_tokens,
            last_output_tokens,
            total_input_tokens,
            ..
        } = details
        {
            let tx = self.conn.transaction()?;
            tx.execute("INSERT INTO provider_usage_snapshots(id,provider_thread_id,turn_id,total_input_tokens,input_tokens,cached_tokens,output_tokens) VALUES (?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(provider_thread_id,total_input_tokens) DO UPDATE SET output_tokens=MAX(output_tokens,excluded.output_tokens)",params![hub_core::TaskId::default().to_string(),thread,turn,total_input_tokens,last_input_tokens,last_cached_tokens,last_output_tokens])?;
            tx.execute("INSERT INTO model_usage(id,provider,model,task_id,turn_id,prompt_tokens,cached_tokens,completion_tokens) SELECT ?1,'codex',?2,(SELECT task_id FROM provider_threads WHERE provider_thread_id=?3),?4,SUM(input_tokens),SUM(cached_tokens),SUM(output_tokens) FROM provider_usage_snapshots WHERE provider_thread_id=?3 AND turn_id=?4 HAVING NOT EXISTS (SELECT 1 FROM model_usage WHERE provider='codex' AND turn_id=?4)",params![hub_core::TaskId::default().to_string(),model.as_deref().unwrap_or("Codex (model not reported)"),thread,turn])?;
            tx.execute("UPDATE model_usage SET prompt_tokens=(SELECT SUM(input_tokens) FROM provider_usage_snapshots WHERE provider_thread_id=?1 AND turn_id=?2),cached_tokens=(SELECT SUM(cached_tokens) FROM provider_usage_snapshots WHERE provider_thread_id=?1 AND turn_id=?2),completion_tokens=(SELECT SUM(output_tokens) FROM provider_usage_snapshots WHERE provider_thread_id=?1 AND turn_id=?2) WHERE provider='codex' AND turn_id=?2",params![thread,turn])?;
            tx.commit()?;
        }
        Ok(())
    }
    pub fn record_usage(&self, u: &hub_core::ModelUsageRecord) -> Result<()> {
        self.conn.execute("INSERT INTO model_usage(id,provider,model,task_id,turn_id,prompt_tokens,cached_tokens,completion_tokens,estimated_cost,latency_ms) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![u.id,u.provider,u.model,u.task_id.map(|id|id.to_string()),u.turn_id,u.prompt_tokens,u.cached_tokens,u.completion_tokens,u.estimated_cost,u.latency_ms])?;
        Ok(())
    }
    pub fn usage(&self) -> Result<Vec<hub_core::ModelUsageRecord>> {
        self.scoped_usage(None)
    }
    pub fn thread_usage(&self, id: HubThreadId) -> Result<Vec<hub_core::ModelUsageRecord>> {
        self.scoped_usage(Some(id))
    }
    fn scoped_usage(&self, thread: Option<HubThreadId>) -> Result<Vec<hub_core::ModelUsageRecord>> {
        let mut q=self.conn.prepare("SELECT u.id,u.provider,u.model,u.turn_id,u.prompt_tokens,u.cached_tokens,u.completion_tokens,u.estimated_cost,
          COALESCE(u.latency_ms,(SELECT CAST(ROUND((julianday(MIN(CASE WHEN e.kind='turn_completed' THEN e.created_at END))-julianday(MIN(CASE WHEN e.kind='turn_started' THEN e.created_at END)))*86400000) AS INTEGER) FROM events e WHERE json_extract(e.body,'$.turn_id')=u.turn_id AND u.provider='codex')),u.task_id
          FROM model_usage u WHERE ?1 IS NULL OR EXISTS (SELECT 1 FROM provider_threads t WHERE t.id=?1 AND ((u.turn_id IS NULL AND u.task_id IS NOT NULL AND u.task_id=t.task_id AND (SELECT COUNT(*) FROM provider_threads linked WHERE linked.task_id=u.task_id)=1) OR EXISTS (SELECT 1 FROM turns x WHERE x.thread_id=t.id AND x.provider_turn_id=u.turn_id) OR EXISTS (SELECT 1 FROM events e WHERE e.thread_id=t.id AND json_extract(e.body,'$.turn_id')=u.turn_id) OR EXISTS (SELECT 1 FROM provider_usage_snapshots p WHERE p.provider_thread_id=t.provider_thread_id AND p.turn_id=u.turn_id))) ORDER BY u.created_at DESC")?;
        let rows = q.query_map([thread.map(|id|id.to_string())], |r| {
            Ok(hub_core::ModelUsageRecord {
                id: r.get(0)?,
                provider: r.get(1)?,
                model: r.get(2)?,
                task_id: r
                    .get::<_, Option<String>>(9)?
                    .map(|s| s.parse().map(hub_core::TaskId))
                    .transpose()
                    .map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            9,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })?,
                turn_id: r.get(3)?,
                prompt_tokens: r.get(4)?,
                cached_tokens: r.get(5)?,
                completion_tokens: r.get(6)?,
                estimated_cost: r.get(7)?,
                latency_ms: r.get(8)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
    pub fn task_control_evidence(&self, task: hub_core::TaskId) -> Result<Vec<String>> {
        let mut q=self.conn.prepare("SELECT body FROM events WHERE kind IN ('worktree_created','dependency_applied') AND json_extract(json_extract(body,'$.text'),'$.task_id')=?1 ORDER BY id")?;
        let rows = q.query_map([task.to_string()], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
    pub fn review_evidence(&self, thread: HubThreadId) -> Result<Vec<String>> {
        let mut q=self.conn.prepare("SELECT body FROM events WHERE thread_id=?1 AND kind IN ('item_completed','turn_completed','error') ORDER BY id DESC LIMIT 16")?;
        let rows = q.query_map([thread.to_string()], |r| r.get(0))?;
        let mut result = rows.collect::<rusqlite::Result<Vec<String>>>()?;
        let mut q=self.conn.prepare("SELECT id,body FROM events WHERE thread_id=?1 AND kind IN ('item_started','item_completed') AND (json_extract(body,'$.details.type')='tool' OR json_extract(body,'$.text')='fileChange') ORDER BY id LIMIT 24")?;
        for row in q.query_map([thread.to_string()], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })? {
            let (seq, body) = row?;
            result.push(format!("Chronology sequence {seq}: {body}"));
        }
        Ok(result)
    }
    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        use rusqlite::OptionalExtension;
        Ok(self
            .conn
            .query_row("SELECT value FROM settings WHERE key=?1", [key], |r| {
                r.get(0)
            })
            .optional()?)
    }
    pub fn remove_setting(&self, key: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM settings WHERE key=?1", [key])?;
        Ok(())
    }
    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute("INSERT INTO settings(key,value) VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key,value])?;
        Ok(())
    }
    pub fn append_event(
        &self,
        provider_thread_id: Option<&str>,
        kind: &str,
        body: &str,
    ) -> Result<i64> {
        self.conn.execute("INSERT INTO events(thread_id,kind,body,created_at) VALUES ((SELECT id FROM provider_threads WHERE provider_thread_id=?1),?2,?3,strftime('%Y-%m-%d %H:%M:%f','now'))",params![provider_thread_id,kind,body])?;
        Ok(self.conn.last_insert_rowid())
    }
    pub fn event_history(&self, id: HubThreadId, after: i64) -> Result<Vec<(i64, String)>> {
        let mut q=self.conn.prepare("SELECT e.id,e.body FROM events e WHERE e.id>?2 AND (e.thread_id=?1 OR json_extract(e.body,'$.thread_id')=(SELECT provider_thread_id FROM provider_threads WHERE id=?1)) AND e.kind!='command_intent' ORDER BY e.id LIMIT 2000")?;
        let rows = q.query_map(params![id.to_string(), after], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
    /// Bounded, durable activity for a worker's current turn. Streaming deltas
    /// are omitted by the journal; use the live event bus for those instead.
    pub fn recent_activity(&self, id: HubThreadId, turn: Option<&str>) -> Result<Vec<(i64, String)>> {
        let mut q = self.conn.prepare("SELECT id,body FROM events WHERE thread_id=?1 AND (?2 IS NULL OR json_extract(body,'$.turn_id')=?2) AND kind IN ('message_completed','item_started','item_completed','error','turn_started','turn_completed','diff') ORDER BY id DESC LIMIT 60")?;
        let rows = q.query_map(params![id.to_string(), turn], |r| Ok((r.get(0)?, r.get(1)?)))?;
        let mut events = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        events.reverse();
        Ok(events)
    }
    pub fn update_provider_status(
        &self,
        provider_thread: &str,
        status: &str,
        turn: Option<&str>,
    ) -> Result<()> {
        if let Some(turn) = turn {
            // A delayed start cannot reopen a completed turn; a delayed completion
            // cannot replace the state of a newer turn on the same thread.
            let completed: bool = self.conn.query_row("SELECT EXISTS(SELECT 1 FROM events WHERE kind='turn_completed' AND json_extract(body,'$.turn_id')=?1)",[turn],|r|r.get(0))?;
            if status == "inProgress" && completed {
                return Ok(());
            }
            self.conn.execute("UPDATE turns SET status=?2 WHERE provider_turn_id=?1 AND (status NOT IN ('completed','failed','interrupted') OR ?2!='inProgress')",params![turn,status])?;
            let current: Option<String> = self.conn.query_row("SELECT json_extract(body,'$.turn_id') FROM events WHERE thread_id=(SELECT id FROM provider_threads WHERE provider_thread_id=?1) AND kind='turn_started' ORDER BY id DESC LIMIT 1",[provider_thread],|r|r.get(0)).optional()?;
            if current.as_deref().is_some_and(|id| id != turn) {
                return Ok(());
            }
        }
        self.conn.execute("UPDATE provider_threads SET status=?2,updated_at=CURRENT_TIMESTAMP WHERE provider_thread_id=?1",params![provider_thread,status])?;
        Ok(())
    }
    pub fn record_turn_intent(&self, thread: HubThreadId) -> Result<String> {
        let id = hub_core::TaskId::default().to_string();
        self.conn.execute("INSERT INTO turns(id,thread_id,status,intent) VALUES (?1,?2,'dispatching','turn/start')", params![id,thread.to_string()])?;
        Ok(id)
    }
    pub fn resolve_turn_intent(
        &self,
        id: &str,
        provider_turn: Option<&str>,
        status: &str,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE turns SET provider_turn_id=?2,status=COALESCE((SELECT json_extract(body,'$.text') FROM events WHERE kind='turn_completed' AND json_extract(body,'$.turn_id')=?2 ORDER BY id DESC LIMIT 1),?3) WHERE id=?1",
            params![id, provider_turn, status],
        )?;
        Ok(())
    }
    pub fn record_command(&self, thread: HubThreadId, command: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO events(thread_id,kind,body) VALUES (?1,'command_intent',?2)",
            params![thread.to_string(), serde_json::to_string(command)?],
        )?;
        Ok(())
    }
    pub fn threads(&self) -> Result<Vec<ThreadMapping>> {
        let mut q=self.conn.prepare("SELECT id,project_id,provider,provider_thread_id,title,status FROM provider_threads ORDER BY created_at DESC")?;
        let rows = q.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        })?;
        rows.map(|r| {
            let (id, project, provider, provider_thread_id, title, status) = r?;
            Ok(ThreadMapping {
                id: HubThreadId(id.parse()?),
                project_id: ProjectId(project.parse()?),
                provider,
                provider_thread_id,
                title,
                status,
            })
        })
        .collect()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn usage_is_scoped_before_reading_and_elapsed_time_is_derived() -> Result<()> {
        let dir=tempfile::tempdir()?;
        assert!(std::process::Command::new("git").arg("init").arg(dir.path()).output()?.status.success());
        let store=Store::open(&dir.path().join("usage.db"))?;
        let project=store.register_project(dir.path())?;
        let mut ids=Vec::new();
        for n in 0..2 {
            let thread=ThreadMapping{id:HubThreadId::default(),project_id:project.id,provider:"codex".into(),provider_thread_id:format!("p{n}"),title:"test".into(),status:"completed".into()};
            store.save_thread(&thread)?;
            store.record_usage(&hub_core::ModelUsageRecord{id:format!("usage{n}"),provider:"codex".into(),model:"sol".into(),task_id:None,turn_id:Some(format!("turn{n}")),prompt_tokens:Some(10),cached_tokens:None,completion_tokens:Some(2),estimated_cost:None,latency_ms:None})?;
            for (kind,time) in [("turn_started","2026-09-09 01:00:00.000"),("turn_completed","2026-09-09 01:00:02.500")] {
                let seq=store.append_event(Some(&thread.provider_thread_id),kind,&serde_json::json!({"turn_id":format!("turn{n}")}).to_string())?;
                store.conn.execute("UPDATE events SET created_at=?2 WHERE id=?1",params![seq,time])?;
            }
            ids.push(thread.id);
        }
        assert_eq!(store.usage()?.len(),2);
        for (n,id) in ids.into_iter().enumerate() {let rows=store.thread_usage(id)?;assert_eq!(rows.len(),1);assert_eq!(rows[0].id,format!("usage{n}"));assert_eq!(rows[0].latency_ms,Some(2500));}
        assert!(store.thread_usage(HubThreadId::default())?.is_empty());
        Ok(())
    }
    #[test]
    fn archive_delete_and_unregister_preserve_workspace() -> Result<()> {
        let dir = tempfile::tempdir()?;
        assert!(std::process::Command::new("git")
            .args(["init", "-q"])
            .arg(dir.path())
            .status()?
            .success());
        std::fs::write(dir.path().join("keep.txt"), "preserved")?;
        let mut store = Store::open(&dir.path().join("hub.db"))?;
        let project = store.register_project(dir.path())?;
        let mut thread = ThreadMapping {
            id: HubThreadId::default(),
            project_id: project.id,
            provider: "chatgpt".into(),
            provider_thread_id: "test-browser".into(),
            title: "test".into(),
            status: "running".into(),
        };
        store.save_thread(&thread)?;
        assert!(store.unregister_project(project.id).is_err());
        assert!(store.delete_thread(thread.id).is_err());
        assert!(store.set_thread_archived(thread.id, true).is_err());
        thread.status = "completed".into();
        store.save_thread(&thread)?;
        store.set_thread_archived(thread.id, true)?;
        assert_eq!(store.archived_threads()?, vec![thread.id.to_string()]);
        store.set_thread_archived(thread.id, false)?;
        assert!(store.archived_threads()?.is_empty());
        store.unregister_project(project.id)?;
        assert!(store.projects()?.is_empty());
        assert_eq!(store.register_project(dir.path())?.id, project.id);
        assert_eq!(store.threads()?.len(), 1);
        let key = format!("chatgpt_messages:{}", thread.id);
        store.set_setting(&key, "private transcript")?;
        store.delete_thread(thread.id)?;
        assert!(store.threads()?.is_empty());
        assert_eq!(store.setting(&key)?, None);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("keep.txt"))?,
            "preserved"
        );
        Ok(())
    }
    #[test]
    fn migration_is_idempotent_and_persists() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let p = dir.path().join("hub.db");
        {
            let s = Store::open(&p)?;
            s.conn
                .execute("INSERT INTO settings VALUES ('test','saved')", [])?;
        }
        let s = Store::open(&p)?;
        assert_eq!(
            s.conn
                .query_row("SELECT value FROM settings WHERE key='test'", [], |r| {
                    r.get::<_, String>(0)
                })?,
            "saved"
        );
        Ok(())
    }
    #[test]
    fn late_events_and_duplicate_usage_do_not_regress_state() -> Result<()> {
        let d = tempfile::tempdir()?;
        let mut s = Store::open(&d.path().join("hub.db"))?;
        let project = ProjectId::default();
        s.conn.execute(
            "INSERT INTO projects(id,name,root) VALUES (?1,'fixture','/fixture')",
            [project.to_string()],
        )?;
        let t = ThreadMapping {
            id: HubThreadId::default(),
            project_id: project,
            provider: "codex".into(),
            provider_thread_id: "remote".into(),
            title: "fixture".into(),
            status: "idle".into(),
        };
        s.save_thread(&t)?;
        for (kind, turn, status) in [
            ("turn_started", "a", "inProgress"),
            ("turn_completed", "a", "completed"),
            ("turn_started", "b", "inProgress"),
            ("turn_completed", "a", "completed"),
        ] {
            s.append_event(
                Some("remote"),
                kind,
                &serde_json::json!({"turn_id":turn,"text":status}).to_string(),
            )?;
            s.update_provider_status("remote", status, Some(turn))?;
        }
        assert_eq!(s.threads()?[0].status, "inProgress");
        s.rename_thread(t.id, "  確認済みタスク  ")?;
        s.save_thread(&t)?; // Provider lifecycle updates must preserve the operator's title.
        assert_eq!(s.threads()?[0].title, "確認済みタスク");
        assert!(s.rename_thread(t.id, "\n").is_err());
        assert_eq!(s.threads()?[0].title, "確認済みタスク");

        let details = protocol_types::EventDetails::Usage {
            model: Some("fixture".into()),
            last_input_tokens: 100,
            last_cached_tokens: 20,
            last_output_tokens: 5,
            total_input_tokens: 100,
            total_cached_tokens: 20,
            total_output_tokens: 5,
        };
        s.record_codex_usage("remote", "b", &details)?;
        s.record_codex_usage("remote", "b", &details)?;
        assert_eq!(s.usage()?[0].prompt_tokens, Some(100));
        Ok(())
    }
    #[test]
    fn rejects_future_database() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let p = dir.path().join("hub.db");
        Connection::open(&p)?.execute_batch("PRAGMA user_version=99")?;
        assert!(Store::open(&p).is_err());
        Ok(())
    }
}
