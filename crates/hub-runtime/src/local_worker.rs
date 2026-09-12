use anyhow::{anyhow, bail, Context, Result};
use cap_std::{ambient_authority, fs::Dir};
use hub_core::{HubThreadId, ModelUsageRecord, ProjectId, ThreadMapping};
use hub_db::Store;
use hub_events::EventBus;
use protocol_types::{local::*, AgentEvent};
use serde::{Deserialize, Serialize};
use similar::{ChangeTag, TextDiff};
use std::{
    collections::HashSet,
    io::Write,
    path::{Component, Path},
    sync::{Arc, Mutex},
};

struct Replacement {
    path: String,
    before: String,
    after: String,
}
pub struct PreparedEdits {
    dir: Dir,
    edits: Vec<Replacement>,
    pub diff: String,
}
fn validate_path(path: &str) -> Result<()> {
    if path.is_empty()
        || Path::new(path)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        bail!("file path must be relative and cannot traverse directories");
    }
    if Path::new(path).components().any(|c| {
        let n = c.as_os_str().to_string_lossy().to_lowercase();
        n == ".git"
            || n == ".codex"
            || n == ".agents"
            || n == ".ssh"
            || n.starts_with(".env")
            || n.ends_with(".pem")
            || n.ends_with(".key")
    }) {
        bail!("sensitive/configuration path is not eligible for the local worker");
    }
    Ok(())
}
fn read_file(dir: &Dir, path: &str) -> Result<String> {
    validate_path(path)?;
    let mut cursor = std::path::PathBuf::new();
    for component in Path::new(path).components() {
        cursor.push(component);
        if dir.symlink_metadata(&cursor)?.file_type().is_symlink() {
            bail!("symbolic links are not eligible for the local worker");
        }
    }
    let metadata = dir.metadata(path)?;
    if !metadata.is_file() || metadata.len() > 32000 {
        bail!("selected file is not a small regular text file");
    }
    let text = dir.read_to_string(path)?;
    if hub_policy::redact(&text) != text {
        bail!("selected file contains secret-like text; do not put it in local context");
    }
    Ok(text)
}
pub fn collect_files(root: &Path, paths: &[String]) -> Result<Vec<FileContext>> {
    if paths.is_empty() || paths.len() > 2 {
        bail!("select one or two files for Spark");
    }
    let dir = Dir::open_ambient_dir(root, ambient_authority())?;
    let mut seen = HashSet::new();
    let mut total = 0;
    paths
        .iter()
        .map(|p| {
            if !seen.insert(p) {
                bail!("duplicate selected path");
            }
            let content = read_file(&dir, p)?;
            total += content.len();
            if total > 24000 {
                bail!("selected files exceed local context allowance");
            }
            Ok(FileContext {
                path: p.clone(),
                content,
            })
        })
        .collect()
}
impl PreparedEdits {
    pub fn prepare(root: &Path, files: &[FileContext], proposal: &EditProposal) -> Result<Self> {
        if proposal.edits.is_empty() || proposal.edits.len() > 2 {
            bail!("expected one or two edits");
        }
        let dir = Dir::open_ambient_dir(root, ambient_authority())?;
        let mut seen = HashSet::new();
        let mut edits = Vec::new();
        let mut diff = String::new();
        let mut lines = 0;
        for edit in &proposal.edits {
            if !seen.insert(&edit.path) {
                bail!("multiple edits to one path are not supported");
            }
            let file = files
                .iter()
                .find(|f| f.path == edit.path)
                .context("model selected an unapproved path")?;
            if read_file(&dir, &edit.path)? != file.content {
                bail!("file changed since context capture");
            }
            if edit.old.is_empty() || file.content.matches(&edit.old).count() != 1 {
                bail!("replacement must match exactly once");
            }
            let after = file.content.replacen(&edit.old, &edit.new, 1);
            if after == file.content {
                bail!("local edit is a no-op");
            }
            if after.len() > 32000 || hub_policy::redact(&after) != after {
                bail!("replacement exceeds file limit or contains secret-like text");
            }
            let d = TextDiff::from_lines(&file.content, &after);
            lines += d
                .iter_all_changes()
                .filter(|c| c.tag() != ChangeTag::Equal)
                .count();
            if lines >= 100 {
                bail!("local change exceeds 99 changed lines");
            }
            diff += &d.unified_diff().header(&edit.path, &edit.path).to_string();
            edits.push(Replacement {
                path: edit.path.clone(),
                before: file.content.clone(),
                after,
            });
        }
        Ok(Self { dir, edits, diff })
    }
    fn atomic_write(&self, path: &str, content: &str) -> Result<()> {
        let target = Path::new(path);
        let parent = target.parent().unwrap_or(Path::new(""));
        let temp = parent.join(format!(".astra-{}.tmp", uuid::Uuid::new_v4()));
        let mut options = cap_std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        let mut f = self.dir.open_with(&temp, &options)?;
        let result = (|| -> Result<()> {
            f.set_permissions(self.dir.metadata(path)?.permissions())?;
            f.write_all(content.as_bytes())?;
            f.sync_all()?;
            self.dir.rename(&temp, &self.dir, path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = self.dir.remove_file(temp);
        }
        result
    }
    pub fn apply(self) -> Result<()> {
        // Revalidate every preimage before the first write. A mutation failure never
        // silently escalates into another writer; restore only our exact writes.
        for e in &self.edits {
            if read_file(&self.dir, &e.path)? != e.before {
                bail!("preimage changed before apply");
            }
        }
        let mut written: Vec<&Replacement> = Vec::new();
        for e in &self.edits {
            let result = self.atomic_write(&e.path, &e.after).and_then(|_| {
                if self.dir.read_to_string(&e.path)? != e.after {
                    bail!("post-write verification failed");
                }
                Ok(())
            });
            if let Err(error) = result {
                let mut rollback_failed = false;
                // Include the current path if the rename succeeded before verification.
                if self.dir.read_to_string(&e.path).ok().as_deref() == Some(&e.after) {
                    written.push(e);
                }
                for prior in written.iter().rev() {
                    if self.dir.read_to_string(&prior.path).ok().as_deref() != Some(&prior.after)
                        || self.atomic_write(&prior.path, &prior.before).is_err()
                    {
                        rollback_failed = true;
                    }
                }
                bail!("local apply failed: {error}; rollback_failed={rollback_failed}; inspect the diff before any retry");
            }
            written.push(e);
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalResult {
    pub summary: String,
    pub diff: String,
    pub review: ReviewResult,
}
pub struct LocalExecutor {
    pub store: Arc<Mutex<Store>>,
    pub bus: EventBus,
    pub provider: Arc<dyn LocalModelProvider>,
    pub lock: tokio::sync::Mutex<()>,
}
impl LocalExecutor {
    pub fn record_usage(&self, usage: &LocalUsage, turn: Option<String>) -> Result<()> {
        self.store
            .lock()
            .map_err(|_| anyhow!("database lock poisoned"))?
            .record_usage(&ModelUsageRecord {
                id: uuid::Uuid::new_v4().to_string(),
                provider: "spark".into(),
                model: self.provider.model_id().into(),
                task_id: None,
                turn_id: turn,
                prompt_tokens: Some(usage.prompt_tokens),
                cached_tokens: None,
                completion_tokens: Some(usage.completion_tokens),
                estimated_cost: None,
                latency_ms: Some(usage.latency_ms),
            })
    }
    pub fn create(&self, project: ProjectId, title: String) -> Result<ThreadMapping> {
        let m = ThreadMapping {
            id: HubThreadId::default(),
            project_id: project,
            provider: "spark".into(),
            provider_thread_id: format!("spark:{}", uuid::Uuid::new_v4()),
            title,
            status: "idle".into(),
        };
        let store = self
            .store
            .lock()
            .map_err(|_| anyhow!("database lock poisoned"))?;
        store.save_thread(&m)?;
        store.set_setting(&format!("thread_model:{}", m.id), self.provider.model_id())?;
        let target = protocol_types::providers::ModelTarget {
            profile_id: "spark".into(),
            model_id: self.provider.model_id().into(),
            effort: None,
        };
        store.set_session_model_policy(
            m.id,
            &protocol_types::providers::SessionModelPolicy {
                default_target: target.clone(),
                reviewer_target: None,
                allow_turn_override: false,
            },
        )?;
        if let Some(profile) = store.provider_profile("spark")? {
            store.start_provider_segment(&hub_db::ProviderSegmentRecord {
                id: uuid::Uuid::new_v4().to_string(),
                thread_id: m.id,
                target,
                profile_revision: profile.revision,
                provider_thread_id: Some(m.provider_thread_id.clone()),
                ended: false,
            })?;
        }
        Ok(m)
    }
    #[allow(unreachable_code)]
    pub async fn run(
        &self,
        id: HubThreadId,
        request: String,
        paths: Vec<String>,
    ) -> Result<LocalResult> {
        let _ = (id, request, paths);
        return Err(anyhow!(
            "direct local writes are disabled; run the bounded edit in a reviewed worktree"
        ));
        let _guard = self.lock.lock().await;
        let (mapping, root) = {
            let s = self
                .store
                .lock()
                .map_err(|_| anyhow!("database lock poisoned"))?;
            let m = s
                .threads()?
                .into_iter()
                .find(|m| m.id == id && m.provider == "spark")
                .context("unknown Spark thread")?;
            let pinned = s
                .setting(&format!("thread_model:{id}"))?
                .unwrap_or_else(|| "abenzerps/Spark-X2.5-4B-MLX-8bit".into());
            if pinned != self.provider.model_id() {
                bail!("このタスクのローカルモデルは {} です。接続設定を戻すか、新しいタスクを作成してください。", pinned);
            }
            let root = s
                .projects()?
                .into_iter()
                .find(|p| p.id == m.project_id)
                .context("unregistered project")?
                .root;
            (m, root)
        };
        if root.canonicalize()? != root {
            bail!("registered project root changed");
        }
        let turn = uuid::Uuid::new_v4().to_string();
        let emit = |kind: &str, text: String| {
            self.bus.publish(AgentEvent {
                details: None,
                thread_id: Some(mapping.provider_thread_id.clone()),
                turn_id: Some(turn.clone()),
                item_id: Some(turn.clone()),
                kind: kind.into(),
                text,
            })
        };
        emit("user_message", request.clone())?;
        emit("turn_started", "inProgress".into())?;
        let result = async {
            let files = collect_files(&root, &paths)?;
            let generation = self.provider.implement(&request, &files).await?;
            self.record_usage(&generation.usage, Some(turn.clone()))?;
            let prepared = PreparedEdits::prepare(&root, &files, &generation.output)?;
            let review = self.provider.review_diff(&request, &prepared.diff).await?;
            self.record_usage(&review.usage, Some(turn.clone()))?;
            if review.output.verdict != ReviewVerdict::Approve {
                bail!(
                    "Spark first-pass review requires attention: {}",
                    review.output.summary
                );
            }
            let result = LocalResult {
                summary: generation.output.summary,
                diff: prepared.diff.clone(),
                review: review.output,
            };
            prepared.apply()?;
            emit("diff", result.diff.clone())?;
            emit("message_completed", result.summary.clone())?;
            Ok::<_, anyhow::Error>(result)
        }
        .await;
        match &result {
            Ok(_) => emit("turn_completed", "completed".into())?,
            Err(e) => {
                emit("error", format!("{e:#}"))?;
                emit("turn_completed", "failed".into())?;
            }
        }
        result
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn legacy_direct_local_write_is_fail_closed() -> Result<()> {
        let dir = tempfile::tempdir()?;
        assert!(std::process::Command::new("git")
            .arg("init")
            .arg(dir.path())
            .output()?
            .status
            .success());
        std::fs::write(dir.path().join("a.txt"), "unchanged")?;
        let store = Arc::new(Mutex::new(Store::open(&dir.path().join("hub.db"))?));
        let project = store.lock().unwrap().register_project(dir.path())?;
        let executor = LocalExecutor {
            store: store.clone(),
            bus: EventBus::new(store.clone()),
            provider: Arc::new(provider_spark::SparkProvider::new("http://127.0.0.1:1")?),
            lock: tokio::sync::Mutex::new(()),
        };
        let thread = executor.create(project.id, "test".into())?;
        let error = executor
            .run(thread.id, "edit".into(), vec!["a.txt".into()])
            .await
            .unwrap_err();
        assert!(error.to_string().contains("reviewed worktree"));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.txt"))?,
            "unchanged"
        );
        Ok(())
    }
    #[test]
    fn preimage_and_scope_protected() -> Result<()> {
        let d = tempfile::tempdir()?;
        std::fs::write(d.path().join("a.txt"), "hello hello\n")?;
        let f = collect_files(d.path(), &["a.txt".into()])?;
        let p = EditProposal {
            edits: vec![Edit {
                path: "a.txt".into(),
                old: "hello".into(),
                new: "hi".into(),
            }],
            summary: "rename".into(),
        };
        assert!(PreparedEdits::prepare(d.path(), &f, &p).is_err());
        assert!(collect_files(d.path(), &["../outside".into()]).is_err());
        Ok(())
    }
    #[test]
    fn edit_verified_and_concurrent_change_refused() -> Result<()> {
        let d = tempfile::tempdir()?;
        let p = d.path().join("a.txt");
        std::fs::write(&p, "hello\n")?;
        let f = collect_files(d.path(), &["a.txt".into()])?;
        let proposal = EditProposal {
            edits: vec![Edit {
                path: "a.txt".into(),
                old: "hello".into(),
                new: "astra".into(),
            }],
            summary: "rename".into(),
        };
        let prepared = PreparedEdits::prepare(d.path(), &f, &proposal)?;
        std::fs::write(&p, "someone else's edit\n")?;
        assert!(prepared.apply().is_err());
        assert_eq!(std::fs::read_to_string(&p)?, "someone else's edit\n");
        Ok(())
    }
    #[test]
    fn bounded_edit_applies() -> Result<()> {
        let d = tempfile::tempdir()?;
        let p = d.path().join("a.txt");
        std::fs::write(&p, "hello\n")?;
        let files = collect_files(d.path(), &["a.txt".into()])?;
        let proposal = EditProposal {
            edits: vec![Edit {
                path: "a.txt".into(),
                old: "hello".into(),
                new: "astra".into(),
            }],
            summary: "rename".into(),
        };
        PreparedEdits::prepare(d.path(), &files, &proposal)?.apply()?;
        assert_eq!(std::fs::read_to_string(p)?, "astra\n");
        Ok(())
    }
    #[cfg(unix)]
    #[test]
    fn symlink_escape_refused() -> Result<()> {
        let d = tempfile::tempdir()?;
        std::os::unix::fs::symlink("/etc/hosts", d.path().join("a.txt"))?;
        assert!(collect_files(d.path(), &["a.txt".into()]).is_err());
        Ok(())
    }
}
