//! Explicit Manifest handoff and durable CLI-backed workers; no browser transport.
//! No automatic resume: a lost runner always requires reconciliation.
use crate::AppState;
use hub_core::{HubThreadId, ProjectId, TaskId, ThreadMapping, Worktree};
use hub_router::automatic::{AutoSettings, ModelProvider, ModelTarget};
use hub_runtime::local_worker::{collect_files, PreparedEdits};
use hub_worktree::GitWorktrees;
use protocol_types::{CodingAgentProvider, EventDetails, Message, ProviderThread, ProviderTurn};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    path::{Component, Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::Duration,
};
use tauri::Manager;

type Result<T> = std::result::Result<T, String>;
const CAPSULE_LIMIT: usize = 48 * 1024;
const REVIEW_LIMIT: usize = 512 * 1024;
const DEADLINE: Duration = Duration::from_secs(1800);
static LIVE: OnceLock<Mutex<HashSet<HubThreadId>>> = OnceLock::new();
struct LiveGuard(HubThreadId);
impl Drop for LiveGuard {
    fn drop(&mut self) {
        if let Ok(mut ids) = live().lock() {
            ids.remove(&self.0);
        }
    }
}
fn live() -> &'static Mutex<HashSet<HubThreadId>> {
    LIVE.get_or_init(Default::default)
}
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AutonomousRequest {
    pub project_id: String,
    pub text: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanStep {
    pub key: String,
    pub title: String,
    pub goal: String,
    pub dependencies: Vec<String>,
    pub level: u8,
    #[serde(alias = "files")]
    pub owned_paths: Vec<String>,
    pub acceptance: Vec<String>,
    /// Mandatory explicit scope for local execution; absence fails closed.
    #[serde(default)]
    pub risk: Option<String>,
    #[serde(default)]
    pub estimated_loc: Option<u32>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Plan {
    steps: Vec<PlanStep>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StepState {
    pub task_id: TaskId,
    pub step: PlanStep,
    pub target: ModelTarget,
    pub status: String,
    pub capsule: Option<String>,
    pub dispatch_phase: Option<String>,
    pub worktree: Option<Worktree>,
    pub input_tree: Option<String>,
    pub child_id: Option<HubThreadId>,
    pub output: Option<String>,
    /// Incremental patch against input_tree, excluding dependency patches.
    pub patch: Option<String>,
    pub verification: Vec<Value>,
    pub usage: Vec<Value>,
    pub error: Option<String>,
    #[serde(default)]
    pub revision_instruction: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Child {
    pub id: HubThreadId,
    pub role: String,
    pub provider: String,
    pub provider_thread: Option<ProviderThread>,
    pub turn: Option<ProviderTurn>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AutonomousSnapshot {
    pub thread: ThreadMapping,
    pub request: AutonomousRequest,
    pub source_root: PathBuf,
    pub source_head: String,
    pub settings_snapshot: AutoSettings,
    #[serde(default)]
    pub manifest: Option<crate::manifest::TaskManifest>,
    #[serde(default)]
    pub artifact_version: Option<String>,
    #[serde(default)]
    pub iteration: u32,
    #[serde(default)]
    pub history: Vec<IterationRecord>,
    pub steps: Vec<StepState>,
    pub children: Vec<Child>,
    pub messages: Vec<Message>,
    pub stop_requested: bool,
    pub artifact: Option<Worktree>,
    pub final_diff: Option<String>,
    pub review: Option<Value>,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IterationRecord {
    pub iteration: u32,
    pub artifact: Option<Worktree>,
    pub artifact_version: Option<String>,
    pub final_diff: Option<String>,
    pub steps: Vec<StepState>,
    pub review: Option<Value>,
}
pub fn artifact_version(source_head: &str, diff: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(source_head.as_bytes());
    h.update(b"\0");
    h.update(diff.as_bytes());
    format!("{:x}", h.finalize())
}
fn key(id: HubThreadId) -> String {
    format!("autonomous:{id}")
}
pub(crate) fn read(state: &AppState, id: HubThreadId) -> Result<AutonomousSnapshot> {
    let s = state.store.lock().map_err(err)?;
    serde_json::from_str(
        &s.setting(&key(id))
            .map_err(err)?
            .ok_or("autonomous workflow not found")?,
    )
    .map_err(err)
}
/// Store lock covers read/modify/write, so parallel completions cannot erase stops.
fn update(
    state: &AppState,
    id: HubThreadId,
    f: impl FnOnce(&mut AutonomousSnapshot),
) -> Result<AutonomousSnapshot> {
    let s = state.store.lock().map_err(err)?;
    let mut w: AutonomousSnapshot = serde_json::from_str(
        &s.setting(&key(id))
            .map_err(err)?
            .ok_or("workflow not found")?,
    )
    .map_err(err)?;
    f(&mut w);
    s.set_setting(&key(id), &serde_json::to_string(&w).map_err(err)?)
        .map_err(err)?;
    s.save_thread(&w.thread).map_err(err)?;
    Ok(w)
}
fn check(state: &AppState, id: HubThreadId) -> Result<()> {
    if read(state, id)?.stop_requested {
        Err("workflow stopped; reconcile dispatched children before retrying".into())
    } else {
        Ok(())
    }
}
fn terminal(status: &str) -> bool {
    matches!(
        status,
        "completed"
            | "failed"
            | "interrupted"
            | "reconciliation_required"
            | "review_rejected"
            | "awaiting_review"
            | "legacy_read_only"
    )
}
fn parse_id(id: &str) -> Result<HubThreadId> {
    Ok(HubThreadId(id.parse().map_err(err)?))
}
pub(crate) fn valid_path(p: &str) -> bool {
    !p.is_empty()
        && !p.contains('\\')
        && !p.contains('\0')
        && !p.ends_with('/')
        && p.split('/').all(|c| {
            !c.is_empty() && c != "." && c != ".." && c != ".git" && c != ".agent-worktrees"
        })
        && Path::new(p)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}
fn overlap(a: &str, b: &str) -> bool {
    Path::new(a).starts_with(b) || Path::new(b).starts_with(a)
}
fn conflict(a: &PlanStep, b: &PlanStep) -> bool {
    a.owned_paths
        .iter()
        .any(|x| b.owned_paths.iter().any(|y| overlap(x, y)))
}
pub(crate) fn parse_plan(text: &str) -> Result<Vec<PlanStep>> {
    if text.len() > CAPSULE_LIMIT {
        return Err("plan exceeds 48 KB".into());
    }
    let plan: Plan = serde_json::from_str(text.trim()).map_err(err)?;
    if plan.steps.is_empty() || plan.steps.len() > 8 {
        return Err("plan requires 1..8 steps".into());
    }
    let mut keys = HashSet::new();
    for s in &plan.steps {
        if s.key.is_empty()
            || s.key.len() > 64
            || !s
                .key
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
            || !keys.insert(s.key.clone())
            || s.title.trim().is_empty()
            || s.goal.trim().is_empty()
            || !(1..=5).contains(&s.level)
            || s.owned_paths.is_empty()
            || s.owned_paths.len() > 32
            || !s.owned_paths.iter().all(|p| valid_path(p))
            || s.acceptance.is_empty()
            || s.acceptance.iter().any(|a| a.trim().is_empty())
        {
            return Err("invalid step key, level, ownership or acceptance".into());
        }
    }
    let mut done = HashSet::new();
    let mut ordered = Vec::new();
    while ordered.len() < plan.steps.len() {
        let next = plan
            .steps
            .iter()
            .find(|s| !done.contains(&s.key) && s.dependencies.iter().all(|d| done.contains(d)));
        let s = next.ok_or("cyclic plan or unknown dependency")?;
        if s.dependencies.iter().collect::<HashSet<_>>().len() != s.dependencies.len() {
            return Err("duplicate dependency".into());
        }
        done.insert(s.key.clone());
        ordered.push(s.clone());
    }
    // Overlapping tasks need serial artifact visibility even if omitted by planner.
    for i in 0..ordered.len() {
        for j in 0..i {
            if conflict(&ordered[i], &ordered[j])
                && !ordered[i].dependencies.contains(&ordered[j].key)
            {
                let dependency = ordered[j].key.clone();
                ordered[i].dependencies.push(dependency);
            }
        }
    }
    Ok(ordered)
}
fn ready_batch(steps: &[StepState]) -> Vec<usize> {
    let mut selected: Vec<usize> = Vec::new();
    for (i, s) in steps.iter().enumerate() {
        if s.status != "pending"
            || !s.step.dependencies.iter().all(|d| {
                steps
                    .iter()
                    .any(|p| &p.step.key == d && p.status == "completed")
            })
        {
            continue;
        }
        if selected.iter().all(|j| !conflict(&s.step, &steps[*j].step)) {
            selected.push(i);
        }
        if selected.len() == 3 {
            break;
        }
    }
    selected
}
fn ancestors(steps: &[StepState], i: usize) -> Vec<usize> {
    let mut needed: HashSet<String> = steps[i].step.dependencies.iter().cloned().collect();
    for s in steps.iter().rev() {
        if needed.contains(&s.step.key) {
            needed.extend(s.step.dependencies.clone());
        }
    }
    steps
        .iter()
        .enumerate()
        .filter_map(|(j, s)| needed.contains(&s.step.key).then_some(j))
        .collect()
}
fn capsule(w: &AutonomousSnapshot, i: usize) -> Result<String> {
    let dependencies: Vec<_> = w.steps[i]
        .step
        .dependencies
        .iter()
        .map(|d| {
            let s = w
                .steps
                .iter()
                .find(|s| &s.step.key == d)
                .ok_or("missing dependency")?;
            if s.status != "completed" {
                return Err("dependency did not succeed");
            }
            Ok(json!({"key":d,"outcome":s.output,"verification":s.verification}))
        })
        .collect::<std::result::Result<_, &str>>()
        .map_err(err)?;
    let payload = json!({"task":w.steps[i].step,"target":w.steps[i].target,"dependencies":dependencies,
        "overall_acceptance":w.manifest.as_ref().map(|m| &m.acceptance),"revision_instruction":w.steps[i].revision_instruction,
        "iteration":w.iteration,"worktree_note":"Each iteration has a new worktree containing only base sources and current dependency outputs. Reimplement your step here; your prior own patch is not pre-applied. Never write to a previous worktree.",
        "instructions":"Execute only this task in the supplied isolated worktree. Read repository sources as needed. No subagents, no commit, merge, push, external publication or changes outside owned_paths. Run acceptance checks and report their exact results. Do not claim unrun tests passed. No parent conversation is available."}).to_string();
    bounded(payload, CAPSULE_LIMIT)
}
fn bounded(text: String, limit: usize) -> Result<String> {
    if text.len() > limit {
        Err(format!(
            "context exceeds {limit} bytes; no truncated dispatch"
        ))
    } else if hub_policy::redact(&text) != text {
        Err("context contains secret-like material; inspect locally".into())
    } else {
        Ok(text)
    }
}
async fn git(root: &Path, args: &[&str]) -> Result<String> {
    let mut command = tokio::process::Command::new("git");
    command
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .kill_on_drop(true);
    let out = tokio::time::timeout(Duration::from_secs(30), command.output())
        .await
        .map_err(err)?
        .map_err(err)?;
    if !out.status.success() {
        return Err(format!(
            "git failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    String::from_utf8(out.stdout).map_err(err)
}
async fn index_tree(w: &Worktree) -> Result<String> {
    git(&w.path, &["add", "-A", "--", "."]).await?;
    Ok(git(&w.path, &["write-tree"]).await?.trim().into())
}

pub async fn import_task(
    manifest: crate::manifest::TaskManifest,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<ThreadMapping> {
    bounded(manifest.request.clone(), CAPSULE_LIMIT)?;
    let request = AutonomousRequest {
        project_id: manifest.project_id.clone(),
        text: manifest.request.clone(),
    };
    let plan = parse_plan(&json!({"steps":manifest.steps}).to_string())?;
    let project_id = ProjectId(request.project_id.parse().map_err(err)?);
    let (root, settings) = {
        let s = state.store.lock().map_err(err)?;
        let root = s
            .projects()
            .map_err(err)?
            .into_iter()
            .find(|p| p.id == project_id)
            .ok_or("project not found")?
            .root;
        let settings: AutoSettings = serde_json::from_str(
            &s.setting("auto_routing_settings")
                .map_err(err)?
                .ok_or("save all five routing assignments first")?,
        )
        .map_err(err)?;
        settings.validate().map_err(err)?;
        (root, settings)
    };
    let root = root.canonicalize().map_err(err)?;
    let top = git(&root, &["rev-parse", "--show-toplevel"]).await?;
    if Path::new(top.trim()).canonicalize().map_err(err)? != root {
        return Err("project must point to the repository root for unambiguous ownership".into());
    }
    if !git(
        &root,
        &[
            "status",
            "--porcelain=v1",
            "--untracked-files=all",
            "--",
            ".",
            ":!.agent-worktrees",
        ],
    )
    .await?
    .is_empty()
    {
        return Err(
            "source checkout is dirty; commit or isolate its inputs before Manifest execution"
                .into(),
        );
    }
    let head = git(&root, &["rev-parse", "--verify", "HEAD^{commit}"])
        .await?
        .trim()
        .to_string();
    if head != manifest.base_revision {
        return Err("Manifestの基準revisionが現在のHEADと一致しません".into());
    }
    let mut steps = Vec::new();
    for step in plan {
        let target = settings.target(step.level).map_err(err)?;
        steps.push(StepState {
            task_id: TaskId::default(),
            step,
            target,
            status: "pending".into(),
            capsule: None,
            dispatch_phase: None,
            worktree: None,
            input_tree: None,
            child_id: None,
            output: None,
            patch: None,
            verification: vec![],
            usage: vec![],
            error: None,
            revision_instruction: None,
        });
    }
    let id = HubThreadId::default();
    let thread = ThreadMapping {
        id,
        project_id,
        provider: "autonomous".into(),
        provider_thread_id: format!("autonomous:{id}"),
        title: request.text.chars().take(80).collect(),
        status: "queued".into(),
    };
    let w = AutonomousSnapshot {
        thread: thread.clone(),
        request: request.clone(),
        source_root: root,
        source_head: head,
        settings_snapshot: settings,
        manifest: Some(manifest.clone()),
        artifact_version: None,
        iteration: 1,
        history: vec![],
        steps,
        children: vec![],
        messages: vec![Message {
            role: "user".into(),
            text: request.text,
        }],
        stop_requested: false,
        artifact: None,
        final_diff: None,
        review: None,
        error: None,
    };
    {
        let mut store = state.store.lock().map_err(err)?;
        store
            .accept_manifest(
                &manifest.manifest_id,
                &json!({"manifest":manifest,"thread_id":id}).to_string(),
                &thread,
                &key(id),
                &serde_json::to_string(&w).map_err(err)?,
            )
            .map_err(err)?;
    }
    launch(app, id)?;
    Ok(thread)
}
fn launch(app: tauri::AppHandle, id: HubThreadId) -> Result<()> {
    live().lock().map_err(err)?.insert(id);
    tauri::async_runtime::spawn(async move {
        let _live_guard = LiveGuard(id);
        let state = app.state::<AppState>();
        let result = guarded(&state, id, run(&app, id)).await;
        if let Err(e) = result {
            let _ = update(&state, id, |w| {
                w.thread.status = if w.stop_requested {
                    "interrupted"
                } else {
                    "reconciliation_required"
                }
                .into();
                w.error = Some(e.clone());
                for s in &mut w.steps {
                    if s.status == "pending" {
                        s.status = "blocked".into();
                    } else if s.status == "running" {
                        s.status = "interrupted".into();
                    }
                }
                w.messages.push(Message {
                    role: "assistant".into(),
                    text: e,
                });
            });
            if let Err(stop_error) = interrupt_children(&state, id).await {
                let _ = update(&state, id, |w| {
                    w.thread.status = "reconciliation_required".into();
                    let previous = w.error.take().unwrap_or_default();
                    w.error = Some(format!(
                        "{previous}\nChild stop not confirmed: {stop_error}"
                    ));
                });
            }
        }
    });
    Ok(())
}

fn prepare_review(
    w: &mut AutonomousSnapshot,
    review: &crate::manifest::ReviewManifest,
) -> Result<bool> {
    use crate::manifest::Verdict;
    if w.manifest.is_none()
        || w.thread.project_id.to_string() != review.project_id
        || w.thread.id.to_string() != review.task_id
    {
        return Err("Review Manifestの対象が一致しません".into());
    }
    if !matches!(
        w.thread.status.as_str(),
        "awaiting_review" | "review_rejected" | "completed"
    ) || w.artifact_version.as_deref() != Some(&review.artifact_version)
    {
        return Err("レビュー対象の版またはタスク状態が変わっています".into());
    }
    let mut affected: HashSet<String> = review.changes.iter().map(|c| c.step_key.clone()).collect();
    if affected
        .iter()
        .any(|key| !w.steps.iter().any(|s| &s.step.key == key))
    {
        return Err("Review Manifestに不明なstepがあります".into());
    }
    if review.verdict == Verdict::Pass {
        evidence_complete(&w.steps)?;
    }
    w.review = Some(json!(review));
    w.messages.push(Message {
        role: "assistant".into(),
        text: format!(
            "取り込んだレビュー: {:?}\n{}",
            review.verdict, review.summary
        ),
    });
    if affected.is_empty() {
        w.thread.status = if review.verdict == Verdict::Pass {
            "completed"
        } else {
            "review_rejected"
        }
        .into();
        return Ok(false);
    }
    // A dependent step must re-evaluate its work against the corrected inputs.
    for s in &w.steps {
        if s.step.dependencies.iter().any(|d| affected.contains(d)) {
            affected.insert(s.step.key.clone());
        }
    }
    w.history.push(IterationRecord {
        iteration: w.iteration,
        artifact: w.artifact.clone(),
        artifact_version: w.artifact_version.clone(),
        final_diff: w.final_diff.clone(),
        steps: w.steps.clone(),
        review: w.review.clone(),
    });
    for s in &mut w.steps {
        if !affected.contains(&s.step.key) {
            continue;
        }
        s.revision_instruction = Some(review.changes.iter().find(|c| c.step_key == s.step.key)
            .map(|c| c.instruction.clone()).unwrap_or_else(|| "Upstream work was corrected. Reimplement and verify this step against the new dependency artifacts.".into()));
        s.task_id = TaskId::default(); // preserve the old worktree and keep the provider session
        s.status = "pending".into();
        s.worktree = None;
        s.input_tree = None;
        s.capsule = None;
        s.dispatch_phase = None;
        s.output = None;
        s.patch = None;
        s.verification.clear();
        s.usage.clear();
        s.error = None;
        if s.target.provider == ModelProvider::Local {
            s.child_id = None;
        }
    }
    w.iteration += 1;
    w.artifact = None;
    w.artifact_version = None;
    w.final_diff = None;
    w.review = None;
    w.error = None;
    w.stop_requested = false;
    w.thread.status = "queued".into();
    Ok(true)
}

pub async fn import_review(
    review: crate::manifest::ReviewManifest,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<ThreadMapping> {
    let id = parse_id(&review.task_id)?;
    if live().lock().map_err(err)?.contains(&id) {
        return Err("workerの終了を待ってください".into());
    }
    let mut w = read(&state, id)?;
    let artifact = w
        .artifact
        .as_ref()
        .ok_or("レビュー対象の統合成果物がありません")?;
    let manager = GitWorktrees::new(&w.source_root).map_err(err)?;
    let diff = manager.diff(artifact).await.map_err(err)?;
    if git(&artifact.path, &["rev-parse", "HEAD"]).await?.trim() != w.source_head
        || artifact_version(&w.source_head, &diff) != review.artifact_version
    {
        return Err(
            "成果物がレビュー後に変更されています。新しい版を再レビューしてください".into(),
        );
    }
    let start = prepare_review(&mut w, &review)?;
    {
        let mut store = state.store.lock().map_err(err)?;
        store
            .accept_manifest(
                &review.manifest_id,
                &json!({"manifest":review,"thread_id":id}).to_string(),
                &w.thread,
                &key(id),
                &serde_json::to_string(&w).map_err(err)?,
            )
            .map_err(err)?;
    }
    if start {
        launch(app, id)?;
    }
    Ok(w.thread)
}
#[tauri::command]
pub async fn autonomous_snapshot(
    thread_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<AutonomousSnapshot> {
    let id = parse_id(&thread_id)?;
    let w = read(&state, id)?;
    if w.manifest.is_none() {
        let mut legacy = w;
        legacy.thread.status = "legacy_read_only".into();
        legacy.error =
            Some("旧ブラウザ方式の記録です。再送せず、新しいManifestで開始してください。".into());
        return Ok(legacy);
    }
    let runner_lost = !terminal(&w.thread.status) && !live().lock().map_err(err)?.contains(&id);
    if runner_lost {
        update(&state, id, |w| {
            w.stop_requested = true;
            w.thread.status = "reconciliation_required".into();
            w.error = Some("runner was lost/restarted; no child send will be replayed; inspect preserved artifacts".into());
            for s in &mut w.steps {
                if s.status == "running" {
                    s.status = "interrupted".into();
                } else if s.status == "pending" {
                    s.status = "blocked".into();
                }
            }
        })?;
        if let Err(e) = interrupt_children(&state, id).await {
            update(&state, id, |w| {
                w.error = Some(format!("runner lost; child reconciliation required: {e}"))
            })?;
        }
        return read(&state, id);
    }
    Ok(w)
}
#[derive(Debug, Serialize)]
pub struct WorkerActivity {
    child_id: HubThreadId,
    events: Vec<hub_events::JournalEvent>,
}
#[derive(Debug, Serialize)]
pub struct WorktreeChange {
    key: String,
    title: String,
    path: PathBuf,
    diff: Option<String>,
    error: Option<String>,
}
#[derive(Debug, Serialize)]
pub struct AutonomousActivity {
    workers: Vec<WorkerActivity>,
    changes: Vec<WorktreeChange>,
}

async fn worktree_changes(w: &AutonomousSnapshot) -> Result<Vec<WorktreeChange>> {
    let manager = GitWorktrees::new(&w.source_root).map_err(err)?;
    let targets: Vec<_> = if let Some(artifact) = &w.artifact {
        vec![("integrated".to_owned(), "統合した成果物".to_owned(), artifact)]
    } else {
        w.steps.iter().filter_map(|s| s.worktree.as_ref().map(|tree|
            (s.step.key.clone(), s.step.title.clone(), tree))).collect()
    };
    let mut changes = vec![];
    for (key, title, tree) in targets {
        // GitWorktrees validates ownership and includes bounded untracked files.
        // A failed read remains visible; it must not look like an empty diff.
        let (diff, error) = match manager.diff(tree).await {
            Ok(diff) => (Some(diff), None),
            Err(e) => (None, Some(format!("{e:#}"))),
        };
        changes.push(WorktreeChange { key, title, path: tree.path.clone(), diff, error });
    }
    Ok(changes)
}

#[tauri::command]
pub async fn autonomous_activity(
    thread_id: String,
    include_changes: bool,
    state: tauri::State<'_, AppState>,
) -> Result<AutonomousActivity> {
    let w = read(&state, parse_id(&thread_id)?)?;
    let mut workers = vec![];
    {
        let store = state.store.lock().map_err(err)?;
        for step in &w.steps {
            let Some(child_id) = step.child_id else { continue };
            let turn = w.children.iter().find(|c| c.id == child_id).and_then(|c| c.turn.as_ref());
            let mut events = vec![];
            for (sequence, body) in store.recent_activity(child_id, turn.map(|t| t.id.as_str())).map_err(err)? {
                let mut event: protocol_types::AgentEvent = serde_json::from_str(&body).map_err(err)?;
                if event.text.chars().count() > 6000 {
                    event.text = event.text.chars().take(6000).collect::<String>() + "\n…（表示を省略）";
                }
                // The readable command and event text suffice for this panel;
                // do not duplicate large tool argument payloads in every poll.
                if !matches!(event.details, Some(EventDetails::Command { .. })) { event.details = None; }
                events.push(hub_events::JournalEvent { sequence, event });
            }
            workers.push(WorkerActivity { child_id, events });
        }
    }
    let changes = if include_changes { worktree_changes(&w).await? } else { vec![] };
    Ok(AutonomousActivity { workers, changes })
}

#[tauri::command]
pub async fn autonomous_stop(
    thread_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<AutonomousSnapshot> {
    let id = parse_id(&thread_id)?;
    let current = read(&state, id)?;
    if matches!(
        current.thread.status.as_str(),
        "completed" | "review_rejected" | "awaiting_review" | "legacy_read_only"
    ) {
        return Ok(current);
    }
    update(&state, id, |w| {
        w.stop_requested = true;
        if !terminal(&w.thread.status) {
            w.thread.status = "stopping".into();
        }
    })?;
    // The live runner owns cleanup after dropping all in-flight task futures.
    // Avoid racing it with a second interrupt of the same browser/CLI turn.
    if live().lock().map_err(err)?.contains(&id) {
        return read(&state, id);
    }
    let failures = interrupt_children(&state, id).await;
    if let Err(e) = failures {
        update(&state, id, |w| {
            w.thread.status = "reconciliation_required".into();
            w.error = Some(e);
        })?;
    } else {
        update(&state, id, |w| {
            if w.thread.status == "stopping" {
                w.thread.status = "interrupted".into();
            }
        })?;
    }
    read(&state, id)
}
fn prepare_resume(w: &mut AutonomousSnapshot) -> Result<()> {
    if w.manifest.is_none()
        || !matches!(
            w.thread.status.as_str(),
            "interrupted" | "failed" | "reconciliation_required"
        )
    {
        return Err("再開できる中断済みManifestタスクではありません".into());
    }
    w.history.push(IterationRecord {
        iteration: w.iteration,
        artifact: w.artifact.clone(),
        artifact_version: w.artifact_version.clone(),
        final_diff: w.final_diff.clone(),
        steps: w.steps.clone(),
        review: w.review.clone(),
    });
    for s in &mut w.steps {
        if s.status == "completed" {
            continue;
        }
        s.task_id = TaskId::default();
        s.status = "pending".into();
        s.worktree = None;
        s.input_tree = None;
        s.capsule = None;
        s.dispatch_phase = None;
        s.output = None;
        s.patch = None;
        s.verification.clear();
        s.usage.clear();
        s.error = None;
        if s.target.provider == ModelProvider::Local {
            s.child_id = None;
        }
        s.revision_instruction=Some(format!("Previous attempt was interrupted or failed. Reimplement this step in this fresh worktree and verify it. {}",s.revision_instruction.as_deref().unwrap_or("")));
    }
    w.iteration += 1;
    w.artifact = None;
    w.artifact_version = None;
    w.final_diff = None;
    w.review = None;
    w.error = None;
    w.stop_requested = false;
    w.thread.status = "queued".into();
    Ok(())
}
#[tauri::command]
pub async fn autonomous_resume(
    thread_id: String,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<AutonomousSnapshot> {
    let _lock = state.workflow_lock.lock().await;
    let id = parse_id(&thread_id)?;
    if live().lock().map_err(err)?.contains(&id) {
        return Err("実行中です".into());
    }
    let mut w = read(&state, id)?;
    if w.manifest.is_none() {
        return Err("旧方式の記録は閲覧専用です".into());
    }
    prepare_resume(&mut w)?;
    interrupt_children(&state, id).await?;
    update(&state, id, |current| *current = w.clone())?;
    launch(app, id)?;
    Ok(w)
}
async fn interrupt_children(state: &AppState, id: HubThreadId) -> Result<()> {
    let mut failures = Vec::new();
    for c in read(state, id)?.children {
        let child_id = c.id;
        let result = tokio::time::timeout(Duration::from_secs(10), interrupt_child(state, c)).await;
        match result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => failures.push(format!("{child_id}: {e}")),
            Err(_) => failures.push(format!(
                "{child_id}: stop acknowledgment timed out; reconcile"
            )),
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n"))
    }
}
async fn interrupt_child(state: &AppState, c: Child) -> Result<()> {
    let status = state
        .store
        .lock()
        .map_err(err)?
        .threads()
        .map_err(err)?
        .into_iter()
        .find(|t| t.id == c.id)
        .map(|t| t.status);
    if status.as_deref() == Some("completed") {
        return Ok(());
    }
    if c.provider == "chatgpt" {
        return Err("旧ブラウザ連携は廃止済みです。再接続・再送は行いません".into());
    }
    if c.provider == "codex" {
        // Never launch/reconnect a provider while stopping a lost runner.
        let provider = state
            .connection
            .lock()
            .await
            .as_ref()
            .map(|c| c.provider.clone());
        let (Some(p), Some(t)) = (provider, c.provider_thread) else {
            return Err("CLI child cannot be reached; reconciliation required".into());
        };
        let turn = match c.turn {
            Some(turn) => Some(turn),
            None => p.read_thread(&t).await.map_err(err)?.active_turn,
        };
        if let Some(turn) = turn {
            p.reconcile_stopped_turn(&turn).await.map_err(err)?;
        } else {
            return Err("child turn/start outcome is ambiguous; no replay or assumed stop".into());
        }
    }
    // Local generation is cancelled by guard_with; local apply checks stop first.
    if c.provider == "local" {
        let store = state.store.lock().map_err(err)?;
        if let Some(mut mapping) = store
            .threads()
            .map_err(err)?
            .into_iter()
            .find(|t| t.id == c.id)
        {
            mapping.status = "interrupted".into();
            store.save_thread(&mapping).map_err(err)?;
        }
    }
    Ok(())
}
/// Dropping an in-flight send is ambiguous; caller persists an interrupted state.
async fn guarded<T>(
    state: &AppState,
    id: HubThreadId,
    future: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
    guard_with(|| check(state, id), future).await
}
async fn guard_with<T>(
    poll: impl Fn() -> Result<()>,
    future: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
    poll()?;
    tokio::pin!(future);
    let deadline = tokio::time::sleep(DEADLINE);
    tokio::pin!(deadline);
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    loop {
        tokio::select! {
            biased;
            _ = tick.tick() => poll()?,
            _ = &mut deadline => return Err("workflow timeout; dispatch may have happened; no automatic replay".into()),
            result = &mut future => { poll()?; return result; }
        }
    }
}
async fn run(app: &tauri::AppHandle, id: HubThreadId) -> Result<()> {
    let state = app.state::<AppState>();
    let initial = read(&state, id)?;
    if initial.manifest.is_none() {
        return Err("旧ブラウザタスクは実行できません".into());
    }
    // Validate every used frozen route before the first worker.
    let levels: std::collections::BTreeSet<_> =
        initial.steps.iter().map(|s| s.step.level).collect();
    for level in levels {
        check(&state, id)?;
        crate::models::validate_target(
            &initial.settings_snapshot.target(level).map_err(err)?,
            &state,
        )
        .await?;
    }
    if git(&initial.source_root, &["rev-parse", "HEAD"])
        .await?
        .trim()
        != initial.source_head
        || !git(
            &initial.source_root,
            &[
                "status",
                "--porcelain=v1",
                "--untracked-files=all",
                "--",
                ".",
                ":!.agent-worktrees",
            ],
        )
        .await?
        .is_empty()
    {
        return Err("source changed during planning; no worker dispatched".into());
    }
    for s in &read(&state, id)?.steps {
        if s.target.provider == ModelProvider::Local {
            local_scope(&s.step)?;
            if state.local.provider.model_id() != s.target.model {
                return Err("local provider model ID differs from frozen route".into());
            }
            collect_files(&initial.source_root, &s.step.owned_paths).map_err(err)?;
        }
    }
    // Initialize once: concurrent first-run create_dir calls would race.
    GitWorktrees::new(&initial.source_root).map_err(err)?;
    update(&state, id, |w| w.thread.status = "running".into())?;
    loop {
        check(&state, id)?;
        let w = read(&state, id)?;
        if w.steps.iter().all(|s| s.status == "completed") {
            break;
        }
        let batch = ready_batch(&w.steps);
        if batch.is_empty() {
            return Err("no runnable tasks; failed/missing dependency evidence".into());
        }
        let mut jobs = tokio::task::JoinSet::new();
        for i in batch {
            let app = app.clone();
            update(&state, id, |w| w.steps[i].status = "running".into())?;
            jobs.spawn(async move {
                let state = app.state::<AppState>();
                let result = guarded(&state, id, worker(&state, id, i)).await;
                if let Err(e) = &result {
                    let _ = update(&state, id, |w| {
                        w.steps[i].status = "failed".into();
                        w.steps[i].error = Some(e.clone());
                    });
                }
                result
            });
        }
        // Drain the batch on failure; no detached workers or successor dispatch.
        let mut errors = Vec::new();
        while let Some(result) = jobs.join_next().await {
            match result {
                Ok(Ok(())) => {}
                Ok(Err(e)) => errors.push(e),
                Err(e) => errors.push(e.to_string()),
            }
        }
        if !errors.is_empty() {
            return Err(errors.join("\n"));
        }
    }
    let w = read(&state, id)?;
    if w.steps.iter().any(|s| {
        s.status != "completed"
            || s.patch.is_none()
            || s.output.as_ref().is_none_or(|v| v.trim().is_empty())
    }) {
        return Err("incomplete worker artifacts".into());
    }
    let manager = GitWorktrees::new(&w.source_root).map_err(err)?;
    let artifact = manager
        .create(TaskId::default(), &w.source_head)
        .await
        .map_err(err)?;
    update(&state, id, |w| {
        w.artifact = Some(artifact.clone());
        w.thread.status = "integrating".into();
    })?;
    for s in &w.steps {
        check(&state, id)?;
        manager
            .apply_dependency_patch(&artifact, s.patch.as_deref().ok_or("missing patch")?)
            .await
            .map_err(err)?;
    }
    let diff = manager.diff(&artifact).await.map_err(err)?;
    update(&state, id, |w| w.final_diff = Some(diff.clone()))?;
    let version = artifact_version(&w.source_head, &diff);
    update(&state, id, |w| {
        w.artifact_version = Some(version.clone());
        w.thread.status = "awaiting_review".into();
        w.messages.push(Message { role:"assistant".into(), text:format!("実装を統合しました。ChatGPTでMCPを使ってレビューしてください。\nTask ID: {}\nArtifact version: {}\n成果物: {}\nレビュー結果はReview Manifestとして取り込めます。", id, version, artifact.path.display()) });
    })?;
    Ok(())
}
fn local_scope(s: &PlanStep) -> Result<()> {
    if s.level != 1
        || s.risk.as_deref() != Some("low")
        || s.estimated_loc.is_none_or(|n| n >= 100)
        || !(1..=2).contains(&s.owned_paths.len())
    {
        return Err(format!("{}: saved local route cannot execute this scope (level 1, low risk, 1-2 existing files, <100 LOC required); no cloud fallback",s.key));
    }
    Ok(())
}
fn observed_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
fn evidence_complete(steps: &[StepState]) -> Result<()> {
    if steps.is_empty()
        || steps.iter().any(|s| {
            s.status != "completed"
                || s.output.as_ref().is_none_or(|v| v.trim().is_empty())
                || s.patch.is_none()
                || s.verification.is_empty()
                || !s.verification.iter().any(|v| v["success"] == true)
        })
    {
        return Err("missing or failed worker evidence; final success forbidden".into());
    }
    Ok(())
}
async fn worker(state: &AppState, id: HubThreadId, i: usize) -> Result<()> {
    check(state, id)?;
    let w = read(state, id)?;
    let s = &w.steps[i];
    let payload = capsule(&w, i)?;
    update(state, id, |w| w.steps[i].capsule = Some(payload.clone()))?;
    let manager = GitWorktrees::new(&w.source_root).map_err(err)?;
    let worktree = manager
        .create(s.task_id, &w.source_head)
        .await
        .map_err(err)?;
    update(state, id, |w| w.steps[i].worktree = Some(worktree.clone()))?;
    for j in ancestors(&w.steps, i) {
        let dependency = &w.steps[j];
        if dependency.status != "completed" {
            return Err("ancestor did not complete".into());
        }
        manager
            .apply_dependency_patch(
                &worktree,
                dependency
                    .patch
                    .as_deref()
                    .ok_or("missing ancestor patch")?,
            )
            .await
            .map_err(err)?;
    }
    let input_tree = index_tree(&worktree).await?;
    update(state, id, |w| {
        w.steps[i].input_tree = Some(input_tree.clone())
    })?;
    check(state, id)?;
    let output = match s.target.provider {
        ModelProvider::Local => {
            local_scope(&s.step)?;
            if state.local.provider.model_id() != s.target.model {
                return Err("local model mismatch".into());
            }
            let files = collect_files(&worktree.path, &s.step.owned_paths).map_err(err)?;
            let local_capsule = bounded(
                json!({"request":payload,"files":files}).to_string(),
                CAPSULE_LIMIT,
            )?;
            let child = HubThreadId::default();
            update(state, id, |w| {
                w.steps[i].capsule = Some(local_capsule);
                w.steps[i].child_id = Some(child);
                w.children.push(Child {
                    id: child,
                    role: s.step.key.clone(),
                    provider: "local".into(),
                    provider_thread: None,
                    turn: None,
                });
            })?;
            {
                let store = state.store.lock().map_err(err)?;
                store
                    .save_thread(&ThreadMapping {
                        id: child,
                        project_id: w.thread.project_id,
                        provider: "spark".into(),
                        provider_thread_id: format!("autonomous-local:{child}"),
                        title: s.step.title.clone(),
                        status: "running".into(),
                    })
                    .map_err(err)?;
                store
                    .set_setting(&format!("autonomous_parent:{child}"), &id.to_string())
                    .map_err(err)?;
                store
                    .set_setting(&format!("thread_model:{child}"), &s.target.model)
                    .map_err(err)?;
            }
            check(state, id)?;
            update(state, id, |w| {
                w.steps[i].dispatch_phase = Some("local_implement_attempted".into())
            })?;
            let generated = state
                .local
                .provider
                .implement(&payload, &files)
                .await
                .map_err(err)?;
            update(state, id, |w| {
                w.steps[i].usage.push(
                    json!({"provider":"local","model":s.target.model,"usage":generated.usage}),
                )
            })?;
            check(state, id)?;
            let edits =
                PreparedEdits::prepare(&worktree.path, &files, &generated.output).map_err(err)?;
            edits.apply().map_err(err)?;
            let version =
                artifact_version(&w.source_head, &manager.diff(&worktree).await.map_err(err)?);
            update(state, id, |w| {
                w.steps[i].verification.push(json!({"success":true,"kind":"bounded_edit_readback","status":"passed","command":null,"exit_code":null,"finished_at_unix_ms":observed_ms(),"target_revision":version,"log_ref":null,"acceptance_status":"not_run","output":"preimages, owned paths, <100 changed lines and exact postimages verified; acceptance tests not executed"}))
            })?;
            generated.output.summary
        }
        ModelProvider::Codex => codex_worker(state, id, i, &worktree, payload).await?,
    };
    check(state, id)?;
    if output.trim().is_empty() {
        return Err("worker returned empty output".into());
    }
    let output = bounded(output, REVIEW_LIMIT)?;
    index_tree(&worktree).await?;
    let names = git(
        &worktree.path,
        &[
            "diff",
            "--cached",
            "--name-only",
            "-z",
            "--no-renames",
            &input_tree,
            "--",
        ],
    )
    .await?;
    for path in names.split('\0').filter(|p| !p.is_empty()) {
        if !valid_path(path)
            || !s
                .step
                .owned_paths
                .iter()
                .any(|p| Path::new(path).starts_with(p))
        {
            return Err(format!("worker modified unowned path: {path}"));
        }
    }
    if git(&worktree.path, &["rev-parse", "HEAD"]).await?.trim() != w.source_head {
        return Err("worker changed HEAD despite no-commit instruction".into());
    }
    let patch = bounded(
        git(
            &worktree.path,
            &[
                "diff",
                "--cached",
                "--binary",
                "--no-ext-diff",
                "--no-textconv",
                "--no-renames",
                &input_tree,
                "--",
            ],
        )
        .await?,
        4_000_000,
    )?;
    if let Some(child_id) = read(state, id)?.steps[i].child_id {
        let store = state.store.lock().map_err(err)?;
        if let Some(mut mapping) = store
            .threads()
            .map_err(err)?
            .into_iter()
            .find(|t| t.id == child_id)
        {
            mapping.status = "completed".into();
            store.save_thread(&mapping).map_err(err)?;
        }
    }
    update(state, id, |w| {
        let s = &mut w.steps[i];
        if s.usage.is_empty() {
            s.usage.push(json!({"provider":s.target.provider,"model":s.target.model,"tokens":null,"status":"usage_not_reported"}));
        }
        s.output = Some(output);
        s.patch = Some(patch);
        s.status = if w.stop_requested {
            "interrupted"
        } else {
            "completed"
        }
        .into();
    })?;
    Ok(())
}
async fn codex_worker(
    state: &AppState,
    id: HubThreadId,
    i: usize,
    worktree: &Worktree,
    payload: String,
) -> Result<String> {
    let s = read(state, id)?.steps[i].clone();
    let mut explicit_payload: Value = serde_json::from_str(&payload).map_err(err)?;
    explicit_payload["working_directory"] = json!(worktree.path);
    let payload = bounded(explicit_payload.to_string(), CAPSULE_LIMIT)?;
    update(state, id, |w| w.steps[i].capsule = Some(payload.clone()))?;
    let provider = crate::models::codex_provider(state).await?;
    let mut events = provider.events();
    // Persist the attempt before thread/start. A lost response is never retried.
    update(state, id, |w| {
        w.steps[i].dispatch_phase = Some("thread_start_attempted".into())
    })?;
    check(state, id)?;
    let (thread, child) = if let Some(child) = s.child_id {
        let mapping = read(state, id)?
            .children
            .into_iter()
            .find(|c| c.id == child && c.provider == "codex")
            .ok_or("saved worker session is missing")?;
        let thread = mapping
            .provider_thread
            .ok_or("saved provider thread is missing")?;
        let snapshot = provider
            .resume_thread_with_model(&thread, worktree.path.clone(), &s.target.model)
            .await
            .map_err(err)?;
        if snapshot
            .active_turn
            .is_some_and(|t| matches!(t.status.as_str(), "inProgress" | "running"))
        {
            return Err("saved worker still has an active turn; no new turn sent".into());
        }
        update(state, id, |w| {
            if let Some(c) = w.children.iter_mut().find(|c| c.id == child) {
                c.turn = None;
            }
        })?;
        (thread, child)
    } else {
        let thread = provider
            .start_thread_with_model(worktree.path.clone(), &s.target.model)
            .await
            .map_err(err)?;
        let child = HubThreadId::default();
        update(state, id, |w| {
            w.steps[i].child_id = Some(child);
            w.children.push(Child {
                id: child,
                role: s.step.key.clone(),
                provider: "codex".into(),
                provider_thread: Some(thread.clone()),
                turn: None,
            });
        })?;
        (thread, child)
    };
    {
        let store = state.store.lock().map_err(err)?;
        let root = read_project_id_without_lock(id, &store)?;
        store
            .save_thread(&ThreadMapping {
                id: child,
                project_id: root,
                provider: "codex".into(),
                provider_thread_id: thread.id.clone(),
                title: s.step.title.clone(),
                status: "running".into(),
            })
            .map_err(err)?;
        for (k, v) in [
            (format!("autonomous_parent:{child}"), id.to_string()),
            (format!("thread_model:{child}"), s.target.model.clone()),
            (
                format!("thread_reasoning:{child}"),
                s.target.reasoning.clone().unwrap_or_default(),
            ),
            (
                format!("thread_root:{child}"),
                worktree.path.to_string_lossy().into_owned(),
            ),
        ] {
            store.set_setting(&k, &v).map_err(err)?;
        }
    }
    check(state, id)?;
    update(state, id, |w| {
        w.steps[i].dispatch_phase = Some("turn_start_attempted".into())
    })?;
    let turn_started=std::time::Instant::now();
    let turn = provider
        .start_turn_in_worktree(
            &thread,
            worktree.path.clone(),
            payload,
            &s.target.model,
            s.target.reasoning.as_deref(),
        )
        .await
        .map_err(err)?;
    update(state, id, |w| {
        if let Some(c) = w.children.iter_mut().find(|c| c.id == child) {
            c.turn = Some(turn.clone());
        }
    })?;
    let mut command_starts = std::collections::HashMap::new();
    loop {
        let event = events
            .recv()
            .await
            .map_err(|e| format!("worker evidence stream lost: {e}"))?;
        if event.kind == "disconnected" {
            return Err("CLI disconnected; reconcile without replay".into());
        }
        if event.thread_id.as_deref() != Some(&thread.id) {
            continue;
        }
        if event.turn_id.as_deref().is_some_and(|t| t != turn.id) {
            continue;
        }
        if let Some(details) = &event.details {
            match details {
                EventDetails::Usage { .. } => {
                    update(state, id, |w| w.steps[i].usage.push(json!(details)))?;
                }
                EventDetails::Command { .. } if event.kind == "item_started" => {
                    command_starts.insert(event.item_id.clone(), observed_ms());
                }
                EventDetails::Command { command, exit_code } if event.kind == "item_completed" => {
                    let diff = GitWorktrees::new(&read(state, id)?.source_root)
                        .map_err(err)?
                        .diff(worktree)
                        .await
                        .map_err(err)?;
                    let revision = artifact_version(&worktree.base_sha, &diff);
                    let log_ref = format!(
                        "verification_log:{id}:{}:{}:{}",
                        read(state, id)?.iteration,
                        read(state, id)?.steps[i].step.key,
                        read(state, id)?.steps[i].verification.len()
                    );
                    let full_log = bounded(event.text.clone(), REVIEW_LIMIT)?;
                    state
                        .store
                        .lock()
                        .map_err(err)?
                        .set_setting(&log_ref, &full_log)
                        .map_err(err)?;
                    let evidence = json!({"kind":"command","status":match exit_code {Some(0)=>"passed",Some(_)=>"failed",None=>"unknown"},"success":*exit_code==Some(0),"command":command,"exit_code":exit_code,"started_at_unix_ms":command_starts.remove(&event.item_id),"finished_at_unix_ms":observed_ms(),"clock":"Localoud event receipt time","target_revision":revision,"revision_observation":"filesystem snapshot after command-completed event; not an attestation of the exact revision during command execution","execution_revision_verified":false,"base_revision":worktree.base_sha,"worktree":worktree.path,"log_ref":log_ref,"output":event.text.chars().take(8000).collect::<String>(),"output_truncated":event.text.chars().count()>8000,"acceptance_status":"unknown"});
                    update(state, id, |w| w.steps[i].verification.push(evidence))?;
                }
                _ => {}
            }
        }
        if event.kind == "error" {
            return Err(format!("worker error: {}", event.text));
        }
        if event.kind == "turn_completed" && event.turn_id.as_deref() == Some(&turn.id) {
            if event.text != "completed" {
                return Err(format!("worker ended: {}", event.text));
            }
            break;
        }
    }
    update(state,id,|w| {
        let usage=&mut w.steps[i].usage;
        if usage.is_empty() { usage.push(json!({"tokens":null})); }
        if let Some(last)=usage.last_mut().and_then(Value::as_object_mut) {last.insert("latency_ms".into(),json!(turn_started.elapsed().as_millis() as u64));}
    })?;
    // Keep failed/unknown attempts as evidence for review, without misclassifying
    // an exploratory command failure as a failed implementation.
    if read(state, id)?.steps[i].verification.is_empty() {
        update(state, id, |w| {
            w.steps[i].verification.push(json!({"kind":"acceptance","status":"not_run","success":false,"command":null,"exit_code":null,"finished_at_unix_ms":observed_ms(),"target_revision":null,"log_ref":null,"output":"No command evidence was observed"}))
        })?;
    }
    let snapshot = provider.read_thread(&thread).await.map_err(err)?;
    if snapshot
        .active_turn
        .as_ref()
        .is_some_and(|t| t.status != "completed")
    {
        return Err("worker still active after completion event".into());
    }
    let text = snapshot
        .messages
        .iter()
        .rev()
        .find(|m| m.role == "assistant" && !m.text.trim().is_empty())
        .ok_or("missing worker answer")?
        .text
        .clone();
    {
        let store = state.store.lock().map_err(err)?;
        if let Some(mut mapping) = store
            .threads()
            .map_err(err)?
            .into_iter()
            .find(|m| m.id == child)
        {
            mapping.status = "completed".into();
            store.save_thread(&mapping).map_err(err)?;
        }
    }
    Ok(text)
}
fn read_project_id_without_lock(id: HubThreadId, store: &hub_db::Store) -> Result<ProjectId> {
    let w: AutonomousSnapshot = serde_json::from_str(
        &store
            .setting(&key(id))
            .map_err(err)?
            .ok_or("missing workflow")?,
    )
    .map_err(err)?;
    Ok(w.thread.project_id)
}

#[cfg(test)]
#[path = "autonomous_tests.rs"]
mod tests;
