//! Explicit user clipboard handoff. This module is not exposed through MCP.
use crate::{autonomous::PlanStep, AppState};
use hub_core::ThreadMapping;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri_plugin_clipboard_manager::ClipboardExt;

pub const MAX_MANIFEST_BYTES: usize = 48 * 1024;
const MAX_CLIPBOARD_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Manifest {
    Task(TaskManifest),
    Review(ReviewManifest),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskManifest {
    pub version: u32,
    pub manifest_id: String,
    pub project_id: String,
    pub base_revision: String,
    pub request: String,
    pub acceptance: Vec<String>,
    pub scope: Vec<String>,
    pub steps: Vec<PlanStep>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewManifest {
    pub version: u32,
    pub manifest_id: String,
    pub project_id: String,
    pub task_id: String,
    pub artifact_version: String,
    pub verdict: Verdict,
    pub summary: String,
    pub findings: Vec<String>,
    pub changes: Vec<ReviewChange>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Pass,
    Fail,
    Inconclusive,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewChange {
    pub step_key: String,
    pub instruction: String,
}

pub fn parse(text: &str) -> Result<Manifest, String> {
    if text.len() > MAX_CLIPBOARD_BYTES {
        return Err("コピーした回答は256KB以内にしてください".into());
    }
    let text = manifest_payload(text)?;
    if text.len() > MAX_MANIFEST_BYTES {
        return Err("Manifestは48KB以内にしてください".into());
    }
    let mut manifest: Manifest = serde_json::from_str(text).map_err(|e| {
        format!("実行用ManifestのJSON形式が不正です。ChatGPTの回答にTaskManifestまたはReviewManifestのJSONコードブロックを1つ含めてください: {e}")
    })?;
    let (version, id, project) = match &manifest {
        Manifest::Task(m) => (m.version, &m.manifest_id, &m.project_id),
        Manifest::Review(m) => (m.version, &m.manifest_id, &m.project_id),
    };
    if version != 1 {
        return Err("対応するManifest versionは1です".into());
    }
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
    {
        return Err(
            "manifest_idは英数字・ハイフン・アンダースコアの1〜128文字にしてください".into(),
        );
    }
    let _ = hub_core::ProjectId(project.parse().map_err(|_| "project_idが不正です")?);
    match &mut manifest {
        Manifest::Task(m) => {
            if !revision(&m.base_revision)
                || m.request.trim().is_empty()
                || !nonempty(&m.acceptance)
                || m.scope.is_empty()
                || m.scope.len() > 128
                || !m.scope.iter().all(|p| crate::autonomous::valid_path(p))
            {
                return Err("基準revision・依頼・受け入れ条件・作業範囲を指定してください".into());
            }
            let steps = crate::autonomous::parse_plan(&json!({"steps": m.steps}).to_string())?;
            if steps.iter().any(|s| {
                s.owned_paths.iter().any(|p| {
                    !m.scope
                        .iter()
                        .any(|scope| std::path::Path::new(p).starts_with(scope))
                })
            }) {
                return Err("stepのowned_pathsがManifestのscopeを超えています".into());
            }
            // Preview and execution must share the scheduler's ownership ordering.
            m.steps = steps;
        }
        Manifest::Review(m) => {
            let _ = hub_core::HubThreadId(m.task_id.parse().map_err(|_| "task_idが不正です")?);
            if !revision(&m.artifact_version)
                || m.summary.trim().is_empty()
                || m.findings.iter().any(|f| f.trim().is_empty())
                || m.changes.len() > 8
                || m.changes
                    .iter()
                    .any(|c| c.step_key.is_empty() || c.instruction.trim().is_empty())
            {
                return Err("レビュー対象の版・結論・修正対象を確認してください".into());
            }
            let keys: std::collections::HashSet<_> =
                m.changes.iter().map(|c| &c.step_key).collect();
            if keys.len() != m.changes.len()
                || (m.verdict == Verdict::Pass && !m.changes.is_empty())
            {
                return Err("重複した修正対象、または合格判定と修正要求の矛盾があります".into());
            }
        }
    }
    Ok(manifest)
}

fn manifest_payload(text: &str) -> Result<&str, String> {
    // Some browser/native clipboard implementations preserve a UTF-8 BOM
    // when copying a code block. `str::trim` intentionally does not remove
    // U+FEFF, so strip it before looking for a raw JSON payload or fences.
    let text = text.trim().trim_start_matches('\u{feff}').trim();
    let mut json_blocks = Vec::new();
    let mut cursor = 0;
    while let Some(relative_start) = text[cursor..].find("```") {
        let fence_start = cursor + relative_start;
        let after_fence = &text[fence_start + 3..];
        let Some(header_end) = after_fence.find('\n') else {
            break;
        };
        let language = after_fence[..header_end].trim();
        let body_start = fence_start + 3 + header_end + 1;
        let Some(relative_end) = text[body_start..].find("```") else {
            return Err("JSONコードブロックが閉じていません".into());
        };
        let body_end = body_start + relative_end;
        if language.eq_ignore_ascii_case("json") {
            json_blocks.push(text[body_start..body_end].trim());
        }
        cursor = body_end + 3;
    }
    match json_blocks.len() {
        1 => return Ok(json_blocks[0]),
        count if count > 1 => {
            return Err(
                "実行用JSONコードブロックが複数あります。取り込む1つだけをコピーしてください"
                    .into(),
            )
        }
        _ => {}
    }
    if let Some(body) = text
        .strip_prefix("```json\n")
        .or_else(|| text.strip_prefix("```\n"))
    {
        return Ok(body
            .strip_suffix("```")
            .ok_or("JSONコードブロックが閉じていません")?
            .trim());
    }
    Ok(text)
}
fn nonempty(values: &[String]) -> bool {
    !values.is_empty() && values.iter().all(|v| !v.trim().is_empty())
}
pub fn revision(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|c| c.is_ascii_hexdigit())
}

#[derive(Serialize)]
pub struct ImportOutcome {
    thread: ThreadMapping,
    imported: bool,
}

pub(crate) fn existing_task(
    store: &hub_db::Store,
    manifest_id: &str,
    project_id: &str,
) -> Result<Option<ThreadMapping>, String> {
    let Some(receipt) = store
        .setting(&format!("manifest_receipt:{manifest_id}"))
        .map_err(|e| e.to_string())?
    else {
        return Ok(None);
    };
    let receipt: Value = serde_json::from_str(&receipt).map_err(|e| e.to_string())?;
    let id = receipt["thread_id"]
        .as_str()
        .ok_or("取り込み記録にタスクIDがありません")?;
    let saved = store
        .setting(&format!("autonomous:{id}"))
        .map_err(|e| e.to_string())?
        .ok_or("保存済みタスクが見つかりません")?;
    let snapshot: crate::autonomous::AutonomousSnapshot =
        serde_json::from_str(&saved).map_err(|e| e.to_string())?;
    if snapshot.thread.project_id.to_string() != project_id {
        return Err("取り込み済みManifestのプロジェクトが一致しません".into());
    }
    // Reopen the saved record only. Never replace its Manifest or replay workers.
    store
        .save_thread(&snapshot.thread)
        .map_err(|e| e.to_string())?;
    store
        .set_thread_archived(snapshot.thread.id, false)
        .map_err(|e| e.to_string())?;
    Ok(Some(snapshot.thread))
}

#[tauri::command]
pub async fn manifest_import_clipboard(
    project_id: String,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<ImportOutcome, String> {
    let text = app
        .clipboard()
        .read_text()
        .map_err(|e| format!("クリップボードを読めません: {e}"))?;
    manifest_import_text(project_id, text, app, state).await
}

fn parse_for_project(text: &str, project_id: &str) -> Result<Manifest, String> {
    let manifest = parse(text)?;
    let target = match &manifest {
        Manifest::Task(m) => &m.project_id,
        Manifest::Review(m) => &m.project_id,
    };
    if target != project_id {
        return Err("選択中のプロジェクトとManifestが一致しません".into());
    }
    Ok(manifest)
}

/// Preview only: reading and parsing does not create a receipt or run workers.
#[tauri::command]
pub fn manifest_preview_clipboard(
    project_id: String,
    app: tauri::AppHandle,
) -> Result<Manifest, String> {
    let text = app
        .clipboard()
        .read_text()
        .map_err(|e| format!("クリップボードを読めません: {e}"))?;
    parse_for_project(&text, &project_id)
}

/// Import the exact payload that was displayed, even if the clipboard changed.
#[tauri::command]
pub async fn manifest_import_text(
    project_id: String,
    text: String,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<ImportOutcome, String> {
    let manifest = parse_for_project(&text, &project_id)?;
    let _lock = state.workflow_lock.lock().await;
    let manifest_id = match &manifest {
        Manifest::Task(m) => &m.manifest_id,
        Manifest::Review(m) => &m.manifest_id,
    };
    {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        if let Some(thread) = existing_task(&store, manifest_id, &project_id)? {
            return Ok(ImportOutcome {
                thread,
                imported: false,
            });
        }
    }
    let thread = match manifest {
        Manifest::Task(m) => crate::autonomous::import_task(m, app, state.clone()).await?,
        Manifest::Review(m) => crate::autonomous::import_review(m, app, state.clone()).await?,
    };
    Ok(ImportOutcome {
        thread,
        imported: true,
    })
}

pub fn examples() -> Value {
    json!({
        "instructions":"Return exactly one JSON object, kind task or review, version 1. Use project_id and full base_revision from project_get; use task_id and artifact_version from task_get for reviews. manifest_id must be unique. The user copies this object into Localoud; MCP never executes it. Every step needs acceptance checks. Difficulty levels 1..5 select the user's saved worker routes. Omit target_override unless the user supplied an exact Localoud provider/profile/model selection; the Localoud confirmation screen can set it before execution. Never invent provider IDs. Level 1 requires low risk, 1-2 existing files, <100 changed lines. Paths are relative to the project. Dependencies reference step keys. Max 8 steps and 48KB per Manifest. For rework, target existing step keys and stay within their original owned_paths; downstream steps will rerun with updated inputs. Pass requires a complete review of the exact artifact and verification evidence; missing checks require inconclusive.",
        "task":{"kind":"task","version":1,"manifest_id":"unique-task-id","project_id":"UUID from project_get","base_revision":"full HEAD from project_get","request":"User's request","acceptance":["Overall observable acceptance criteria"],"scope":["src/example.rs"],"steps":[{"key":"implement","title":"Implement the change","goal":"Concrete bounded task","dependencies":[],"level":2,"target_override":null,"owned_paths":["src/example.rs"],"acceptance":["Run the relevant tests and report results"],"risk":"low","estimated_loc":50}]},
        "review":{"kind":"review","version":1,"manifest_id":"unique-review-id","project_id":"same project UUID","task_id":"task UUID from task_list","artifact_version":"exact version from task_get","verdict":"fail","summary":"Review conclusion","findings":["Evidence-backed finding"],"changes":[{"step_key":"implement","instruction":"Fix this issue and rerun the checks"}]},
        "pass":"Use verdict pass with changes [] after verifying every acceptance criterion. Localoud records the explicit review; it does not infer completion from ChatGPT."
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn task() -> Value {
        let mut v = examples()["task"].clone();
        v["project_id"] = json!(hub_core::ProjectId::default());
        v["base_revision"] = json!("a".repeat(40));
        v
    }
    #[test]
    fn accepts_only_complete_bounded_manifests() {
        let mut v = task();
        assert!(parse(&v.to_string()).is_ok());
        assert!(parse(&format!("\u{feff}{v}")).is_ok());
        assert!(parse(&format!("```json\n{v}\n```")).is_ok());
        assert!(parse(&format!(
            "計画をまとめました。\n\n```json\n{v}\n```\n\nこの回答をコピーしてください。"
        ))
        .is_ok());
        assert!(parse(&format!("```json\n{v}\n```\n```json\n{v}\n```"))
            .unwrap_err()
            .contains("複数"));
        assert!(parse(&format!("Explanation\n{v}")).is_err());
        v["version"] = json!(2);
        assert!(parse(&v.to_string()).is_err());
        v["version"] = json!(1);
        v["model"] = json!("override");
        assert!(parse(&v.to_string()).is_err());
        v.as_object_mut().unwrap().remove("model");
        v["scope"] = json!(["other"]);
        assert!(parse(&v.to_string()).is_err());
        v["scope"] = json!(["../escape"]);
        assert!(parse(&v.to_string()).is_err());
    }
    #[test]
    fn preview_and_import_bind_the_payload_to_the_selected_project() {
        let v = task();
        let project = v["project_id"].as_str().unwrap();
        let preview = parse_for_project(&v.to_string(), project).unwrap();
        let captured = serde_json::to_string(&preview).unwrap();
        assert!(parse_for_project(&captured, project).is_ok());
        assert!(parse_for_project(&captured, &hub_core::ProjectId::default().to_string()).is_err());
        assert!(parse_for_project("not a plan", project).is_err());
    }
    #[test]
    fn preview_contains_the_same_ownership_edges_as_execution() {
        let mut v = task();
        let first = v["steps"][0].clone();
        let mut second = first.clone();
        second["key"] = json!("followup");
        v["steps"] = json!([first, second]);
        let Manifest::Task(preview) = parse(&v.to_string()).unwrap() else {
            panic!("task")
        };
        assert_eq!(preview.steps[1].dependencies, vec!["implement"]);
        let executed =
            crate::autonomous::parse_plan(&json!({"steps": preview.steps}).to_string()).unwrap();
        assert_eq!(
            serde_json::to_value(&preview.steps).unwrap(),
            serde_json::to_value(executed).unwrap()
        );
    }
    #[test]
    fn rejects_contradictory_review_and_invalid_ids() {
        let mut v = examples()["review"].clone();
        v["project_id"] = json!(hub_core::ProjectId::default());
        v["task_id"] = json!(hub_core::HubThreadId::default());
        v["artifact_version"] = json!("a".repeat(40));
        assert!(parse(&v.to_string()).is_ok());
        v["verdict"] = json!("pass");
        assert!(parse(&v.to_string()).is_err());
        v["changes"] = json!([]);
        assert!(parse(&v.to_string()).is_ok());
        v["manifest_id"] = json!("../bad");
        assert!(parse(&v.to_string()).is_err());
    }
}
