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
    collections::{HashMap, HashSet},
    path::{Component, Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::Duration,
};
use tauri::Manager;

type Result<T> = std::result::Result<T, String>;
const PLAN_LIMIT: usize = 256 * 1024;
const CAPSULE_LIMIT: usize = 256 * 1024;
const WORKER_TOKEN_TARGET: usize = 16_000;
const REVIEW_TOKEN_TARGET: usize = 32_000;
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
    /// Optional user/planner selection. It is resolved and frozen when the
    /// manifest is imported; later Auto setting changes never affect the run.
    #[serde(default)]
    pub target_override: Option<ModelTarget>,
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
    #[serde(default)]
    pub agent_name: String,
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

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum AutomatedVerdict {
    Pass,
    Rework,
    Inconclusive,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AutomatedReviewOutput {
    verdict: AutomatedVerdict,
    summary: String,
    findings: Vec<String>,
    changes: Vec<crate::manifest::ReviewChange>,
}

impl AutomatedReviewOutput {
    fn validate(&self, steps: &[StepState]) -> Result<()> {
        if self.summary.trim().is_empty() || self.summary.len() > REVIEW_LIMIT {
            return Err("review summary is empty or too large".into());
        }
        if self.findings.len() > 100 || self.changes.len() > 8 {
            return Err("review output has too many findings or changes".into());
        }
        let unique_changes = self
            .changes
            .iter()
            .map(|change| &change.step_key)
            .collect::<HashSet<_>>();
        if unique_changes.len() != self.changes.len() {
            return Err("review output contains duplicate step changes".into());
        }
        if self.changes.iter().any(|change| {
            change.instruction.trim().is_empty()
                || !steps.iter().any(|step| step.step.key == change.step_key)
        }) {
            return Err("review output contains an invalid step change".into());
        }
        if matches!(self.verdict, AutomatedVerdict::Pass) && !self.changes.is_empty() {
            return Err("passing review cannot request changes".into());
        }
        if matches!(self.verdict, AutomatedVerdict::Rework) && self.changes.is_empty() {
            return Err("rework review must identify at least one step".into());
        }
        if matches!(self.verdict, AutomatedVerdict::Inconclusive) && !self.changes.is_empty() {
            return Err("inconclusive review cannot request executable changes".into());
        }
        Ok(())
    }
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
            | "needs_attention"
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
    if text.len() > PLAN_LIMIT {
        return Err("plan exceeds 256 KiB".into());
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
            || s.target_override
                .as_ref()
                .is_some_and(|target| target.validate().is_err())
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
fn compact_evidence(record: &Value) -> Value {
    let mut result = record.clone();
    if let Some(output) = record["output"].as_str() {
        let excerpt: String = output.chars().take(2000).collect();
        result["output"] = json!(excerpt);
        result["output_excerpted"] = json!(output.chars().count() > 2000);
    }
    result
}
// A reproducible planning estimate, not a tokenizer or provider measurement.
fn context_metrics(before: &str, after: &str, target: usize, limit: usize) -> Value {
    let estimate = after.chars().count().div_ceil(3);
    json!({
        "schema_version":1, "strategy":"artifact_references_and_evidence_excerpts_v1",
        "before_bytes":before.len(), "after_bytes":after.len(),
        "bytes_saved":before.len() as i64 - after.len() as i64,
        "before_estimated_tokens":before.chars().count().div_ceil(3),
        "after_estimated_tokens":estimate, "estimate_method":"unicode_scalar_count_div_3_ceil",
        "token_target":target, "over_target":estimate > target, "hard_byte_limit":limit,
        "provider_context_limit":null,
        "measurement_scope":"serialized_application_payload_only",
        "note":"Soft target only. Excludes system prompts, tools, history and subsequent reads. Provider usage is recorded separately; size reduction is not measured token or latency savings."
    })
}
fn save_context_metrics(
    state: &AppState,
    w: &AutonomousSnapshot,
    key: &str,
    before: &str,
    after: &str,
    target: usize,
    limit: usize,
) -> Result<()> {
    let mut metrics = context_metrics(before, after, target, limit);
    metrics["iteration"] = json!(w.iteration);
    metrics["step_key"] = json!(key);
    metrics["observed_at_unix_ms"] = json!(observed_ms());
    state
        .store
        .lock()
        .map_err(err)?
        .set_setting(
            &format!("context_metrics:{}:{}:{key}", w.thread.id, w.iteration),
            &metrics.to_string(),
        )
        .map_err(err)
}
fn unabridged_capsule(payload: &str, w: &AutonomousSnapshot) -> Result<String> {
    let mut envelope: Value = serde_json::from_str(payload).map_err(err)?;
    if let Some(dependencies) = envelope["parts"][0]["data"]["dependencies"].as_array_mut() {
        for dependency in dependencies {
            if let Some(step) = w.steps.iter().find(|s| dependency["key"] == s.step.key) {
                dependency["verification"] = json!(step.verification);
                dependency
                    .as_object_mut()
                    .unwrap()
                    .remove("attention_evidence");
            }
        }
    }
    serde_json::to_string(&envelope).map_err(err)
}
fn dependency_verification_summary(records: &[Value]) -> Value {
    let count = |status: &str| records.iter().filter(|v| v["status"] == status).count();
    let passed = count("passed");
    let failed = count("failed");
    let not_run = count("not_run");
    json!({
        "records": records.len(), "passed": passed, "failed": failed,
        "not_run": not_run, "unknown": records.len() - passed - failed - not_run,
        "acceptance_status": "unknown",
        "details": "Full command logs remain in the Localoud task history, not in this capsule. Counts are recorded command/evidence statuses, not acceptance proof. Inspect the dependency artifacts in this worktree for findings and verification details; report missing evidence rather than inferring success."
    })
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
            Ok(json!({"key":d,"outcome":s.output,
                "artifacts":s.step.owned_paths,
                "verification":dependency_verification_summary(&s.verification),
                "attention_evidence":s.verification.iter().filter(|v| v["status"] != "passed").map(compact_evidence).collect::<Vec<_>>()}))
        })
        .collect::<std::result::Result<_, &str>>()
        .map_err(err)?;
    let payload = json!({"task":w.steps[i].step,"target":w.steps[i].target,
        "assignment":{"agent_name":w.steps[i].agent_name},"dependencies":dependencies,
        "overall_acceptance":w.manifest.as_ref().map(|m| &m.acceptance),"revision_instruction":w.steps[i].revision_instruction,
        "iteration":w.iteration,"worktree_note":"Each iteration has a new worktree containing only base sources and current dependency outputs. Reimplement your step here; your prior own patch is not pre-applied. Never write to a previous worktree.",
        "instructions":"Execute only this task in the supplied isolated worktree. Read repository sources as needed. No subagents, no commit, merge, push, external publication or changes outside owned_paths. Run acceptance checks and report their exact results. Do not claim unrun tests passed. No parent conversation is available."});
    let reference_task_ids = w.steps[i]
        .step
        .dependencies
        .iter()
        .filter_map(|key| {
            w.steps
                .iter()
                .find(|step| &step.step.key == key)
                .map(|step| step.task_id.to_string())
        })
        .collect();
    let message = protocol_types::a2a::data_message(
        format!(
            "{}:{}:{}",
            w.steps[i].task_id, w.iteration, w.steps[i].step.key
        ),
        Some(w.thread.id.to_string()),
        Some(w.steps[i].task_id.to_string()),
        payload,
        protocol_types::a2a::CONTEXT_CAPSULE_MEDIA_TYPE,
        reference_task_ids,
    )
    .map_err(err)?;
    bounded(serde_json::to_string(&message).map_err(err)?, CAPSULE_LIMIT)
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
// Command output is evidence, not a dispatch payload. Sanitize before storing
// it so incidental examples or sensitive output cannot abort a running worker.
fn command_log(text: &str) -> Result<(String, bool)> {
    let sanitized = hub_policy::redact(text);
    let redacted = sanitized != text;
    Ok((bounded(sanitized, REVIEW_LIMIT)?, redacted))
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

pub(crate) async fn project_head(root: &Path) -> Result<String> {
    Ok(git(root, &["rev-parse", "--verify", "HEAD^{commit}"])
        .await?
        .trim()
        .to_owned())
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
    bounded(manifest.request.clone(), PLAN_LIMIT)?;
    let request = AutonomousRequest {
        project_id: manifest.project_id.clone(),
        text: manifest.request.clone(),
    };
    let plan = parse_plan(&json!({"steps":manifest.steps}).to_string())?;
    let project_id = ProjectId(request.project_id.parse().map_err(err)?);
    let settings = crate::auto_routing::read_settings(&state).await?;
    let root = {
        let s = state.store.lock().map_err(err)?;
        s.projects()
            .map_err(err)?
            .into_iter()
            .find(|p| p.id == project_id)
            .ok_or("project not found")?
            .root
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
        let agent = settings.agent_for_level(step.level).map_err(err)?;
        let target = step
            .target_override
            .clone()
            .map(Ok)
            .unwrap_or_else(|| settings.target(step.level))
            .map_err(err)?;
        steps.push(StepState {
            task_id: TaskId::default(),
            step,
            target,
            agent_name: agent.name,
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
    let task_ids = w
        .steps
        .iter()
        .map(|step| (step.step.key.clone(), step.task_id))
        .collect::<HashMap<_, _>>();
    let durable_tasks = w
        .steps
        .iter()
        .map(|step| hub_core::Task {
            id: step.task_id,
            project_id,
            plan_id: None,
            parent_task_id: None,
            title: step.step.title.clone(),
            description: step.step.goal.clone(),
            dependencies: step
                .step
                .dependencies
                .iter()
                .filter_map(|key| task_ids.get(key).copied())
                .collect(),
            preferred_executor: hub_core::ExecutorPreference::Auto,
            assigned_executor: Some(match step.target.provider {
                ModelProvider::Local => hub_core::ExecutorKind::Spark,
                ModelProvider::Codex => hub_core::ExecutorKind::Codex,
                ModelProvider::Api => hub_core::ExecutorKind::Api,
            }),
            risk: match step.step.risk.as_deref() {
                Some("low") => hub_core::RiskLevel::Low,
                Some("high") => hub_core::RiskLevel::High,
                _ => hub_core::RiskLevel::Medium,
            },
            complexity: match step.step.level {
                0 | 1 => hub_core::Complexity::Trivial,
                2 | 3 => hub_core::Complexity::Normal,
                _ => hub_core::Complexity::Deep,
            },
            status: hub_core::TaskStatus::Ready,
            context_capsule_id: None,
            worktree_id: None,
            attempts: 0,
            max_attempts: 2,
        })
        .collect::<Vec<_>>();
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
        for task in &durable_tasks {
            store.save_task(task).map_err(err)?;
        }
        for task in &durable_tasks {
            store.save_dependencies(task).map_err(err)?;
        }
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
        "awaiting_review" | "review_rejected" | "needs_attention" | "completed"
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
        vec![(
            "integrated".to_owned(),
            "統合した成果物".to_owned(),
            artifact,
        )]
    } else {
        w.steps
            .iter()
            .filter_map(|s| {
                s.worktree
                    .as_ref()
                    .map(|tree| (s.step.key.clone(), s.step.title.clone(), tree))
            })
            .collect()
    };
    let mut changes = vec![];
    for (key, title, tree) in targets {
        // GitWorktrees validates ownership and includes bounded untracked files.
        // A failed read remains visible; it must not look like an empty diff.
        let (diff, error) = match manager.diff(tree).await {
            Ok(diff) => (Some(diff), None),
            Err(e) => (None, Some(format!("{e:#}"))),
        };
        changes.push(WorktreeChange {
            key,
            title,
            path: tree.path.clone(),
            diff,
            error,
        });
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
            let Some(child_id) = step.child_id else {
                continue;
            };
            let turn = w
                .children
                .iter()
                .find(|c| c.id == child_id)
                .and_then(|c| c.turn.as_ref());
            let mut events = vec![];
            for (sequence, body) in store
                .recent_activity(child_id, turn.map(|t| t.id.as_str()))
                .map_err(err)?
            {
                let mut event: protocol_types::AgentEvent =
                    serde_json::from_str(&body).map_err(err)?;
                if event.text.chars().count() > 6000 {
                    event.text =
                        event.text.chars().take(6000).collect::<String>() + "\n…（表示を省略）";
                }
                // The readable command and event text suffice for this panel;
                // do not duplicate large tool argument payloads in every poll.
                if !matches!(event.details, Some(EventDetails::Command { .. })) {
                    event.details = None;
                }
                events.push(hub_events::JournalEvent { sequence, event });
            }
            workers.push(WorkerActivity { child_id, events });
        }
    }
    let changes = if include_changes {
        worktree_changes(&w).await?
    } else {
        vec![]
    };
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
    if w.manifest.is_none() {
        return Err("再開できる中断済みManifestタスクではありません".into());
    }
    if matches!(
        w.thread.status.as_str(),
        "awaiting_review" | "needs_attention"
    ) && w.steps.iter().all(|step| step.status == "completed")
    {
        w.history.push(IterationRecord {
            iteration: w.iteration,
            artifact: w.artifact.clone(),
            artifact_version: w.artifact_version.clone(),
            final_diff: w.final_diff.clone(),
            steps: w.steps.clone(),
            review: w.review.clone(),
        });
        w.artifact = None;
        w.artifact_version = None;
        w.final_diff = None;
        w.review = None;
        w.error = None;
        w.stop_requested = false;
        w.thread.status = "queued".into();
        return Ok(());
    }
    if !matches!(
        w.thread.status.as_str(),
        "interrupted" | "failed" | "reconciliation_required"
    ) {
        return Err("再開できる中断済みManifestタスクではありません".into());
    }
    let explicit_api_retry = matches!(w.thread.status.as_str(), "interrupted" | "failed");
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
        if s.target.provider == ModelProvider::Local
            || (s.target.provider == ModelProvider::Api && explicit_api_retry)
        {
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

fn parse_automated_review(text: &str, steps: &[StepState]) -> Result<AutomatedReviewOutput> {
    let text = text.trim();
    let text = text
        .strip_prefix("```json\n")
        .or_else(|| text.strip_prefix("```\n"))
        .and_then(|body| body.strip_suffix("```"))
        .unwrap_or(text)
        .trim();
    let output: AutomatedReviewOutput = serde_json::from_str(text).map_err(err)?;
    output.validate(steps)?;
    Ok(output)
}

fn review_payload_raw(w: &AutonomousSnapshot, diff: &str, version: &str) -> Result<String> {
    let manifest = w.manifest.as_ref().ok_or("review manifest is missing")?;
    let payload = json!({
        "goal": manifest.request,
        "acceptance_criteria": manifest.acceptance,
        "artifact_hash": version,
        "base_revision": w.source_head,
        "diff": diff,
        "steps": w.steps.iter().map(|step| json!({
            "key": step.step.key,
            "title": step.step.title,
            "description": step.step.goal,
            "owned_paths": step.step.owned_paths,
            "output": step.output,
            "verification": step.verification,
        })).collect::<Vec<_>>(),
        "response_contract": {
            "verdict": "pass | rework | inconclusive",
            "summary": "non-empty string",
            "findings": ["string"],
            "changes": [{"step_key":"existing step key","instruction":"required correction"}]
        }
    });
    let message = protocol_types::a2a::data_message(
        format!("review:{}:{}:{version}", w.thread.id, w.iteration),
        Some(w.thread.id.to_string()),
        Some(w.thread.id.to_string()),
        payload,
        protocol_types::a2a::REVIEW_CAPSULE_MEDIA_TYPE,
        w.steps
            .iter()
            .map(|step| step.task_id.to_string())
            .collect(),
    )
    .map_err(err)?;
    serde_json::to_string(&message).map_err(err)
}
fn review_payload(w: &AutonomousSnapshot, diff: &str, version: &str) -> Result<String> {
    let mut envelope: Value =
        serde_json::from_str(&review_payload_raw(w, diff, version)?).map_err(err)?;
    let data = &mut envelope["parts"][0]["data"];
    data["evidence_policy"] = json!("Command output excerpts are explicitly marked. Original logs remain in Localoud. If supplied evidence cannot establish acceptance, return inconclusive; never infer success from counts or missing output.");
    if let Some(steps) = data["steps"].as_array_mut() {
        for step in steps {
            if let Some(records) = step["verification"].as_array() {
                step["verification"] =
                    json!(records.iter().map(compact_evidence).collect::<Vec<_>>());
            }
        }
    }
    bounded(serde_json::to_string(&envelope).map_err(err)?, REVIEW_LIMIT)
}

async fn codex_review_turn(
    state: &AppState,
    thread: &ProviderThread,
    target: &ModelTarget,
    payload: String,
) -> Result<String> {
    let provider = crate::models::codex_provider(state).await?;
    let mut events = provider.events();
    let prompt = format!(
        "You are a read-only final reviewer in a new session with no executor history. Review only the supplied goal, acceptance criteria, exact diff, and verification evidence. Do not edit files, run commands, delegate, or infer missing evidence. Return exactly one JSON object matching response_contract; pass only when the evidence and diff establish every acceptance condition.\n{payload}"
    );
    let turn = provider
        .start_turn_with_reasoning(thread, prompt, &target.model, target.reasoning.as_deref())
        .await
        .map_err(err)?;
    let deadline = tokio::time::Instant::now() + DEADLINE;
    loop {
        let event = tokio::time::timeout_at(deadline, events.recv())
            .await
            .map_err(|_| "reviewer timed out".to_string())?
            .map_err(|e| format!("reviewer event stream lost: {e}"))?;
        if event.thread_id.as_deref() != Some(&thread.id)
            || event.turn_id.as_deref().is_some_and(|id| id != turn.id)
        {
            continue;
        }
        if event.kind == "error" {
            return Err(format!("reviewer error: {}", event.text));
        }
        if event.kind == "turn_completed" {
            if event.text != "completed" {
                return Err(format!("reviewer ended: {}", event.text));
            }
            break;
        }
    }
    let snapshot = provider.read_thread(thread).await.map_err(err)?;
    snapshot
        .messages
        .iter()
        .rev()
        .find(|message| message.role == "assistant" && !message.text.trim().is_empty())
        .map(|message| message.text.clone())
        .ok_or_else(|| "reviewer returned no answer".into())
}

async fn automated_review(
    state: &AppState,
    id: HubThreadId,
    artifact: &Worktree,
    diff: &str,
    version: &str,
) -> Result<bool> {
    let w = read(state, id)?;
    let reviewer_agent = w
        .settings_snapshot
        .agent_for_route("reviewer")
        .map_err(err)?;
    let Some(target) = w.settings_snapshot.reviewer_default.clone() else {
        update(state, id, |workflow| {
            workflow.thread.status = "needs_attention".into();
            workflow.error =
                Some("reviewer_default is not configured; completion is blocked".into());
        })?;
        return Ok(false);
    };
    if let Err(error) = crate::models::validate_target(&target, state).await {
        update(state, id, |workflow| {
            workflow.thread.status = "needs_attention".into();
            workflow.error = Some(format!("reviewer is unavailable: {error}"));
        })?;
        return Ok(false);
    }
    let payload = match review_payload(&w, diff, version) {
        Ok(payload) => payload,
        Err(error) => {
            update(state, id, |workflow| {
                workflow.thread.status = "needs_attention".into();
                workflow.error = Some(format!("review input is not safely bounded: {error}"));
            })?;
            return Ok(false);
        }
    };
    save_context_metrics(
        state,
        &w,
        "final_review",
        &review_payload_raw(&w, diff, version)?,
        &payload,
        REVIEW_TOKEN_TARGET,
        REVIEW_LIMIT,
    )?;
    let reviewer_id = HubThreadId::default();
    let logical_turn_id = TaskId::default().to_string();
    let mut codex_thread = None;
    let review_copy = if target.provider == ModelProvider::Codex {
        let manager = GitWorktrees::new(&w.source_root).map_err(err)?;
        match manager.create(TaskId::default(), &w.source_head).await {
            Ok(copy) => {
                if let Err(error) = manager.apply_dependency_patch(&copy, diff).await {
                    update(state, id, |workflow| {
                        workflow.thread.status = "needs_attention".into();
                        workflow.error = Some(format!(
                            "reviewer isolation worktree could not be prepared: {error}"
                        ));
                    })?;
                    return Ok(false);
                }
                Some(copy)
            }
            Err(error) => {
                update(state, id, |workflow| {
                    workflow.thread.status = "needs_attention".into();
                    workflow.error = Some(format!(
                        "reviewer isolation worktree could not be created: {error}"
                    ));
                })?;
                return Ok(false);
            }
        }
    } else {
        None
    };
    let api_profile_revision = if target.provider == ModelProvider::Api {
        let revision = {
            let registry = state.providers.read().await;
            target
                .profile_id
                .as_deref()
                .and_then(|profile_id| registry.profile(profile_id))
                .map(|profile| profile.revision)
        };
        let Some(revision) = revision else {
            update(state, id, |workflow| {
                workflow.thread.status = "needs_attention".into();
                workflow.error = Some("reviewer provider profile is unavailable".into());
            })?;
            return Ok(false);
        };
        Some(revision)
    } else {
        None
    };
    let provider_thread_id = match target.provider {
        ModelProvider::Codex => {
            let thread_result: Result<ProviderThread> = async {
                let provider = crate::models::codex_provider(state).await?;
                provider
                    .start_thread_with_model(
                        review_copy
                            .as_ref()
                            .ok_or("reviewer isolation worktree is missing")?
                            .path
                            .clone(),
                        &target.model,
                    )
                    .await
                    .map_err(err)
            }
            .await;
            let thread = match thread_result {
                Ok(thread) => thread,
                Err(error) => {
                    update(state, id, |workflow| {
                        workflow.thread.status = "needs_attention".into();
                        workflow.error = Some(format!("reviewer session could not start: {error}"));
                    })?;
                    return Ok(false);
                }
            };
            let provider_thread_id = thread.id.clone();
            codex_thread = Some(thread);
            provider_thread_id
        }
        ModelProvider::Api => format!("api-review:{reviewer_id}"),
        ModelProvider::Local => format!("local-review:{reviewer_id}"),
    };
    let mapping = ThreadMapping {
        id: reviewer_id,
        project_id: w.thread.project_id,
        provider: match target.provider {
            ModelProvider::Codex => "codex".into(),
            ModelProvider::Api => format!(
                "api:{}",
                target
                    .profile_id
                    .as_deref()
                    .ok_or("reviewer profile is missing")?
            ),
            ModelProvider::Local => "spark".into(),
        },
        provider_thread_id: provider_thread_id.clone(),
        title: format!("Review: {}", w.thread.title),
        status: "running".into(),
    };
    let run_ids = w
        .steps
        .iter()
        .map(|step| (step.task_id, TaskId::default().to_string()))
        .collect::<Vec<_>>();
    {
        let store = state.store.lock().map_err(err)?;
        store.save_thread(&mapping).map_err(err)?;
        store
            .set_setting(&format!("autonomous_parent:{reviewer_id}"), &id.to_string())
            .map_err(err)?;
        if target.provider != ModelProvider::Local {
            let profile_id = target.profile_id.clone().unwrap_or_else(|| "codex".into());
            let profile_revision = api_profile_revision.unwrap_or(1);
            let api_target = protocol_types::providers::ModelTarget {
                profile_id,
                model_id: target.model.clone(),
                effort: target.reasoning.clone(),
            };
            store
                .set_session_model_policy(
                    reviewer_id,
                    &protocol_types::providers::SessionModelPolicy {
                        default_target: api_target.clone(),
                        reviewer_target: None,
                        allow_turn_override: false,
                    },
                )
                .map_err(err)?;
            let segment_id = TaskId::default().to_string();
            store
                .start_provider_segment(&hub_db::ProviderSegmentRecord {
                    id: segment_id.clone(),
                    thread_id: reviewer_id,
                    target: api_target.clone(),
                    profile_revision,
                    provider_thread_id: codex_thread.as_ref().map(|thread| thread.id.clone()),
                    ended: false,
                })
                .map_err(err)?;
            store
                .record_turn_target(&hub_db::TurnTargetRecord {
                    turn_id: logical_turn_id.clone(),
                    thread_id: reviewer_id,
                    segment_id,
                    target: api_target,
                    profile_revision,
                    resolved_from: protocol_types::providers::TargetResolution::ReviewerDefault,
                })
                .map_err(err)?;
        }
        for (task_id, run_id) in &run_ids {
            let executor_thread_id = w
                .steps
                .iter()
                .find(|step| step.task_id == *task_id)
                .and_then(|step| step.child_id)
                .ok_or("executor session is missing for review")?;
            store
                .create_review_run(&hub_db::ReviewRunRecord {
                    id: run_id.clone(),
                    task_id: *task_id,
                    executor_thread_id,
                    reviewer_thread_id: reviewer_id,
                    artifact_hash: version.into(),
                    verdict: "pending".into(),
                    body: json!({"status":"started","iteration":w.iteration}),
                })
                .map_err(err)?;
        }
    }
    update(state, id, |workflow| {
        workflow.children.push(Child {
            id: reviewer_id,
            role: reviewer_agent.name.clone(),
            provider: mapping.provider.clone(),
            provider_thread: Some(ProviderThread {
                id: provider_thread_id.clone(),
            }),
            turn: None,
        });
    })?;

    let attempted: Result<AutomatedReviewOutput> = async {
        match target.provider {
        ModelProvider::Local => {
            let generated = state
                .local
                .provider
                .review_diff(&w.request.text, diff)
                .await
                .map_err(err)?;
            state.local.record_usage(&generated.usage, None).map_err(err)?;
            use protocol_types::local::ReviewVerdict;
            let verdict = match generated.output.verdict {
                ReviewVerdict::Approve => AutomatedVerdict::Pass,
                ReviewVerdict::Rework => AutomatedVerdict::Rework,
                ReviewVerdict::HumanRequired => AutomatedVerdict::Inconclusive,
            };
            let instruction = generated.output.findings.join("\n");
            Ok(AutomatedReviewOutput {
                changes: matches!(verdict, AutomatedVerdict::Rework)
                    .then(|| {
                        w.steps
                            .iter()
                            .map(|step| crate::manifest::ReviewChange {
                                step_key: step.step.key.clone(),
                                instruction: if instruction.trim().is_empty() {
                                    generated.output.summary.clone()
                                } else {
                                    instruction.clone()
                                },
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                verdict,
                summary: generated.output.summary,
                findings: generated.output.findings,
            })
        }
        ModelProvider::Api => {
            let profile_id = target.profile_id.clone().ok_or("reviewer profile is missing")?;
            let transport = state
                .providers
                .read()
                .await
                .transport(&profile_id)
                .map_err(err)?;
            let result = hub_runtime::api_worker::ApiAgent {
                root: artifact.path.clone(),
                logical_thread_id: reviewer_id.to_string(),
                logical_turn_id: logical_turn_id.clone(),
                target: protocol_types::providers::ModelTarget {
                    profile_id: profile_id.clone(),
                    model_id: target.model.clone(),
                    effort: target.reasoning.clone(),
                },
                transport,
                external_tools: Vec::new(),
                external_handler: None,
                command_authorizer: None,
                allow_writes: false,
                allow_commands: false,
            }
            .run(
                "You are a read-only final reviewer in a fresh session. The user message is an A2A v1 envelope; use only its review data Part containing the exact diff and evidence. Never edit, run commands, or delegate. Return exactly one JSON object matching response_contract. Pass only when every acceptance condition is established.".into(),
                payload,
            )
            .await;
            let result = match result {
                Ok(result) => result,
                Err(error) => {
                    let store = state.store.lock().map_err(err)?;
                    store
                        .record_provider_turn_outcome(&hub_db::ProviderTurnOutcomeRecord {
                            turn_id: logical_turn_id.clone(),
                            thread_id: reviewer_id,
                            status: "failed".into(),
                            stop_reason: None,
                            failure_kind: Some(
                                provider_api::failure_kind(&error).as_str().into(),
                            ),
                            detail: Some(
                                hub_policy::redact(&format!("{error:#}"))
                                    .chars()
                                    .take(2_000)
                                    .collect(),
                            ),
                        })
                        .map_err(err)?;
                    return Err(err(error));
                }
            };
            {
                let store = state.store.lock().map_err(err)?;
                for message in &result.transcript {
                    store
                        .append_provider_message(&hub_db::ProviderMessageRecord {
                            id: TaskId::default().to_string(),
                            thread_id: reviewer_id,
                            segment_id: store
                                .active_provider_segment(reviewer_id)
                                .map_err(err)?
                                .map(|segment| segment.id),
                            provider_turn_id: Some(logical_turn_id.clone()),
                            role: message.role,
                            content: message.content.clone(),
                            provider_state: message.provider_state.clone(),
                        })
                        .map_err(err)?;
                }
                store
                    .record_usage(&hub_core::ModelUsageRecord {
                        id: TaskId::default().to_string(),
                        provider: profile_id.clone(),
                        model: target.model.clone(),
                        task_id: None,
                        turn_id: Some(logical_turn_id.clone()),
                        prompt_tokens: Some(result.usage.input_tokens),
                        cached_tokens: Some(result.usage.cached_input_tokens),
                        completion_tokens: Some(result.usage.output_tokens),
                        estimated_cost: None,
                        latency_ms: None,
                    })
                    .map_err(err)?;
                store
                    .record_provider_turn_outcome(&hub_db::ProviderTurnOutcomeRecord {
                        turn_id: logical_turn_id.clone(),
                        thread_id: reviewer_id,
                        status: if result.refused {
                            "refused"
                        } else if result.failure_kind.is_some() {
                            "failed"
                        } else {
                            "completed"
                        }
                        .into(),
                        stop_reason: Some(result.stop_reason.clone()),
                        failure_kind: result
                            .failure_kind
                            .map(|kind| kind.as_str().to_owned()),
                        detail: result.failure.clone().or_else(|| {
                            result
                                .refused
                                .then(|| result.text.chars().take(2_000).collect())
                        }),
                    })
                    .map_err(err)?;
            }
            if result.refused {
                return Err(format!(
                    "reviewer provider refused the request (stop reason: {}): {}",
                    result.stop_reason, result.text
                ));
            }
            if let Some(failure) = &result.failure {
                return Err(format!(
                    "reviewer provider failed ({}): {failure}",
                    result.stop_reason
                ));
            }
            parse_automated_review(&result.text, &w.steps)
        }
        ModelProvider::Codex => {
            let answer = codex_review_turn(
                state,
                codex_thread.as_ref().ok_or("reviewer thread is missing")?,
                &target,
                payload,
            )
            .await?;
            parse_automated_review(&answer, &w.steps)
        }
        }
    }
    .await;
    let mut output = attempted.unwrap_or_else(|error| AutomatedReviewOutput {
        verdict: AutomatedVerdict::Inconclusive,
        summary: format!("Automated reviewer could not establish a verdict: {error}"),
        findings: vec![error],
        changes: Vec::new(),
    });
    if let Err(error) = output.validate(&w.steps) {
        output = AutomatedReviewOutput {
            verdict: AutomatedVerdict::Inconclusive,
            summary: format!("Automated reviewer returned an invalid result: {error}"),
            findings: vec![error],
            changes: Vec::new(),
        };
    }
    let manager = GitWorktrees::new(&w.source_root).map_err(err)?;
    let current_diff = manager.diff(artifact).await.map_err(err)?;
    let reviewer_changed_copy = if let Some(copy) = &review_copy {
        manager.diff(copy).await.map_err(err)? != diff
    } else {
        false
    };
    if artifact_version(&w.source_head, &current_diff) != version || reviewer_changed_copy {
        output = AutomatedReviewOutput {
            verdict: AutomatedVerdict::Inconclusive,
            summary: "Reviewer session changed a reviewed artifact; its verdict was discarded"
                .into(),
            findings: vec![if reviewer_changed_copy {
                "reviewer modified its isolated review copy".into()
            } else {
                "artifact hash changed during read-only review".into()
            }],
            changes: Vec::new(),
        };
    }
    let verdict_name = match output.verdict {
        AutomatedVerdict::Pass => "pass",
        AutomatedVerdict::Rework => "rework",
        AutomatedVerdict::Inconclusive => "inconclusive",
    };
    {
        let store = state.store.lock().map_err(err)?;
        for (_, run_id) in &run_ids {
            store
                .finish_review_run(run_id, version, verdict_name, &json!(output))
                .map_err(err)?;
        }
        if let Some(mut reviewer) = store
            .threads()
            .map_err(err)?
            .into_iter()
            .find(|thread| thread.id == reviewer_id)
        {
            reviewer.status = "completed".into();
            store.save_thread(&reviewer).map_err(err)?;
        }
    }
    match output.verdict {
        AutomatedVerdict::Pass => {
            let mut next = read(state, id)?;
            let review = crate::manifest::ReviewManifest {
                version: 1,
                manifest_id: format!("auto-review-{}-{}", next.iteration, reviewer_id),
                project_id: next.thread.project_id.to_string(),
                task_id: id.to_string(),
                artifact_version: version.into(),
                verdict: crate::manifest::Verdict::Pass,
                summary: output.summary,
                findings: output.findings,
                changes: Vec::new(),
            };
            prepare_review(&mut next, &review)?;
            update(state, id, |workflow| *workflow = next.clone())?;
            Ok(false)
        }
        AutomatedVerdict::Rework if w.iteration < 2 => {
            let mut next = read(state, id)?;
            let review = crate::manifest::ReviewManifest {
                version: 1,
                manifest_id: format!("auto-review-{}-{}", next.iteration, reviewer_id),
                project_id: next.thread.project_id.to_string(),
                task_id: id.to_string(),
                artifact_version: version.into(),
                verdict: crate::manifest::Verdict::Fail,
                summary: output.summary,
                findings: output.findings,
                changes: output.changes,
            };
            let rerun = prepare_review(&mut next, &review)?;
            update(state, id, |workflow| *workflow = next.clone())?;
            Ok(rerun)
        }
        AutomatedVerdict::Rework | AutomatedVerdict::Inconclusive => {
            update(state, id, |workflow| {
                workflow.review = Some(json!(output));
                workflow.thread.status = "needs_attention".into();
                workflow.error = Some(if matches!(output.verdict, AutomatedVerdict::Rework) {
                    "review rework limit reached after two attempts".into()
                } else {
                    "automated review was inconclusive".into()
                });
                workflow.messages.push(Message {
                    role: "assistant".into(),
                    text: workflow.error.clone().unwrap_or_default(),
                });
            })?;
            Ok(false)
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
    for step in &initial.steps {
        check(&state, id)?;
        crate::models::validate_worker_target(&step.target, &state).await?;
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
    'review_iterations: loop {
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
            w.messages.push(Message {
                role: "assistant".into(),
                text: format!(
                    "実装を統合しました。別セッションreviewerで成果物 {} を検証します。",
                    version
                ),
            });
        })?;
        if automated_review(&state, id, &artifact, &diff, &version).await? {
            continue 'review_iterations;
        }
        break 'review_iterations;
    }
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

fn durable_task(w: &AutonomousSnapshot, i: usize) -> hub_core::Task {
    let step = &w.steps[i];
    let task_ids = w
        .steps
        .iter()
        .map(|item| (item.step.key.as_str(), item.task_id))
        .collect::<HashMap<_, _>>();
    hub_core::Task {
        id: step.task_id,
        project_id: w.thread.project_id,
        plan_id: None,
        parent_task_id: None,
        title: step.step.title.clone(),
        description: step.step.goal.clone(),
        dependencies: step
            .step
            .dependencies
            .iter()
            .filter_map(|key| task_ids.get(key.as_str()).copied())
            .collect(),
        preferred_executor: hub_core::ExecutorPreference::Auto,
        assigned_executor: Some(match step.target.provider {
            ModelProvider::Local => hub_core::ExecutorKind::Spark,
            ModelProvider::Codex => hub_core::ExecutorKind::Codex,
            ModelProvider::Api => hub_core::ExecutorKind::Api,
        }),
        risk: match step.step.risk.as_deref() {
            Some("low") => hub_core::RiskLevel::Low,
            Some("high") => hub_core::RiskLevel::High,
            _ => hub_core::RiskLevel::Medium,
        },
        complexity: match step.step.level {
            0 | 1 => hub_core::Complexity::Trivial,
            2 | 3 => hub_core::Complexity::Normal,
            _ => hub_core::Complexity::Deep,
        },
        status: hub_core::TaskStatus::Ready,
        context_capsule_id: None,
        worktree_id: None,
        attempts: w.iteration.saturating_sub(1),
        max_attempts: 2,
    }
}

async fn worker(state: &AppState, id: HubThreadId, i: usize) -> Result<()> {
    check(state, id)?;
    let w = read(state, id)?;
    let s = &w.steps[i];
    {
        let store = state.store.lock().map_err(err)?;
        if !store
            .tasks(w.thread.project_id)
            .map_err(err)?
            .iter()
            .any(|task| task.id == s.task_id)
        {
            let task = durable_task(&w, i);
            store.save_task(&task).map_err(err)?;
            store.save_dependencies(&task).map_err(err)?;
        }
    }
    let payload = capsule(&w, i)?;
    let before = unabridged_capsule(&payload, &w)?;
    save_context_metrics(
        state,
        &w,
        &s.step.key,
        &before,
        &payload,
        WORKER_TOKEN_TARGET,
        CAPSULE_LIMIT,
    )?;
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
                    role: s.agent_name.clone(),
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
        ModelProvider::Api => api_worker(state, id, i, &worktree, payload).await?,
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

async fn api_worker(
    state: &AppState,
    id: HubThreadId,
    i: usize,
    worktree: &Worktree,
    payload: String,
) -> Result<String> {
    use protocol_types::providers::{
        ContentBlock, ModelTarget as ApiTarget, TargetResolution, TranscriptMessage, TranscriptRole,
    };
    let step = read(state, id)?.steps[i].clone();
    let profile_id = step
        .target
        .profile_id
        .clone()
        .ok_or("API worker profile is missing")?;
    let (profile, transport) = {
        let registry = state.providers.read().await;
        let profile = registry
            .profile(&profile_id)
            .cloned()
            .ok_or("API worker profile is unavailable")?;
        let transport = registry.transport(&profile_id).map_err(err)?;
        (profile, transport)
    };
    let turn_id = TaskId::default().to_string();
    let project_id = read(state, id)?.thread.project_id;
    let target = ApiTarget {
        profile_id: profile_id.clone(),
        model_id: step.target.model.clone(),
        effort: step.target.reasoning.clone(),
    };
    let (child, segment_id, provider_thread_id, transcript, persisted_count) = {
        let store = state.store.lock().map_err(err)?;
        let (child, segment_id, provider_thread_id, mut transcript) =
            if let Some(child) = step.child_id {
                let mut mapping = store
                    .threads()
                    .map_err(err)?
                    .into_iter()
                    .find(|mapping| mapping.id == child)
                    .ok_or("saved API worker session is missing")?;
                if mapping.provider != format!("api:{profile_id}") {
                    return Err("saved API worker provider does not match the frozen target".into());
                }
                let segment = store
                    .active_provider_segment(child)
                    .map_err(err)?
                    .ok_or("saved API worker segment is missing")?;
                if segment.target != target || segment.profile_revision != profile.revision {
                    return Err(
                    "API provider profile or frozen target changed before rework; no request sent"
                        .into(),
                );
                }
                let transcript = store
                    .provider_messages(child)
                    .map_err(err)?
                    .into_iter()
                    .map(|message| TranscriptMessage {
                        role: message.role,
                        content: message.content,
                        provider_state: (message.segment_id.as_deref() == Some(&segment.id))
                            .then_some(message.provider_state)
                            .flatten(),
                    })
                    .collect();
                mapping.status = "dispatching".into();
                store.save_thread(&mapping).map_err(err)?;
                (child, segment.id, mapping.provider_thread_id, transcript)
            } else {
                let child = HubThreadId::default();
                let segment_id = TaskId::default().to_string();
                let provider_thread_id = format!("api-worker:{child}");
                store
                    .save_thread(&ThreadMapping {
                        id: child,
                        project_id,
                        provider: format!("api:{profile_id}"),
                        provider_thread_id: provider_thread_id.clone(),
                        title: step.step.title.clone(),
                        status: "dispatching".into(),
                    })
                    .map_err(err)?;
                store
                    .set_session_model_policy(
                        child,
                        &protocol_types::providers::SessionModelPolicy {
                            default_target: target.clone(),
                            reviewer_target: None,
                            allow_turn_override: false,
                        },
                    )
                    .map_err(err)?;
                store
                    .start_provider_segment(&hub_db::ProviderSegmentRecord {
                        id: segment_id.clone(),
                        thread_id: child,
                        target: target.clone(),
                        profile_revision: profile.revision,
                        provider_thread_id: None,
                        ended: false,
                    })
                    .map_err(err)?;
                store
                    .set_setting(&format!("autonomous_parent:{child}"), &id.to_string())
                    .map_err(err)?;
                (child, segment_id, provider_thread_id, Vec::new())
            };
        store
            .record_turn_target(&hub_db::TurnTargetRecord {
                turn_id: turn_id.clone(),
                thread_id: child,
                segment_id: segment_id.clone(),
                target: target.clone(),
                profile_revision: profile.revision,
                resolved_from: if step.step.target_override.is_some() {
                    TargetResolution::PlanStepOverride
                } else {
                    TargetResolution::DifficultyDefault
                },
            })
            .map_err(err)?;
        let user_message = TranscriptMessage {
            role: TranscriptRole::User,
            content: vec![ContentBlock::Text {
                text: payload.clone(),
            }],
            provider_state: None,
        };
        store
            .append_provider_message(&hub_db::ProviderMessageRecord {
                id: TaskId::default().to_string(),
                thread_id: child,
                segment_id: Some(segment_id.clone()),
                provider_turn_id: Some(turn_id.clone()),
                role: user_message.role,
                content: user_message.content.clone(),
                provider_state: None,
            })
            .map_err(err)?;
        transcript.push(user_message);
        let persisted_count = transcript.len();
        (
            child,
            segment_id,
            provider_thread_id,
            transcript,
            persisted_count,
        )
    };
    update(state, id, |workflow| {
        workflow.steps[i].child_id = Some(child);
        workflow.steps[i].dispatch_phase = Some("api_turn_start_attempted".into());
        let turn = Some(ProviderTurn {
            thread_id: provider_thread_id.clone(),
            id: turn_id.clone(),
            status: "inProgress".into(),
        });
        if let Some(existing) = workflow.children.iter_mut().find(|item| item.id == child) {
            existing.turn = turn;
        } else {
            workflow.children.push(Child {
                id: child,
                role: step.agent_name.clone(),
                provider: format!("api:{profile_id}"),
                provider_thread: Some(ProviderThread {
                    id: provider_thread_id.clone(),
                }),
                turn,
            });
        }
    })?;
    check(state, id)?;
    let system = "You are an implementation worker in an isolated Git worktree. The user message is an A2A v1 envelope; its context-capsule data Part is the task data and current authority boundary. Inspect only the workspace with the provided tools, make the smallest coherent change, and verify acceptance criteria with sandboxed commands. Do not delegate, access the network, install packages, commit, merge, or claim tests passed unless a command proved it. Finish with a concise implementation and verification summary.".to_owned();
    let started = std::time::Instant::now();
    let result = hub_runtime::api_worker::ApiAgent {
        root: worktree.path.clone(),
        logical_thread_id: child.to_string(),
        logical_turn_id: turn_id.clone(),
        target,
        transport,
        external_tools: Vec::new(),
        external_handler: None,
        command_authorizer: Some(state.command_approvals.clone()),
        allow_writes: true,
        allow_commands: true,
    }
    .run_with_history(system, transcript)
    .await;
    let result = match result {
        Ok(result) => result,
        Err(error) => {
            let store = state.store.lock().map_err(err)?;
            store
                .record_provider_turn_outcome(&hub_db::ProviderTurnOutcomeRecord {
                    turn_id: turn_id.clone(),
                    thread_id: child,
                    status: "failed".into(),
                    stop_reason: None,
                    failure_kind: Some(provider_api::failure_kind(&error).as_str().into()),
                    detail: Some(
                        hub_policy::redact(&format!("{error:#}"))
                            .chars()
                            .take(2_000)
                            .collect(),
                    ),
                })
                .map_err(err)?;
            if let Some(mut mapping) = store
                .threads()
                .map_err(err)?
                .into_iter()
                .find(|mapping| mapping.id == child)
            {
                mapping.status = "failed".into();
                store.save_thread(&mapping).map_err(err)?;
            }
            drop(store);
            update(state, id, |workflow| {
                if let Some(saved) = workflow.children.iter_mut().find(|item| item.id == child) {
                    if let Some(turn) = saved.turn.as_mut() {
                        turn.status = "failed".into();
                    }
                }
            })?;
            return Err(err(error));
        }
    };
    let revision = artifact_version(
        &worktree.base_sha,
        &GitWorktrees::new(&read(state, id)?.source_root)
            .map_err(err)?
            .diff(worktree)
            .await
            .map_err(err)?,
    );
    let command_evidence = result
        .tool_activity
        .iter()
        .filter(|activity| activity["name"] == "workspace_command")
        .map(|activity| {
            json!({
                "kind":"command",
                "status":if activity["success"] == true { "passed" } else { "failed" },
                "success":activity["success"],
                "command":activity["arguments"]["argv"],
                "exit_code":if activity["success"] == true { json!(0) } else { Value::Null },
                "finished_at_unix_ms":observed_ms(),
                "target_revision":revision,
                "output":activity["output"],
                "acceptance_status":"reported_by_sandbox_runner"
            })
        })
        .collect::<Vec<_>>();
    {
        let store = state.store.lock().map_err(err)?;
        for message in result.transcript.iter().skip(persisted_count) {
            store
                .append_provider_message(&hub_db::ProviderMessageRecord {
                    id: TaskId::default().to_string(),
                    thread_id: child,
                    segment_id: Some(segment_id.clone()),
                    provider_turn_id: Some(turn_id.clone()),
                    role: message.role,
                    content: message.content.clone(),
                    provider_state: message.provider_state.clone(),
                })
                .map_err(err)?;
        }
        store
            .record_usage(&hub_core::ModelUsageRecord {
                id: TaskId::default().to_string(),
                provider: profile_id.clone(),
                model: step.target.model.clone(),
                task_id: Some(step.task_id),
                turn_id: Some(turn_id.clone()),
                prompt_tokens: Some(result.usage.input_tokens),
                cached_tokens: Some(result.usage.cached_input_tokens),
                completion_tokens: Some(result.usage.output_tokens),
                estimated_cost: None,
                latency_ms: Some(started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64),
            })
            .map_err(err)?;
        store
            .record_provider_turn_outcome(&hub_db::ProviderTurnOutcomeRecord {
                turn_id: turn_id.clone(),
                thread_id: child,
                status: if result.refused {
                    "refused"
                } else if result.failure_kind.is_some() {
                    "failed"
                } else {
                    "completed"
                }
                .into(),
                stop_reason: Some(result.stop_reason.clone()),
                failure_kind: result.failure_kind.map(|kind| kind.as_str().to_owned()),
                detail: result.failure.clone().or_else(|| {
                    result
                        .refused
                        .then(|| result.text.chars().take(2_000).collect())
                }),
            })
            .map_err(err)?;
        if let Some(mut mapping) = store
            .threads()
            .map_err(err)?
            .into_iter()
            .find(|mapping| mapping.id == child)
        {
            mapping.status = if result.failure_kind.is_some() {
                "failed"
            } else {
                "completed"
            }
            .into();
            store.save_thread(&mapping).map_err(err)?;
        }
    }
    update(state, id, |workflow| {
        if let Some(saved) = workflow.children.iter_mut().find(|item| item.id == child) {
            if let Some(turn) = saved.turn.as_mut() {
                turn.status = if result.failure_kind.is_some() {
                    "failed"
                } else {
                    "completed"
                }
                .into();
            }
        }
        workflow.steps[i].usage.push(json!({
            "provider": profile_id,
            "model": step.target.model,
            "usage": result.usage,
        }));
        workflow.steps[i]
            .verification
            .extend(command_evidence.clone());
        if workflow.steps[i].verification.is_empty() {
            workflow.steps[i].verification.push(json!({
                "kind":"acceptance",
                "status":"not_observed",
                "success":false,
                "target_revision":null,
                "output":"The API worker returned no host-side command evidence; reviewer must verify the diff and acceptance criteria."
            }));
        }
    })?;
    if result.refused {
        return Err(format!(
            "worker provider refused the request (stop reason: {}): {}",
            result.stop_reason, result.text
        ));
    }
    if let Some(failure) = result.failure {
        return Err(format!(
            "worker provider failed (stop reason: {}): {failure}",
            result.stop_reason
        ));
    }
    Ok(result.text)
}

async fn codex_worker(
    state: &AppState,
    id: HubThreadId,
    i: usize,
    worktree: &Worktree,
    payload: String,
) -> Result<String> {
    let s = read(state, id)?.steps[i].clone();
    let mut handoff: protocol_types::a2a::Message = serde_json::from_str(&payload).map_err(err)?;
    let data = handoff
        .parts
        .iter_mut()
        .find_map(|part| part.data.as_mut())
        .and_then(Value::as_object_mut)
        .ok_or("A2A worker handoff is missing its data Part")?;
    data.insert("working_directory".into(), json!(worktree.path));
    handoff.validate().map_err(err)?;
    let payload = bounded(serde_json::to_string(&handoff).map_err(err)?, CAPSULE_LIMIT)?;
    let snapshot = read(state, id)?;
    let before = unabridged_capsule(&payload, &snapshot)?;
    save_context_metrics(
        state,
        &snapshot,
        &s.step.key,
        &before,
        &payload,
        WORKER_TOKEN_TARGET,
        CAPSULE_LIMIT,
    )?;
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
                role: s.agent_name.clone(),
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
    let turn_started = std::time::Instant::now();
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
                    let (full_log, output_redacted) = command_log(&event.text)?;
                    state
                        .store
                        .lock()
                        .map_err(err)?
                        .set_setting(&log_ref, &full_log)
                        .map_err(err)?;
                    let evidence = json!({"kind":"command","status":match exit_code {Some(0)=>"passed",Some(_)=>"failed",None=>"unknown"},"success":*exit_code==Some(0),"command":command,"exit_code":exit_code,"started_at_unix_ms":command_starts.remove(&event.item_id),"finished_at_unix_ms":observed_ms(),"clock":"Localoud event receipt time","target_revision":revision,"revision_observation":"filesystem snapshot after command-completed event; not an attestation of the exact revision during command execution","execution_revision_verified":false,"base_revision":worktree.base_sha,"worktree":worktree.path,"log_ref":log_ref,"output":full_log.chars().take(8000).collect::<String>(),"output_truncated":full_log.chars().count()>8000,"output_redacted":output_redacted,"acceptance_status":"unknown"});
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
    update(state, id, |w| {
        let usage = &mut w.steps[i].usage;
        if usage.is_empty() {
            usage.push(json!({"tokens":null}));
        }
        if let Some(last) = usage.last_mut().and_then(Value::as_object_mut) {
            last.insert(
                "latency_ms".into(),
                json!(turn_started.elapsed().as_millis() as u64),
            );
        }
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
