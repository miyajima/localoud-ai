use anyhow::{bail, Context, Result};
use hub_core::{TaskId, Worktree, WorktreeId};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::{io::AsyncWriteExt, process::Command};

#[derive(Clone)]
pub struct GitWorktrees {
    repo: PathBuf,
    root: PathBuf,
}
impl GitWorktrees {
    pub fn new(repo: &Path) -> Result<Self> {
        let repo = repo.canonicalize()?;
        let root = repo.join(".agent-worktrees");
        if !root.exists() {
            std::fs::create_dir(&root)?;
        }
        if std::fs::symlink_metadata(&root)?.file_type().is_symlink()
            || root.canonicalize()?.parent() != Some(repo.as_path())
        {
            bail!("worktree root must be a controlled directory in the project");
        }
        Ok(Self { repo, root })
    }
    async fn git(&self, cwd: &Path, args: &[&str]) -> Result<std::process::Output> {
        let mut c = Command::new("git");
        c.arg("-C").arg(cwd).args(args).kill_on_drop(true);
        let out = tokio::time::timeout(Duration::from_secs(30), c.output()).await??;
        if !out.status.success() {
            bail!(
                "Git {} failed: {}",
                args.first().unwrap_or(&"command"),
                String::from_utf8_lossy(&out.stderr)
            );
        }
        Ok(out)
    }
    fn validate_root(&self) -> Result<()> {
        if std::fs::symlink_metadata(&self.root)?
            .file_type()
            .is_symlink()
            || self.root.canonicalize()?.parent() != Some(self.repo.as_path())
        {
            bail!("controlled worktree root changed");
        }
        Ok(())
    }
    fn owned(&self, w: &Worktree) -> Result<()> {
        self.validate_root()?;
        if !matches!(w.base_sha.len(), 40 | 64)
            || !w.base_sha.bytes().all(|b| b.is_ascii_hexdigit())
        {
            bail!("invalid worktree base hash");
        }
        if w.path != self.root.join(format!("task-{}", w.task_id)) {
            bail!("worktree ownership mismatch");
        }
        if w.path.exists()
            && (std::fs::symlink_metadata(&w.path)?.file_type().is_symlink()
                || w.path.canonicalize()?.parent() != Some(self.root.as_path()))
        {
            bail!("worktree escaped its controlled root");
        }
        Ok(())
    }
    pub async fn create(&self, task: TaskId, base_ref: &str) -> Result<Worktree> {
        self.validate_root()?;
        let reference = format!("{base_ref}^{{commit}}");
        let output = self
            .git(
                &self.repo,
                &["rev-parse", "--verify", "--end-of-options", &reference],
            )
            .await?;
        let sha = String::from_utf8(output.stdout)?.trim().to_owned();
        if sha.len() != 40 && sha.len() != 64 {
            bail!("Git did not return a commit hash");
        }
        let path = self.root.join(format!("task-{task}"));
        if path.exists() {
            bail!("task worktree already exists; reconcile it before retrying");
        }
        self.git(
            &self.repo,
            &[
                "worktree",
                "add",
                "--detach",
                path.to_str().context("non UTF-8 worktree path")?,
                &sha,
            ],
        )
        .await?;
        Ok(Worktree {
            id: WorktreeId::default(),
            task_id: task,
            path,
            base_sha: sha,
        })
    }
    pub async fn status(&self, w: &Worktree) -> Result<String> {
        self.owned(w)?;
        Ok(String::from_utf8(
            self.git(&w.path, &["status", "--porcelain=v1"])
                .await?
                .stdout,
        )?)
    }
    pub async fn diff(&self, w: &Worktree) -> Result<String> {
        self.owned(w)?;
        let mut diff = String::from_utf8(
            self.git(
                &w.path,
                &[
                    "diff",
                    "--binary",
                    "--no-ext-diff",
                    "--no-textconv",
                    &w.base_sha,
                    "--",
                ],
            )
            .await?
            .stdout,
        )?;
        if diff.len() > 4_000_000 {
            bail!("worktree diff exceeds 4 MB");
        }
        let untracked = self
            .git(
                &w.path,
                &["ls-files", "--others", "--exclude-standard", "-z"],
            )
            .await?;
        for path in untracked
            .stdout
            .split(|b| *b == 0)
            .filter(|p| !p.is_empty())
        {
            let path = std::str::from_utf8(path)?;
            let metadata = std::fs::symlink_metadata(w.path.join(path))?;
            if !metadata.is_file() || metadata.len() > 2_000_000 {
                bail!("untracked path is not a bounded regular file: {path}");
            }
            let output = Command::new("git")
                .arg("-C")
                .arg(&w.path)
                .args([
                    "diff",
                    "--no-index",
                    "--binary",
                    "--no-ext-diff",
                    "--no-textconv",
                    "--",
                    "/dev/null",
                    path,
                ])
                .kill_on_drop(true)
                .output()
                .await?;
            if !matches!(output.status.code(), Some(0 | 1)) {
                bail!("failed to capture new file {path}");
            }
            diff += &String::from_utf8(output.stdout)?;
            if diff.len() > 4_000_000 {
                bail!("worktree diff exceeds 4 MB");
            }
        }
        Ok(diff)
    }
    pub async fn review_provenance(&self, w: &Worktree) -> Result<String> {
        self.owned(w)?;
        let head = String::from_utf8(self.git(&w.path, &["rev-parse", "HEAD"]).await?.stdout)?;
        let range = format!("{}..HEAD", w.base_sha);
        let merges = String::from_utf8(
            self.git(&w.path, &["log", "--format=%H", "--merges", &range])
                .await?
                .stdout,
        )?;
        Ok(format!(
            "Worktree base SHA: {}; current HEAD: {}; merge commits since base: {}",
            w.base_sha,
            head.trim(),
            if merges.trim().is_empty() {
                "none"
            } else {
                merges.trim()
            }
        ))
    }
    pub async fn apply_dependency_patch(&self, w: &Worktree, patch: &str) -> Result<()> {
        self.owned(w)?;
        if patch.len() > 4_000_000 {
            bail!("dependency patch exceeds limit");
        }
        if patch.is_empty() {
            return Ok(());
        }
        for check in [true, false] {
            let mut command = Command::new("git");
            command.arg("-C").arg(&w.path).arg("apply");
            if check {
                command.arg("--check");
            }
            let mut child = command
                .args(["--whitespace=nowarn", "-"])
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true)
                .spawn()?;
            child
                .stdin
                .take()
                .context("missing Git stdin")?
                .write_all(patch.as_bytes())
                .await?;
            let result =
                tokio::time::timeout(Duration::from_secs(30), child.wait_with_output()).await??;
            if !result.status.success() {
                bail!(
                    "dependency patch conflict; no automatic merge: {}",
                    String::from_utf8_lossy(&result.stderr)
                );
            }
        }
        Ok(())
    }
    pub async fn remove(&self, w: &Worktree) -> Result<()> {
        self.owned(w)?;
        if !self.status(w).await?.is_empty() {
            bail!("refusing to remove a dirty worktree");
        }
        self.git(
            &self.repo,
            &[
                "worktree",
                "remove",
                w.path.to_str().context("non UTF-8 path")?,
            ],
        )
        .await?;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn worktrees_are_isolated_and_dirty_removal_is_rejected() -> Result<()> {
        let dir = tempfile::tempdir()?;
        for args in [
            vec!["init"],
            vec![
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@localhost",
                "commit",
                "--allow-empty",
                "-m",
                "base",
            ],
        ] {
            assert!(std::process::Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(args)
                .output()?
                .status
                .success());
        }
        let manager = GitWorktrees::new(dir.path())?;
        let a = manager.create(TaskId::default(), "HEAD").await?;
        let b = manager.create(TaskId::default(), "HEAD").await?;
        std::fs::write(a.path.join("new.txt"), "isolated\n")?;
        assert!(!b.path.join("new.txt").exists());
        let patch = manager.diff(&a).await?;
        assert!(patch.contains("+isolated"));
        assert!(manager.remove(&a).await.is_err());
        manager.git(&a.path, &["add", "new.txt"]).await?;
        manager
            .git(
                &a.path,
                &[
                    "-c",
                    "user.name=Fixture",
                    "-c",
                    "user.email=fixture@localhost",
                    "commit",
                    "-m",
                    "worker result",
                ],
            )
            .await?;
        assert_eq!(
            manager.diff(&a).await?,
            patch,
            "committed changes must remain in the delivered patch"
        );
        manager.apply_dependency_patch(&b, &patch).await?;
        assert_eq!(
            std::fs::read_to_string(b.path.join("new.txt"))?,
            "isolated\n"
        );
        Ok(())
    }
}
