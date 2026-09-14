use anyhow::{bail, Context, Result};
use hub_core::{HubThreadId, Project, ProjectId, ThreadMapping};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderSegmentRecord {
    pub id: String,
    pub thread_id: HubThreadId,
    pub target: protocol_types::providers::ModelTarget,
    pub profile_revision: u64,
    pub provider_thread_id: Option<String>,
    pub ended: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnTargetRecord {
    pub turn_id: String,
    pub thread_id: HubThreadId,
    pub segment_id: String,
    pub target: protocol_types::providers::ModelTarget,
    pub profile_revision: u64,
    pub resolved_from: protocol_types::providers::TargetResolution,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderMessageRecord {
    pub id: String,
    pub thread_id: HubThreadId,
    pub segment_id: Option<String>,
    pub provider_turn_id: Option<String>,
    pub role: protocol_types::providers::TranscriptRole,
    pub content: Vec<protocol_types::providers::ContentBlock>,
    pub provider_state: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderTurnOutcomeRecord {
    pub turn_id: String,
    pub thread_id: HubThreadId,
    pub status: String,
    pub stop_reason: Option<String>,
    pub failure_kind: Option<String>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolApprovalRecord {
    pub id: String,
    pub thread_id: HubThreadId,
    pub turn_id: String,
    pub digest: String,
    pub request: Value,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReviewRunRecord {
    pub id: String,
    pub task_id: hub_core::TaskId,
    pub executor_thread_id: HubThreadId,
    pub reviewer_thread_id: HubThreadId,
    pub artifact_hash: String,
    pub verdict: String,
    pub body: Value,
}

pub struct Store {
    conn: Connection,
}
impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version > 6 {
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
        if version < 5 {
            conn.execute_batch(concat!(
                "BEGIN;",
                include_str!("../../../migrations/005_providers.sql"),
                "COMMIT;"
            ))?;
        }
        if version < 6 {
            conn.execute_batch(concat!(
                "BEGIN;",
                include_str!("../../../migrations/006_provider_outcomes.sql"),
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
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 120 {
            bail!("プロジェクト名は1〜120文字で入力してください。");
        }
        if self.conn.execute(
            "UPDATE projects SET name=?2 WHERE id=?1 AND registered=1",
            params![id.to_string(), name],
        )? != 1
        {
            bail!("プロジェクトが見つかりません。");
        }
        Ok(())
    }
    pub fn archive_project_threads(&mut self, id: ProjectId) -> Result<()> {
        let ids = self
            .threads()?
            .into_iter()
            .filter(|t| t.project_id == id)
            .map(|t| t.id)
            .collect::<Vec<_>>();
        self.check_threads_idle(&ids)?;
        if self.threads()?.iter().any(|t| {
            t.project_id == id && matches!(t.status.as_str(), "queued" | "integrating" | "stopping")
        }) {
            bail!("実行完了後に操作してください。");
        }
        let tx = self.conn.transaction()?;
        tx.execute(
            "UPDATE provider_threads SET archived=1 WHERE project_id=?1",
            [id.to_string()],
        )?;
        tx.commit()?;
        Ok(())
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
            "thread_mode:",
            "chatgpt_messages:",
            "chatgpt_conversation:",
            "chatgpt_turn:",
            "workflow:",
            "autonomous:",
            "autonomous_parent:",
            "outcome:",
            "review:",
            "review_scope:",
            "recovery:",
            "recovery_consumed:",
            "memory_candidates:",
            "memory_candidate_scope:",
            "memory_saved:",
            "api_transcript:",
            "api_active_turn:",
            "codex_context_capsule:",
            "codex_context_capsule_consumed:",
            "codex_context_replay:",
            "codex_context_replay_consumed:",
            "codex_archive_synced:",
            "context_metrics:",
        ] {
            let key = format!("{prefix}{id}");
            tx.execute(
                "DELETE FROM settings WHERE key=?1 OR key LIKE ?2",
                params![key, format!("{prefix}{id}:%")],
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
    pub fn switch_thread_provider(
        &self,
        id: HubThreadId,
        provider: &str,
        provider_thread_id: &str,
    ) -> Result<()> {
        self.check_thread_idle(id)?;
        anyhow::ensure!(
            !provider.trim().is_empty() && !provider_thread_id.trim().is_empty(),
            "provider mapping cannot be empty"
        );
        if self.conn.execute(
            "UPDATE provider_threads SET provider=?2,provider_thread_id=?3,status='idle',updated_at=CURRENT_TIMESTAMP WHERE id=?1",
            params![id.to_string(), provider, provider_thread_id],
        )? != 1 {
            bail!("thread mapping not found");
        }
        Ok(())
    }
    pub fn provider_profiles(&self) -> Result<Vec<protocol_types::providers::ProviderProfile>> {
        use protocol_types::providers::{ProviderLocality, ProviderProfile, ProviderProtocol};
        let mut query = self.conn.prepare("SELECT id,name,protocol,base_url,locality,credential_env,max_concurrency,enabled,revision FROM provider_profiles ORDER BY name,id")?;
        let rows = query.query_map([], |row| {
            let protocol = match row.get::<_, String>(2)?.as_str() {
                "codex_app_server" => ProviderProtocol::CodexAppServer,
                "local_dedicated" => ProviderProtocol::LocalDedicated,
                "open_ai_chat" | "openai_chat" => ProviderProtocol::OpenAiChat,
                "open_ai_responses" | "openai_responses" => ProviderProtocol::OpenAiResponses,
                "anthropic_messages" => ProviderProtocol::AnthropicMessages,
                value => {
                    return Err(rusqlite::Error::FromSqlConversionFailure(
                        2,
                        rusqlite::types::Type::Text,
                        format!("unknown provider protocol: {value}").into(),
                    ))
                }
            };
            let locality = match row.get::<_, String>(4)?.as_str() {
                "local" => ProviderLocality::Local,
                "cloud" => ProviderLocality::Cloud,
                value => {
                    return Err(rusqlite::Error::FromSqlConversionFailure(
                        4,
                        rusqlite::types::Type::Text,
                        format!("unknown provider locality: {value}").into(),
                    ))
                }
            };
            Ok(ProviderProfile {
                id: row.get(0)?,
                name: row.get(1)?,
                protocol,
                base_url: row.get(3)?,
                locality,
                credential_env: row.get(5)?,
                max_concurrency: row.get(6)?,
                enabled: row.get(7)?,
                revision: row.get(8)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
    pub fn provider_profile(
        &self,
        id: &str,
    ) -> Result<Option<protocol_types::providers::ProviderProfile>> {
        Ok(self
            .provider_profiles()?
            .into_iter()
            .find(|profile| profile.id == id))
    }
    /// Creates revision 1 or replaces exactly the immediately preceding revision.
    pub fn save_provider_profile(
        &self,
        profile: &protocol_types::providers::ProviderProfile,
    ) -> Result<()> {
        use protocol_types::providers::{ProviderLocality, ProviderProtocol};
        profile.validate()?;
        let current = self
            .conn
            .query_row(
                "SELECT revision FROM provider_profiles WHERE id=?1",
                [&profile.id],
                |row| row.get::<_, u64>(0),
            )
            .optional()?;
        match current {
            None if profile.revision != 1 => bail!("new provider profile must start at revision 1"),
            Some(revision) if profile.revision != revision + 1 => {
                bail!("provider profile was modified; reload before saving")
            }
            _ => {}
        }
        let protocol = match profile.protocol {
            ProviderProtocol::CodexAppServer => "codex_app_server",
            ProviderProtocol::LocalDedicated => "local_dedicated",
            ProviderProtocol::OpenAiChat => "openai_chat",
            ProviderProtocol::OpenAiResponses => "openai_responses",
            ProviderProtocol::AnthropicMessages => "anthropic_messages",
        };
        let locality = match profile.locality {
            ProviderLocality::Local => "local",
            ProviderLocality::Cloud => "cloud",
        };
        self.conn.execute("INSERT INTO provider_profiles(id,name,protocol,base_url,locality,credential_env,max_concurrency,enabled,revision) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9) ON CONFLICT(id) DO UPDATE SET name=excluded.name,protocol=excluded.protocol,base_url=excluded.base_url,locality=excluded.locality,credential_env=excluded.credential_env,max_concurrency=excluded.max_concurrency,enabled=excluded.enabled,revision=excluded.revision,updated_at=CURRENT_TIMESTAMP", params![profile.id,profile.name,protocol,profile.base_url,locality,profile.credential_env,profile.max_concurrency,profile.enabled,profile.revision])?;
        Ok(())
    }
    pub fn set_session_model_policy(
        &self,
        thread: HubThreadId,
        policy: &protocol_types::providers::SessionModelPolicy,
    ) -> Result<()> {
        policy.validate()?;
        self.conn.execute("INSERT INTO session_model_policies(thread_id,body) VALUES (?1,?2) ON CONFLICT(thread_id) DO UPDATE SET body=excluded.body,updated_at=CURRENT_TIMESTAMP", params![thread.to_string(),serde_json::to_string(policy)?])?;
        Ok(())
    }
    pub fn session_model_policy(
        &self,
        thread: HubThreadId,
    ) -> Result<Option<protocol_types::providers::SessionModelPolicy>> {
        self.conn
            .query_row(
                "SELECT body FROM session_model_policies WHERE thread_id=?1",
                [thread.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .map(|body| Ok(serde_json::from_str(&body)?))
            .transpose()
    }
    pub fn start_provider_segment(&self, record: &ProviderSegmentRecord) -> Result<()> {
        record.target.validate()?;
        anyhow::ensure!(
            !record.ended,
            "new provider segment cannot already be ended"
        );
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("UPDATE provider_segments SET ended_at=CURRENT_TIMESTAMP WHERE thread_id=?1 AND ended_at IS NULL", [record.thread_id.to_string()])?;
        tx.execute("INSERT INTO provider_segments(id,thread_id,profile_id,profile_revision,model_id,effort,provider_thread_id) VALUES (?1,?2,?3,?4,?5,?6,?7)", params![record.id,record.thread_id.to_string(),record.target.profile_id,record.profile_revision,record.target.model_id,record.target.effort,record.provider_thread_id])?;
        tx.commit()?;
        Ok(())
    }
    pub fn active_provider_segment(
        &self,
        thread: HubThreadId,
    ) -> Result<Option<ProviderSegmentRecord>> {
        self.conn.query_row("SELECT id,profile_id,profile_revision,model_id,effort,provider_thread_id FROM provider_segments WHERE thread_id=?1 AND ended_at IS NULL ORDER BY started_at DESC,rowid DESC LIMIT 1", [thread.to_string()], |row| Ok(ProviderSegmentRecord { id: row.get(0)?, thread_id: thread, target: protocol_types::providers::ModelTarget { profile_id: row.get(1)?, model_id: row.get(3)?, effort: row.get(4)? }, profile_revision: row.get(2)?, provider_thread_id: row.get(5)?, ended: false })).optional().map_err(Into::into)
    }
    pub fn record_turn_target(&self, record: &TurnTargetRecord) -> Result<()> {
        record.target.validate()?;
        let resolved = serde_json::to_value(&record.resolved_from)?
            .as_str()
            .context("invalid target resolution")?
            .to_owned();
        self.conn.execute("INSERT INTO turn_model_targets(turn_id,thread_id,segment_id,profile_id,profile_revision,model_id,effort,resolved_from) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)", params![record.turn_id,record.thread_id.to_string(),record.segment_id,record.target.profile_id,record.profile_revision,record.target.model_id,record.target.effort,resolved])?;
        Ok(())
    }
    pub fn turn_target(&self, turn_id: &str) -> Result<Option<TurnTargetRecord>> {
        type TurnTargetRow = (String, String, String, u64, String, Option<String>, String);
        let row: Option<TurnTargetRow> = self.conn.query_row("SELECT thread_id,segment_id,profile_id,profile_revision,model_id,effort,resolved_from FROM turn_model_targets WHERE turn_id=?1", [turn_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?))).optional()?;
        row.map(
            |(thread, segment, profile, revision, model, effort, resolved)| {
                Ok(TurnTargetRecord {
                    turn_id: turn_id.into(),
                    thread_id: HubThreadId(thread.parse()?),
                    segment_id: segment,
                    target: protocol_types::providers::ModelTarget {
                        profile_id: profile,
                        model_id: model,
                        effort,
                    },
                    profile_revision: revision,
                    resolved_from: serde_json::from_value(serde_json::Value::String(resolved))?,
                })
            },
        )
        .transpose()
    }
    pub fn append_provider_message(&self, record: &ProviderMessageRecord) -> Result<()> {
        anyhow::ensure!(!record.id.trim().is_empty(), "provider message ID is empty");
        let role = serde_json::to_value(record.role)?
            .as_str()
            .context("invalid provider message role")?
            .to_owned();
        self.conn.execute(
            "INSERT INTO provider_messages(id,thread_id,segment_id,provider_turn_id,role,body,provider_state) VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![
                record.id,
                record.thread_id.to_string(),
                record.segment_id,
                record.provider_turn_id,
                role,
                serde_json::to_string(&record.content)?,
                record.provider_state.as_ref().map(serde_json::to_string).transpose()?,
            ],
        )?;
        Ok(())
    }
    pub fn provider_messages(&self, thread: HubThreadId) -> Result<Vec<ProviderMessageRecord>> {
        let mut query = self.conn.prepare(
            "SELECT id,segment_id,provider_turn_id,role,body,provider_state FROM provider_messages WHERE thread_id=?1 ORDER BY created_at,rowid",
        )?;
        let rows = query.query_map([thread.to_string()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })?;
        rows.map(|row| {
            let (id, segment_id, provider_turn_id, role, body, provider_state) = row?;
            Ok(ProviderMessageRecord {
                id,
                thread_id: thread,
                segment_id,
                provider_turn_id,
                role: serde_json::from_value(Value::String(role))?,
                content: serde_json::from_str(&body)?,
                provider_state: provider_state
                    .map(|value| serde_json::from_str(&value))
                    .transpose()?,
            })
        })
        .collect()
    }

    pub fn record_provider_turn_outcome(&self, record: &ProviderTurnOutcomeRecord) -> Result<()> {
        anyhow::ensure!(
            matches!(
                record.status.as_str(),
                "completed" | "refused" | "failed" | "interrupted" | "unknown"
            ),
            "invalid provider turn outcome status"
        );
        anyhow::ensure!(
            record
                .stop_reason
                .as_ref()
                .is_none_or(|value| value.len() <= 200),
            "provider stop reason is too long"
        );
        anyhow::ensure!(
            record
                .failure_kind
                .as_ref()
                .is_none_or(|value| value.len() <= 80),
            "provider failure kind is too long"
        );
        anyhow::ensure!(
            record
                .detail
                .as_ref()
                .is_none_or(|value| value.len() <= 4_000),
            "provider outcome detail is too long"
        );
        self.conn.execute(
            "INSERT INTO provider_turn_outcomes(turn_id,thread_id,status,stop_reason,failure_kind,detail) VALUES (?1,?2,?3,?4,?5,?6) ON CONFLICT(turn_id) DO UPDATE SET status=excluded.status,stop_reason=excluded.stop_reason,failure_kind=excluded.failure_kind,detail=excluded.detail,updated_at=CURRENT_TIMESTAMP",
            params![
                record.turn_id,
                record.thread_id.to_string(),
                record.status,
                record.stop_reason,
                record.failure_kind,
                record.detail
            ],
        )?;
        Ok(())
    }

    pub fn provider_turn_outcome(
        &self,
        turn_id: &str,
    ) -> Result<Option<ProviderTurnOutcomeRecord>> {
        self.conn
            .query_row(
                "SELECT thread_id,status,stop_reason,failure_kind,detail FROM provider_turn_outcomes WHERE turn_id=?1",
                [turn_id],
                |row| {
                    let thread_id: String = row.get(0)?;
                    Ok(ProviderTurnOutcomeRecord {
                        turn_id: turn_id.into(),
                        thread_id: HubThreadId(thread_id.parse().map_err(|error| {
                            rusqlite::Error::FromSqlConversionFailure(
                                0,
                                rusqlite::types::Type::Text,
                                Box::new(error),
                            )
                        })?),
                        status: row.get(1)?,
                        stop_reason: row.get(2)?,
                        failure_kind: row.get(3)?,
                        detail: row.get(4)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }
    pub fn create_tool_approval(&self, record: &ToolApprovalRecord) -> Result<()> {
        anyhow::ensure!(
            record.status == "pending",
            "new tool approval must be pending"
        );
        anyhow::ensure!(
            !record.id.trim().is_empty()
                && !record.turn_id.trim().is_empty()
                && record.digest.len() == 64
                && record.digest.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "tool approval identity is invalid"
        );
        self.conn.execute(
            "INSERT INTO tool_approvals(id,thread_id,turn_id,digest,request,status) VALUES (?1,?2,?3,?4,?5,'pending')",
            params![
                record.id,
                record.thread_id.to_string(),
                record.turn_id,
                record.digest,
                serde_json::to_string(&record.request)?,
            ],
        )?;
        Ok(())
    }
    pub fn tool_approval(&self, id: &str) -> Result<Option<ToolApprovalRecord>> {
        let row: Option<(String, String, String, String, String)> = self
            .conn
            .query_row(
                "SELECT thread_id,turn_id,digest,request,status FROM tool_approvals WHERE id=?1",
                [id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .optional()?;
        row.map(|(thread, turn_id, digest, request, status)| {
            Ok(ToolApprovalRecord {
                id: id.into(),
                thread_id: HubThreadId(thread.parse()?),
                turn_id,
                digest,
                request: serde_json::from_str(&request)?,
                status,
            })
        })
        .transpose()
    }
    pub fn decide_tool_approval(&self, id: &str, approve: bool) -> Result<()> {
        let status = if approve { "approved" } else { "denied" };
        if self.conn.execute(
            "UPDATE tool_approvals SET status=?2,decided_at=CURRENT_TIMESTAMP WHERE id=?1 AND status='pending'",
            params![id, status],
        )? != 1
        {
            bail!("tool approval is missing or was already decided");
        }
        Ok(())
    }
    /// Atomically spends the approval. A second invocation cannot reuse it.
    pub fn consume_tool_approval(
        &self,
        id: &str,
        thread: HubThreadId,
        turn_id: &str,
        digest: &str,
    ) -> Result<Value> {
        let tx = self.conn.unchecked_transaction()?;
        let request: Option<String> = tx
            .query_row(
                "SELECT request FROM tool_approvals WHERE id=?1 AND thread_id=?2 AND turn_id=?3 AND digest=?4 AND status='approved'",
                params![id, thread.to_string(), turn_id, digest],
                |row| row.get(0),
            )
            .optional()?;
        let request = request.context("matching one-shot tool approval is not approved")?;
        if tx.execute(
            "UPDATE tool_approvals SET status='consumed',consumed_at=CURRENT_TIMESTAMP WHERE id=?1 AND status='approved'",
            [id],
        )? != 1
        {
            bail!("tool approval was already consumed");
        }
        tx.commit()?;
        Ok(serde_json::from_str(&request)?)
    }
    pub fn create_review_run(&self, record: &ReviewRunRecord) -> Result<()> {
        anyhow::ensure!(
            record.verdict == "pending",
            "new review run must be pending"
        );
        anyhow::ensure!(
            record.executor_thread_id != record.reviewer_thread_id,
            "reviewer session must differ from executor session"
        );
        self.conn.execute(
            "INSERT INTO review_runs(id,task_id,executor_thread_id,reviewer_thread_id,artifact_hash,verdict,body) VALUES (?1,?2,?3,?4,?5,'pending',?6)",
            params![
                record.id,
                record.task_id.to_string(),
                record.executor_thread_id.to_string(),
                record.reviewer_thread_id.to_string(),
                record.artifact_hash,
                serde_json::to_string(&record.body)?,
            ],
        )?;
        Ok(())
    }
    pub fn finish_review_run(
        &self,
        id: &str,
        artifact_hash: &str,
        verdict: &str,
        body: &Value,
    ) -> Result<()> {
        anyhow::ensure!(
            matches!(verdict, "pass" | "rework" | "inconclusive"),
            "invalid review verdict"
        );
        if self.conn.execute(
            "UPDATE review_runs SET verdict=?3,body=?4,completed_at=CURRENT_TIMESTAMP WHERE id=?1 AND artifact_hash=?2 AND verdict='pending'",
            params![id, artifact_hash, verdict, serde_json::to_string(body)?],
        )? != 1
        {
            bail!("review run is missing, stale, or already completed");
        }
        Ok(())
    }
    pub fn review_runs(&self, task: hub_core::TaskId) -> Result<Vec<ReviewRunRecord>> {
        let mut query = self.conn.prepare("SELECT id,executor_thread_id,reviewer_thread_id,artifact_hash,verdict,body FROM review_runs WHERE task_id=?1 ORDER BY created_at,rowid")?;
        let rows = query.query_map([task.to_string()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
            ))
        })?;
        rows.map(|row| {
            let (id, executor, reviewer, artifact_hash, verdict, body) = row?;
            Ok(ReviewRunRecord {
                id,
                task_id: task,
                executor_thread_id: HubThreadId(executor.parse()?),
                reviewer_thread_id: HubThreadId(reviewer.parse()?),
                artifact_hash,
                verdict,
                body: serde_json::from_str(&body)?,
            })
        })
        .collect()
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
        if tx
            .query_row("SELECT 1 FROM settings WHERE key=?1", [&key], |_| Ok(()))
            .optional()?
            .is_some()
        {
            bail!("このManifestは取り込み済みです。保存されたタスクを確認してください。");
        }
        tx.execute(
            "INSERT INTO settings(key,value) VALUES (?1,?2)",
            params![key, manifest],
        )?;
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
    /// Attribute only records with one evidenced project; never use the active UI project.
    pub fn usage_projects(&self) -> Result<std::collections::HashMap<String, (String, String)>> {
        let mut query = self.conn.prepare(
            "WITH links AS (
                SELECT u.id, t.project_id FROM model_usage u JOIN tasks t ON t.id=u.task_id
                UNION
                SELECT u.id, t.project_id FROM model_usage u JOIN provider_threads t
                ON (u.task_id IS NOT NULL AND t.task_id=u.task_id)
                OR (u.provider=t.provider AND u.turn_id IS NOT NULL AND (
                    EXISTS (SELECT 1 FROM turns x WHERE x.thread_id=t.id AND x.provider_turn_id=u.turn_id)
                    OR EXISTS (SELECT 1 FROM events e WHERE e.thread_id=t.id AND json_extract(e.body,'$.turn_id')=u.turn_id)
                    OR EXISTS (SELECT 1 FROM provider_usage_snapshots s WHERE s.provider_thread_id=t.provider_thread_id AND s.turn_id=u.turn_id)
                ))
            ) SELECT l.id, p.id, p.name FROM links l JOIN projects p ON p.id=l.project_id
              GROUP BY l.id HAVING COUNT(DISTINCT l.project_id)=1"
        )?;
        let rows = query.query_map([], |row| Ok((row.get(0)?, (row.get(1)?, row.get(2)?))))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
    pub fn thread_usage(&self, id: HubThreadId) -> Result<Vec<hub_core::ModelUsageRecord>> {
        self.scoped_usage(Some(id))
    }
    fn scoped_usage(&self, thread: Option<HubThreadId>) -> Result<Vec<hub_core::ModelUsageRecord>> {
        let mut q=self.conn.prepare("SELECT u.id,u.provider,u.model,u.turn_id,u.prompt_tokens,u.cached_tokens,u.completion_tokens,u.estimated_cost,
          COALESCE(u.latency_ms,(SELECT CAST(ROUND((julianday(MIN(CASE WHEN e.kind='turn_completed' THEN e.created_at END))-julianday(MIN(CASE WHEN e.kind='turn_started' THEN e.created_at END)))*86400000) AS INTEGER) FROM events e WHERE json_extract(e.body,'$.turn_id')=u.turn_id AND u.provider='codex')),u.task_id
          FROM model_usage u WHERE ?1 IS NULL OR EXISTS (SELECT 1 FROM provider_threads t WHERE t.id=?1 AND ((u.turn_id IS NULL AND u.task_id IS NOT NULL AND u.task_id=t.task_id AND (SELECT COUNT(*) FROM provider_threads linked WHERE linked.task_id=u.task_id)=1) OR EXISTS (SELECT 1 FROM turns x WHERE x.thread_id=t.id AND x.provider_turn_id=u.turn_id) OR EXISTS (SELECT 1 FROM events e WHERE e.thread_id=t.id AND json_extract(e.body,'$.turn_id')=u.turn_id) OR EXISTS (SELECT 1 FROM provider_usage_snapshots p WHERE p.provider_thread_id=t.provider_thread_id AND p.turn_id=u.turn_id))) ORDER BY u.created_at DESC")?;
        let rows = q.query_map([thread.map(|id| id.to_string())], |r| {
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
    pub fn recent_activity(
        &self,
        id: HubThreadId,
        turn: Option<&str>,
    ) -> Result<Vec<(i64, String)>> {
        let mut q = self.conn.prepare("SELECT id,body FROM events WHERE thread_id=?1 AND (?2 IS NULL OR json_extract(body,'$.turn_id')=?2) AND kind IN ('message_completed','item_started','item_completed','error','turn_started','turn_completed','diff') ORDER BY id DESC LIMIT 60")?;
        let rows = q.query_map(params![id.to_string(), turn], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?;
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
        let dir = tempfile::tempdir()?;
        assert!(std::process::Command::new("git")
            .arg("init")
            .arg(dir.path())
            .output()?
            .status
            .success());
        let store = Store::open(&dir.path().join("usage.db"))?;
        let project = store.register_project(dir.path())?;
        let mut ids = Vec::new();
        for n in 0..2 {
            let thread = ThreadMapping {
                id: HubThreadId::default(),
                project_id: project.id,
                provider: "codex".into(),
                provider_thread_id: format!("p{n}"),
                title: "test".into(),
                status: "completed".into(),
            };
            store.save_thread(&thread)?;
            store.record_usage(&hub_core::ModelUsageRecord {
                id: format!("usage{n}"),
                provider: "codex".into(),
                model: "sol".into(),
                task_id: None,
                turn_id: Some(format!("turn{n}")),
                prompt_tokens: Some(10),
                cached_tokens: None,
                completion_tokens: Some(2),
                estimated_cost: None,
                latency_ms: None,
            })?;
            for (kind, time) in [
                ("turn_started", "2026-09-09 01:00:00.000"),
                ("turn_completed", "2026-09-09 01:00:02.500"),
            ] {
                let seq = store.append_event(
                    Some(&thread.provider_thread_id),
                    kind,
                    &serde_json::json!({"turn_id":format!("turn{n}")}).to_string(),
                )?;
                store.conn.execute(
                    "UPDATE events SET created_at=?2 WHERE id=?1",
                    params![seq, time],
                )?;
            }
            ids.push(thread.id);
        }
        let attributed = store.usage_projects()?;
        assert_eq!(attributed.len(), 2);
        assert_eq!(attributed["usage0"].0, project.id.to_string());
        assert_eq!(attributed["usage1"].1, project.name);
        // A second project sharing an ambiguous provider turn must not inherit attribution.
        let other_dir = tempfile::tempdir()?;
        assert!(std::process::Command::new("git").args(["init", "-q"]).arg(other_dir.path()).status()?.success());
        let other = store.register_project(other_dir.path())?;
        let other_thread = ThreadMapping {
            id: HubThreadId::default(), project_id: other.id, provider: "codex".into(),
            provider_thread_id: "other-provider".into(), title: "other".into(), status: "completed".into(),
        };
        store.save_thread(&other_thread)?;
        store.append_event(Some("other-provider"), "turn_completed", r#"{"turn_id":"turn0"}"#)?;
        assert!(!store.usage_projects()?.contains_key("usage0"));
        store.record_usage(&hub_core::ModelUsageRecord {
            id: "unattributed".into(), provider: "local".into(), model: "local".into(), task_id: None,
            turn_id: None, prompt_tokens: None, cached_tokens: None, completion_tokens: None,
            estimated_cost: None, latency_ms: None,
        })?;
        assert!(!store.usage_projects()?.contains_key("unattributed"));
        assert_eq!(store.usage()?.len(), 3);
        for (n, id) in ids.into_iter().enumerate() {
            let rows = store.thread_usage(id)?;
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].id, format!("usage{n}"));
            assert_eq!(rows[0].latency_ms, Some(2500));
        }
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
        let autonomous_key = format!("autonomous:{}", thread.id);
        let review_key = format!("review:{}:iteration", thread.id);
        store.set_setting(&autonomous_key, "private execution state")?;
        store.set_setting(&review_key, "private review")?;
        store.delete_thread(thread.id)?;
        assert!(store.threads()?.is_empty());
        assert_eq!(store.setting(&key)?, None);
        assert_eq!(store.setting(&autonomous_key)?, None);
        assert_eq!(store.setting(&review_key)?, None);
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
    #[test]
    fn provider_profiles_are_versioned_and_turn_targets_are_immutable() -> Result<()> {
        use protocol_types::providers::{
            ModelTarget, ProviderLocality, ProviderProfile, ProviderProtocol, SessionModelPolicy,
            TargetResolution,
        };
        let dir = tempfile::tempdir()?;
        let store = Store::open(&dir.path().join("hub.db"))?;
        let project = ProjectId::default();
        store.conn.execute(
            "INSERT INTO projects(id,name,root) VALUES (?1,'fixture','/fixture')",
            [project.to_string()],
        )?;
        let thread = ThreadMapping {
            id: HubThreadId::default(),
            project_id: project,
            provider: "api".into(),
            provider_thread_id: "logical".into(),
            title: "fixture".into(),
            status: "idle".into(),
        };
        store.save_thread(&thread)?;
        let mut profile = ProviderProfile {
            id: "anthropic-main".into(),
            name: "Anthropic".into(),
            protocol: ProviderProtocol::AnthropicMessages,
            base_url: Some("https://api.anthropic.com".into()),
            locality: ProviderLocality::Cloud,
            credential_env: Some("ANTHROPIC_API_KEY".into()),
            max_concurrency: 1,
            enabled: true,
            revision: 1,
        };
        store.save_provider_profile(&profile)?;
        assert!(store.save_provider_profile(&profile).is_err());
        profile.revision = 2;
        profile.max_concurrency = 2;
        store.save_provider_profile(&profile)?;
        assert_eq!(store.provider_profile("anthropic-main")?.unwrap(), profile);

        let target = ModelTarget {
            profile_id: profile.id.clone(),
            model_id: "claude-fixture".into(),
            effort: Some("high".into()),
        };
        let policy = SessionModelPolicy {
            default_target: target.clone(),
            reviewer_target: None,
            allow_turn_override: true,
        };
        store.set_session_model_policy(thread.id, &policy)?;
        assert_eq!(store.session_model_policy(thread.id)?, Some(policy));
        let segment = ProviderSegmentRecord {
            id: "segment-1".into(),
            thread_id: thread.id,
            target: target.clone(),
            profile_revision: 2,
            provider_thread_id: None,
            ended: false,
        };
        store.start_provider_segment(&segment)?;
        assert_eq!(
            store.active_provider_segment(thread.id)?,
            Some(segment.clone())
        );
        let conflicting_segment = ProviderSegmentRecord {
            target: ModelTarget {
                model_id: "claude-other".into(),
                ..target.clone()
            },
            ..segment.clone()
        };
        assert!(store.start_provider_segment(&conflicting_segment).is_err());
        assert_eq!(
            store.active_provider_segment(thread.id)?,
            Some(segment.clone())
        );
        let turn = TurnTargetRecord {
            turn_id: "turn-1".into(),
            thread_id: thread.id,
            segment_id: segment.id,
            target,
            profile_revision: 2,
            resolved_from: TargetResolution::SessionOverride,
        };
        store.record_turn_target(&turn)?;
        assert_eq!(store.turn_target("turn-1")?, Some(turn.clone()));
        assert!(store.record_turn_target(&turn).is_err());
        let outcome = ProviderTurnOutcomeRecord {
            turn_id: turn.turn_id,
            thread_id: thread.id,
            status: "refused".into(),
            stop_reason: Some("refusal".into()),
            failure_kind: Some("refusal".into()),
            detail: Some("policy".into()),
        };
        store.record_provider_turn_outcome(&outcome)?;
        assert_eq!(store.provider_turn_outcome("turn-1")?, Some(outcome));
        Ok(())
    }

    #[test]
    fn provider_messages_approvals_and_reviews_are_durable_and_one_shot() -> Result<()> {
        use protocol_types::providers::{ContentBlock, TranscriptRole};
        let dir = tempfile::tempdir()?;
        let store = Store::open(&dir.path().join("provider-state.db"))?;
        let project = ProjectId::default();
        let task = hub_core::TaskId::default();
        store.conn.execute(
            "INSERT INTO projects(id,name,root) VALUES (?1,'fixture','/fixture')",
            [project.to_string()],
        )?;
        store.conn.execute(
            "INSERT INTO tasks(id,project_id,status,body) VALUES (?1,?2,'running','{}')",
            params![task.to_string(), project.to_string()],
        )?;
        let executor = ThreadMapping {
            id: HubThreadId::default(),
            project_id: project,
            provider: "api:test".into(),
            provider_thread_id: "executor".into(),
            title: "executor".into(),
            status: "running".into(),
        };
        let reviewer = ThreadMapping {
            id: HubThreadId::default(),
            provider_thread_id: "reviewer".into(),
            title: "reviewer".into(),
            ..executor.clone()
        };
        store.save_thread(&executor)?;
        store.save_thread(&reviewer)?;

        let message = ProviderMessageRecord {
            id: "message-1".into(),
            thread_id: executor.id,
            segment_id: None,
            provider_turn_id: Some("turn-1".into()),
            role: TranscriptRole::Assistant,
            content: vec![ContentBlock::Text {
                text: "done".into(),
            }],
            provider_state: Some(serde_json::json!({"opaque":"same-segment-only"})),
        };
        store.append_provider_message(&message)?;
        assert!(store.append_provider_message(&message).is_err());
        assert_eq!(store.provider_messages(executor.id)?, vec![message]);

        let approval = ToolApprovalRecord {
            id: "approval-1".into(),
            thread_id: executor.id,
            turn_id: "turn-1".into(),
            digest: "a".repeat(64),
            request: serde_json::json!({"argv":["custom-check"]}),
            status: "pending".into(),
        };
        store.create_tool_approval(&approval)?;
        store.decide_tool_approval(&approval.id, true)?;
        assert!(store.decide_tool_approval(&approval.id, true).is_err());
        assert_eq!(
            store.consume_tool_approval(&approval.id, executor.id, "turn-1", &"a".repeat(64))?,
            approval.request
        );
        assert!(store
            .consume_tool_approval(&approval.id, executor.id, "turn-1", &"a".repeat(64))
            .is_err());

        let review = ReviewRunRecord {
            id: "review-1".into(),
            task_id: task,
            executor_thread_id: executor.id,
            reviewer_thread_id: reviewer.id,
            artifact_hash: "artifact-a".into(),
            verdict: "pending".into(),
            body: serde_json::json!({"status":"started"}),
        };
        store.create_review_run(&review)?;
        assert!(store
            .finish_review_run(
                &review.id,
                "artifact-b",
                "pass",
                &serde_json::json!({"summary":"stale"})
            )
            .is_err());
        store.finish_review_run(
            &review.id,
            "artifact-a",
            "pass",
            &serde_json::json!({"summary":"ok"}),
        )?;
        let runs = store.review_runs(task)?;
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].verdict, "pass");
        assert!(store
            .finish_review_run(
                &review.id,
                "artifact-a",
                "pass",
                &serde_json::json!({"summary":"reused"})
            )
            .is_err());
        Ok(())
    }
}
