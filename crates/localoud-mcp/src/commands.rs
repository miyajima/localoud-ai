//! Explicitly opted-in host execution. The cwd check is not an OS sandbox.
use crate::{safe_path, Arguments, Result};
use serde_json::{json, Value};
use std::{
    io,
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};
use tokio::io::{AsyncRead, AsyncReadExt};

const CAPTURE_BYTES: usize = 16 * 1024;
const DEFAULT_TIMEOUT_MS: u64 = 30_000;
const MAX_TIMEOUT_MS: u64 = 120_000;

fn command_directory(root: &Path, cwd: Option<&str>) -> Result<PathBuf> {
    let cwd = cwd.unwrap_or("");
    if cwd.is_empty() || cwd == "." {
        return Ok(root.to_path_buf());
    }
    if cwd.len() > 4096 || !safe_path(cwd) {
        return Err("cwd must be a relative publishable project directory".into());
    }
    let mut directory = root.to_path_buf();
    for part in Path::new(cwd).components() {
        directory.push(part);
        let meta = std::fs::symlink_metadata(&directory).map_err(|e| e.to_string())?;
        if meta.file_type().is_symlink() || !meta.is_dir() {
            return Err("cwd must be a directory without symlinks".into());
        }
    }
    let canonical = directory.canonicalize().map_err(|e| e.to_string())?;
    if !canonical.starts_with(root) {
        return Err("cwd is outside the project".into());
    }
    Ok(canonical)
}

#[derive(Default)]
struct Capture {
    bytes: Vec<u8>,
    truncated: bool,
}

impl Capture {
    async fn read(&mut self, mut stream: impl AsyncRead + Unpin) -> io::Result<()> {
        let mut buffer = [0; 4096];
        loop {
            let n = stream.read(&mut buffer).await?;
            if n == 0 {
                return Ok(());
            }
            let keep = n.min(CAPTURE_BYTES.saturating_sub(self.bytes.len()));
            self.bytes.extend_from_slice(&buffer[..keep]);
            self.truncated |= keep < n;
            // Continue draining both pipes after the cap so the process cannot
            // deadlock on a full stdout/stderr pipe. Memory remains bounded.
        }
    }

    fn text(&mut self) -> (String, bool) {
        let original = String::from_utf8_lossy(&self.bytes);
        let mut text = hub_policy::redact(&original);
        let redacted = text != original;
        // Escaped control characters can expand sixfold in JSON. Keep each
        // serialized output below 24KiB, leaving room for the other pipe/receipt.
        while serde_json::to_string(&text)
            .expect("string serialization")
            .len()
            > 24 * 1024
        {
            let mut end = text.len() / 2;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            text.truncate(end);
            self.truncated = true;
        }
        (text, redacted)
    }
}

#[cfg(unix)]
struct ProcessGroup(Option<u32>);

#[cfg(unix)]
impl ProcessGroup {
    fn terminate(&mut self) {
        // Negative pid targets the dedicated process group, including ordinary
        // descendants. This also runs when an HTTP request/future is cancelled.
        // A deliberately detached process can escape; this is not a sandbox.
        // Never signal the same numeric group again after it has been reaped;
        // output draining can outlive the group and its ID could be reused.
        if let Some(pid) = self.0.take() {
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
        }
    }
}

#[cfg(unix)]
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        self.terminate();
    }
}

#[cfg(unix)]
pub(super) async fn run(root: &Path, a: &Arguments) -> Result<Value> {
    use std::os::unix::process::ExitStatusExt;
    let program = a.program.as_deref().ok_or("program is required")?;
    let args = a.args.as_deref().unwrap_or(&[]);
    let timeout_ms = a.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS);
    if program.is_empty()
        || program.len() > 4096
        || program.contains('\0')
        || args.len() > 256
        || args.iter().any(|arg| arg.contains('\0'))
        || args.iter().map(String::len).sum::<usize>() + program.len() > 32 * 1024
        || !(1..=MAX_TIMEOUT_MS).contains(&timeout_ms)
    {
        return Err("Use a program and up to 256 literal args within 32KiB, without NUL; timeout_ms must be 1..120000".into());
    }
    if a.path.is_some()
        || a.query.is_some()
        || a.base_revision.is_some()
        || a.start_line.is_some()
        || a.line_count.is_some()
        || a.offset.is_some()
        || a.limit.is_some()
        || a.content.is_some()
        || a.expected_sha256.is_some()
        || a.old_text.is_some()
        || a.new_text.is_some()
    {
        return Err("Unexpected arguments for command_run".into());
    }
    let cwd = command_directory(root, a.cwd.as_deref())?;
    let mut command = tokio::process::Command::new(program);
    command
        .args(args)
        .current_dir(&cwd)
        .env_clear()
        .env(
            "PATH",
            std::env::var_os("PATH").unwrap_or_else(|| "/usr/bin:/bin".into()),
        )
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .kill_on_drop(true);
    // Do not forward the MCP token, API keys or other ambient credentials.
    // HOME still allows normal tool configuration; opt-in grants host privileges.
    for key in ["HOME", "TMPDIR", "LANG", "LC_ALL"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    let started = Instant::now();
    let mut child = command
        .spawn()
        .map_err(|e| format!("cannot start command: {e}"))?;
    let mut group = ProcessGroup(Some(child.id().ok_or("missing command pid")?));
    let stdout = child.stdout.take().ok_or("missing stdout pipe")?;
    let stderr = child.stderr.take().ok_or("missing stderr pipe")?;
    let mut out = Capture::default();
    let mut error = Capture::default();
    let completed = tokio::time::timeout(Duration::from_millis(timeout_ms), async {
        let wait = async {
            let status = child.wait().await;
            // Stop descendants on direct-child exit, before pipe EOF: those
            // descendants may otherwise keep both pipes open indefinitely.
            group.terminate();
            status
        };
        tokio::try_join!(wait, out.read(stdout), error.read(stderr))
    })
    .await;
    // Stop background children even if their parent exited successfully.
    drop(group);
    let (status, timed_out) = match completed {
        Ok(Ok((status, (), ()))) => (status, false),
        Ok(Err(e)) => {
            let _ = child.wait().await;
            return Err(format!("command output unavailable: {e}"));
        }
        Err(_) => (child.wait().await.map_err(|e| e.to_string())?, true),
    };
    let (stdout, out_redacted) = out.text();
    let (stderr, err_redacted) = error.text();
    Ok(json!({
        "operation_id": uuid::Uuid::new_v4().to_string(),
        "project_id": a.project_id,
        "cwd": cwd,
        "exit_code": status.code(),
        "signal": status.signal(),
        "success": status.success() && !timed_out,
        "timed_out": timed_out,
        "timeout_ms": timeout_ms,
        "duration_ms": started.elapsed().as_millis() as u64,
        "stdout": stdout,
        "stderr": stderr,
        "output_truncated": out.truncated || error.truncated,
        "output_redacted": out_redacted || err_redacted
    }))
}

#[cfg(not(unix))]
pub(super) async fn run(_root: &Path, _a: &Arguments) -> Result<Value> {
    Err("command_run currently requires Unix process-group support".into())
}
