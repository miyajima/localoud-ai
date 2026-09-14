use super::*;

#[test]
fn command_log_redacts_without_aborting_worker_evidence() {
    let raw = "example: api_key=synthetic-test-value\nverification failed";
    let (log, redacted) = command_log(raw).unwrap();
    assert!(redacted);
    assert!(!log.contains("synthetic-test-value"));
    assert!(log.contains("verification failed"));
    assert!(bounded(raw.into(), REVIEW_LIMIT).is_err());
}

#[test]
fn command_log_preserves_plain_output_and_size_limit() {
    assert_eq!(
        command_log("exit 1: missing file").unwrap(),
        ("exit 1: missing file".into(), false)
    );
    assert!(command_log(&"x".repeat(REVIEW_LIMIT + 1)).is_err());
}

fn step(key: &str, paths: &[&str], dependencies: &[&str]) -> PlanStep {
    PlanStep {
        key: key.into(),
        title: key.into(),
        goal: "implement a bounded change".into(),
        dependencies: dependencies.iter().map(|s| s.to_string()).collect(),
        level: 2,
        target_override: None,
        owned_paths: paths.iter().map(|s| s.to_string()).collect(),
        acceptance: vec!["verify the changed behavior".into()],
        risk: Some("low".into()),
        estimated_loc: Some(10),
    }
}
fn target() -> ModelTarget {
    ModelTarget {
        provider: ModelProvider::Codex,
        profile_id: Some("codex".into()),
        model: "saved-model".into(),
        reasoning: Some("xhigh".into()),
    }
}
fn task(step: PlanStep) -> StepState {
    StepState {
        task_id: TaskId::default(),
        step,
        target: target(),
        agent_name: "Builder".into(),
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
    }
}
fn workflow(steps: Vec<StepState>) -> AutonomousSnapshot {
    let id = HubThreadId::default();
    AutonomousSnapshot {
        thread: ThreadMapping {
            id,
            project_id: ProjectId::default(),
            provider: "autonomous".into(),
            provider_thread_id: id.to_string(),
            title: "fixture".into(),
            status: "running".into(),
        },
        request: AutonomousRequest {
            project_id: "fixture".into(),
            text: "original request MUST NOT enter worker capsules".into(),
        },
        source_root: PathBuf::from("/fixture"),
        source_head: "a".repeat(40),
        settings_snapshot: AutoSettings::initial("local".into(), Some(target())),
        manifest: None,
        artifact_version: None,
        iteration: 1,
        history: vec![],
        steps,
        children: vec![],
        messages: vec![],
        stop_requested: false,
        artifact: None,
        final_diff: None,
        review: None,
        error: None,
    }
}
fn completed(s: &mut StepState) {
    s.status = "completed".into();
    s.output = Some("verified output".into());
    s.patch = Some(String::new());
    s.verification = vec![json!({"success":true,"output":"test passed"})];
}
fn parse(steps: Vec<PlanStep>) -> Result<Vec<PlanStep>> {
    parse_plan(&json!({"steps":steps}).to_string())
}

#[test]
fn malformed_unknown_and_cyclic_plans_are_rejected() {
    for text in ["not json", "{}", "{\"steps\":[]}", "```json\n{}\n```"] {
        assert!(parse_plan(text).is_err());
    }
    assert!(parse(vec![step("a", &["a"], &["missing"])]).is_err());
    assert!(parse(vec![step("a", &["a"], &["b"]), step("b", &["b"], &["a"])]).is_err());
    assert!(parse(vec![step("a", &["a"], &[]), step("a", &["b"], &[])]).is_err());
    let mut s = step("a", &["a"], &[]);
    s.level = 6;
    assert!(parse(vec![s]).is_err());
    for path in [
        "../escape",
        "/abs",
        "a/../b",
        "a/./b",
        "a//b",
        ".git/config",
        ".agent-worktrees/x",
        "a\\b",
    ] {
        assert!(parse(vec![step("a", &[path], &[])]).is_err(), "{path}");
    }
    assert!(parse(
        (0..9)
            .map(|i| step(&format!("s{i}"), &["src"], &[]))
            .collect()
    )
    .is_err());
}
#[test]
fn saved_routes_preserve_every_model_and_exact_effort() {
    let mut config = AutoSettings::initial("spark-exact".into(), Some(target()));
    for assignment in &mut config.levels {
        assignment.target = ModelTarget {
            provider: ModelProvider::Codex,
            profile_id: Some("codex".into()),
            model: format!("model-{}", assignment.level),
            reasoning: Some(format!("effort-{}", assignment.level)),
        };
    }
    let frozen: AutoSettings =
        serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
    config.levels[2].target.reasoning = Some("changed-after-run".into());
    for level in 1..=5 {
        let t = frozen.target(level).unwrap();
        assert_eq!(t.model, format!("model-{level}"));
        assert_eq!(t.reasoning, Some(format!("effort-{level}")));
    }
    assert_eq!(frozen.agent_for_level(3).unwrap().name, "Builder");
    assert!(frozen.target(0).is_err());
}
#[test]
fn plan_step_override_is_validated_and_frozen_in_the_step() {
    let mut overridden = step("override", &["src/lib.rs"], &[]);
    overridden.target_override = Some(ModelTarget {
        provider: ModelProvider::Api,
        profile_id: Some("anthropic-prod".into()),
        model: "claude-model".into(),
        reasoning: Some("high".into()),
    });
    let parsed = parse(vec![overridden.clone()]).unwrap();
    assert_eq!(parsed[0].target_override, overridden.target_override);

    overridden.target_override.as_mut().unwrap().profile_id = None;
    assert!(parse(vec![overridden]).is_err());
}
#[test]
fn conflicts_are_serialized_including_directory_ownership() {
    let ordered = parse(vec![
        step("a", &["src"], &[]),
        step("b", &["src/lib.rs"], &[]),
        step("c", &["src2/lib.rs"], &[]),
        step("d", &["docs"], &[]),
        step("e", &["tests"], &[]),
    ])
    .unwrap();
    assert!(ordered[1].dependencies.contains(&"a".into()));
    let mut tasks: Vec<_> = ordered.into_iter().map(task).collect();
    assert_eq!(ready_batch(&tasks), vec![0, 2, 3]);
    completed(&mut tasks[0]);
    assert_eq!(ready_batch(&tasks), vec![1, 2, 3]);
}
#[test]
fn failed_dependency_blocks_successors_and_review() {
    let mut tasks = vec![
        task(step("a", &["a"], &[])),
        task(step("b", &["b"], &["a"])),
    ];
    tasks[0].status = "failed".into();
    assert!(ready_batch(&tasks).is_empty());
    assert!(capsule(&workflow(tasks.clone()), 1).is_err());
    assert!(evidence_complete(&tasks).is_err());
    completed(&mut tasks[0]);
    completed(&mut tasks[1]);
    assert!(evidence_complete(&tasks).is_ok());
    tasks[0].verification.clear();
    assert!(evidence_complete(&tasks).is_err());
    tasks[0].verification = vec![json!({"success":false})];
    assert!(evidence_complete(&tasks).is_err());
    completed(&mut tasks[0]);
    tasks[1].patch = None;
    assert!(evidence_complete(&tasks).is_err());
    completed(&mut tasks[1]);
    tasks[1].output = Some(" ".into());
    assert!(evidence_complete(&tasks).is_err());
}

#[test]
fn automated_review_contract_is_fail_closed() {
    let steps = vec![task(step("a", &["a"], &[]))];
    let pass = parse_automated_review(
        r#"```json
{"verdict":"pass","summary":"all evidence matches","findings":[],"changes":[]}
```"#,
        &steps,
    )
    .unwrap();
    assert!(matches!(pass.verdict, AutomatedVerdict::Pass));

    for invalid in [
        r#"{"verdict":"pass","summary":"ok","findings":[],"changes":[{"step_key":"a","instruction":"fix"}]}"#,
        r#"{"verdict":"rework","summary":"fix","findings":[],"changes":[]}"#,
        r#"{"verdict":"rework","summary":"fix","findings":[],"changes":[{"step_key":"missing","instruction":"fix"}]}"#,
        r#"{"verdict":"inconclusive","summary":"unknown","findings":[],"changes":[{"step_key":"a","instruction":"fix"}]}"#,
        r#"{"verdict":"pass","summary":"","findings":[],"changes":[]}"#,
        r#"{"verdict":"pass","summary":"ok","findings":[],"changes":[],"extra":true}"#,
    ] {
        assert!(
            parse_automated_review(invalid, &steps).is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn review_pass_completes_and_rework_invalidates_dependents() {
    let mut steps = vec![
        task(step("a", &["a"], &[])),
        task(step("b", &["b"], &["a"])),
    ];
    for step in &mut steps {
        completed(step);
    }
    let mut w = workflow(steps);
    w.manifest = Some(crate::manifest::TaskManifest {
        version: 1,
        manifest_id: "manifest".into(),
        project_id: w.thread.project_id.to_string(),
        base_revision: w.source_head.clone(),
        request: "request".into(),
        acceptance: vec!["done".into()],
        scope: vec!["a".into(), "b".into()],
        steps: w.steps.iter().map(|step| step.step.clone()).collect(),
    });
    w.thread.status = "awaiting_review".into();
    w.artifact_version = Some("b".repeat(64));
    let review_message: protocol_types::a2a::Message = serde_json::from_str(
        &review_payload(
            &w,
            "diff --git a/a b/a",
            w.artifact_version.as_deref().unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    review_message.validate().unwrap();
    assert_eq!(
        review_message.parts[0].media_type.as_deref(),
        Some(protocol_types::a2a::REVIEW_CAPSULE_MEDIA_TYPE)
    );
    assert_eq!(
        review_message.parts[0].data.as_ref().unwrap()["goal"],
        "request"
    );
    assert_eq!(review_message.reference_task_ids.len(), 2);
    let rework = crate::manifest::ReviewManifest {
        version: 1,
        manifest_id: "review-one".into(),
        project_id: w.thread.project_id.to_string(),
        task_id: w.thread.id.to_string(),
        artifact_version: w.artifact_version.clone().unwrap(),
        verdict: crate::manifest::Verdict::Fail,
        summary: "a needs correction".into(),
        findings: vec!["finding".into()],
        changes: vec![crate::manifest::ReviewChange {
            step_key: "a".into(),
            instruction: "correct a".into(),
        }],
    };
    assert!(prepare_review(&mut w, &rework).unwrap());
    assert_eq!(w.iteration, 2);
    assert!(w.steps.iter().all(|step| step.status == "pending"));
    assert!(w.steps[1]
        .revision_instruction
        .as_deref()
        .unwrap()
        .contains("Upstream"));

    for step in &mut w.steps {
        completed(step);
    }
    w.thread.status = "awaiting_review".into();
    w.artifact_version = Some("c".repeat(64));
    let pass = crate::manifest::ReviewManifest {
        version: 1,
        manifest_id: "review-two".into(),
        project_id: w.thread.project_id.to_string(),
        task_id: w.thread.id.to_string(),
        artifact_version: w.artifact_version.clone().unwrap(),
        verdict: crate::manifest::Verdict::Pass,
        summary: "accepted".into(),
        findings: vec![],
        changes: vec![],
    };
    assert!(!prepare_review(&mut w, &pass).unwrap());
    assert_eq!(w.thread.status, "completed");
}
#[test]
fn capsules_are_bounded_and_exclude_orchestration_and_unrelated_outputs() {
    let mut tasks = vec![
        task(step("a", &["a"], &[])),
        task(step("b", &["b"], &["a"])),
        task(step("other", &["c"], &[])),
    ];
    completed(&mut tasks[0]);
    completed(&mut tasks[2]);
    tasks[2].output = Some("UNRELATED TRANSCRIPT".into());
    let mut w = workflow(tasks);
    w.messages.push(Message {
        role: "assistant".into(),
        text: "ORCHESTRATOR TRANSCRIPT".into(),
    });
    let p = capsule(&w, 1).unwrap();
    let envelope: protocol_types::a2a::Message = serde_json::from_str(&p).unwrap();
    envelope.validate().unwrap();
    assert_eq!(envelope.role, protocol_types::a2a::Role::User);
    assert_eq!(
        envelope.parts[0].media_type.as_deref(),
        Some(protocol_types::a2a::CONTEXT_CAPSULE_MEDIA_TYPE)
    );
    assert_eq!(envelope.parts[0].data.as_ref().unwrap()["task"]["key"], "b");
    assert!(envelope.parts[0].data.as_ref().unwrap()["assignment"]
        .get("role_name")
        .is_none());
    assert_eq!(
        envelope.parts[0].data.as_ref().unwrap()["assignment"]["agent_name"],
        "Builder"
    );
    assert_eq!(envelope.reference_task_ids.len(), 1);
    assert!(!p.contains("TRANSCRIPT"));
    assert!(!p.contains("original request"));
    assert!(p.contains("verified output"));
    assert!(bounded("a".repeat(CAPSULE_LIMIT), CAPSULE_LIMIT).is_ok());
    assert!(bounded("a".repeat(CAPSULE_LIMIT + 1), CAPSULE_LIMIT).is_err());
    w.steps[0].output = Some("あ".repeat(CAPSULE_LIMIT / 3));
    assert!(capsule(&w, 1).is_err());
}
#[test]
fn dependency_capsule_keeps_failures_without_copying_large_command_logs() {
    let mut tasks = vec![
        task(step(
            "baseline",
            &["reports/baseline.md", "reports/verification.json"],
            &[],
        )),
        task(step("audit", &["reports/audit.md"], &["baseline"])),
    ];
    completed(&mut tasks[0]);
    tasks[0].verification = (0..99).map(|i| json!({
        "status": if i < 95 { "passed" } else if i < 97 { "failed" } else if i == 97 { "not_run" } else { "unknown" },
        "output": "large log text".repeat(1000),
        "command": "fixture command",
        "log_ref": format!("verification_log:fixture:{i}")
    })).collect();
    let w = workflow(tasks);
    let original = w.steps[0].verification.clone();
    let payload = capsule(&w, 1).unwrap();
    assert!(payload.len() < CAPSULE_LIMIT);
    assert!(!payload.contains(&"large log text".repeat(1000)));
    let envelope: protocol_types::a2a::Message = serde_json::from_str(&payload).unwrap();
    let dependency = &envelope.parts[0].data.as_ref().unwrap()["dependencies"][0];
    assert_eq!(dependency["verification"]["records"], 99);
    assert_eq!(dependency["verification"]["passed"], 95);
    assert_eq!(dependency["verification"]["failed"], 2);
    assert_eq!(dependency["verification"]["not_run"], 1);
    assert_eq!(dependency["verification"]["unknown"], 1);
    assert_eq!(dependency["verification"]["acceptance_status"], "unknown");
    assert_eq!(dependency["artifacts"][1], "reports/verification.json");
    assert_eq!(w.steps[0].verification, original);
}

#[tokio::test]
async fn stop_prevents_dispatch_and_interrupts_inflight_work() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let dispatched = AtomicBool::new(false);
    let result = guard_with(|| Err("stopped".into()), async {
        dispatched.store(true, Ordering::SeqCst);
        Ok(())
    })
    .await;
    assert!(result.is_err());
    assert!(!dispatched.load(Ordering::SeqCst));
    let stop = AtomicBool::new(false);
    let result = guard_with(
        || {
            if stop.load(Ordering::SeqCst) {
                Err("stopped".into())
            } else {
                Ok(())
            }
        },
        async {
            stop.store(true, Ordering::SeqCst);
            std::future::pending::<Result<()>>().await
        },
    )
    .await;
    assert!(result.is_err());
    // Even a simultaneous completion cannot report success after stop.
    stop.store(false, Ordering::SeqCst);
    let result = guard_with(
        || {
            if stop.load(Ordering::SeqCst) {
                Err("stopped".into())
            } else {
                Ok(())
            }
        },
        async {
            stop.store(true, Ordering::SeqCst);
            Ok(())
        },
    )
    .await;
    assert!(result.is_err());
}
#[test]
fn local_scope_never_silently_falls_back() {
    let mut s = step("a", &["a"], &[]);
    assert!(local_scope(&s).is_err());
    s.level = 1;
    assert!(local_scope(&s).is_ok());
    s.estimated_loc = None;
    assert!(local_scope(&s).is_err());
    s.estimated_loc = Some(100);
    assert!(local_scope(&s).is_err());
    s.estimated_loc = Some(99);
    s.risk = Some("high".into());
    assert!(local_scope(&s).is_err());
}
#[test]
fn workflow_capsules_children_and_stop_survive_store_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let mut w = workflow(vec![task(step("a", &["a"], &[]))]);
    w.stop_requested = true;
    w.steps[0].capsule = Some("persist before dispatch".into());
    w.steps[0].dispatch_phase = Some("turn_start_attempted".into());
    w.children.push(Child {
        id: HubThreadId::default(),
        role: "a".into(),
        provider: "codex".into(),
        provider_thread: None,
        turn: None,
    });
    {
        let store = hub_db::Store::open(&path).unwrap();
        store
            .set_setting(&key(w.thread.id), &serde_json::to_string(&w).unwrap())
            .unwrap();
    }
    let store = hub_db::Store::open(&path).unwrap();
    let recovered: AutonomousSnapshot =
        serde_json::from_str(&store.setting(&key(w.thread.id)).unwrap().unwrap()).unwrap();
    assert!(recovered.stop_requested);
    assert_eq!(recovered.children[0].id, w.children[0].id);
    assert_eq!(recovered.steps[0].capsule, w.steps[0].capsule);
    assert_eq!(recovered.settings_snapshot, w.settings_snapshot);
    assert_eq!(recovered.iteration, w.iteration);
}

#[test]
fn review_attention_resume_retries_review_without_reexecuting_steps() {
    let mut state = task(step("a", &["a"], &[]));
    completed(&mut state);
    let original_task = state.task_id;
    let mut w = workflow(vec![state]);
    w.manifest = Some(crate::manifest::TaskManifest {
        version: 1,
        manifest_id: "manifest".into(),
        project_id: w.thread.project_id.to_string(),
        base_revision: w.source_head.clone(),
        request: "request".into(),
        acceptance: vec!["done".into()],
        scope: vec!["a".into()],
        steps: w.steps.iter().map(|step| step.step.clone()).collect(),
    });
    w.thread.status = "needs_attention".into();
    w.artifact_version = Some("d".repeat(64));
    w.final_diff = Some("diff".into());
    prepare_resume(&mut w).unwrap();
    assert_eq!(w.thread.status, "queued");
    assert_eq!(w.iteration, 1);
    assert_eq!(w.steps[0].status, "completed");
    assert_eq!(w.steps[0].task_id, original_task);
    assert!(w.artifact_version.is_none());
    assert_eq!(w.history.len(), 1);
}

#[test]
fn explicit_failed_api_resume_creates_a_fresh_attempt_but_unknown_outcomes_do_not_replay() {
    let mut state = task(step("api", &["src/lib.rs"], &[]));
    state.target = ModelTarget {
        provider: ModelProvider::Api,
        profile_id: Some("fixture-api".into()),
        model: "fixture-model".into(),
        reasoning: None,
    };
    state.status = "failed".into();
    state.child_id = Some(HubThreadId::default());
    let mut workflow = workflow(vec![state]);
    workflow.manifest = Some(crate::manifest::TaskManifest {
        version: 1,
        manifest_id: "fixture".into(),
        project_id: workflow.thread.project_id.to_string(),
        base_revision: workflow.source_head.clone(),
        request: "retry".into(),
        acceptance: vec!["verified".into()],
        scope: vec!["src/lib.rs".into()],
        steps: workflow
            .steps
            .iter()
            .map(|item| item.step.clone())
            .collect(),
    });

    let mut known_failure = workflow.clone();
    known_failure.thread.status = "failed".into();
    prepare_resume(&mut known_failure).unwrap();
    assert!(known_failure.steps[0].child_id.is_none());

    workflow.thread.status = "reconciliation_required".into();
    let original_child = workflow.steps[0].child_id;
    prepare_resume(&mut workflow).unwrap();
    assert_eq!(workflow.steps[0].child_id, original_child);
}
#[tokio::test]
async fn diamond_dependencies_apply_each_incremental_patch_once() {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init"]).await.unwrap();
    std::fs::write(dir.path().join("a.txt"), "base\n").unwrap();
    git(dir.path(), &["add", "a.txt"]).await.unwrap();
    git(
        dir.path(),
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@localhost",
            "commit",
            "-m",
            "fixture",
        ],
    )
    .await
    .unwrap();
    let head = git(dir.path(), &["rev-parse", "HEAD"]).await.unwrap();
    let manager = GitWorktrees::new(dir.path()).unwrap();
    let mut tasks = vec![
        task(step("a", &["a.txt"], &[])),
        task(step("b", &["b.txt"], &["a"])),
        task(step("c", &["c.txt"], &["a"])),
        task(step("d", &["d.txt"], &["b", "c"])),
    ];
    assert_eq!(ancestors(&tasks, 3), vec![0, 1, 2]);
    for i in 0..4 {
        let wt = manager
            .create(TaskId::default(), head.trim())
            .await
            .unwrap();
        for j in ancestors(&tasks, i) {
            manager
                .apply_dependency_patch(&wt, tasks[j].patch.as_deref().unwrap())
                .await
                .unwrap();
        }
        let base = index_tree(&wt).await.unwrap();
        std::fs::write(
            wt.path.join(format!("{}.txt", tasks[i].step.key)),
            format!("{} result\n", tasks[i].step.key),
        )
        .unwrap();
        index_tree(&wt).await.unwrap();
        let patch = git(&wt.path, &["diff", "--cached", "--binary", &base, "--"])
            .await
            .unwrap();
        completed(&mut tasks[i]);
        tasks[i].patch = Some(patch);
    }
    let final_wt = manager
        .create(TaskId::default(), head.trim())
        .await
        .unwrap();
    for task in &tasks {
        manager
            .apply_dependency_patch(&final_wt, task.patch.as_deref().unwrap())
            .await
            .unwrap();
    }
    for key in ["a", "b", "c", "d"] {
        assert_eq!(
            std::fs::read_to_string(final_wt.path.join(format!("{key}.txt"))).unwrap(),
            format!("{key} result\n")
        );
    }
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "base\n"
    );
}

#[test]
fn review_rework_preserves_sessions_requeues_dependents_and_rejects_stale_versions() {
    use crate::manifest::{ReviewChange, ReviewManifest, TaskManifest, Verdict};
    let mut w = workflow(vec![
        task(step("a", &["a"], &[])),
        task(step("b", &["b"], &["a"])),
        task(step("c", &["c"], &[])),
    ]);
    for s in &mut w.steps {
        completed(s);
        s.child_id = Some(HubThreadId::default());
    }
    w.thread.status = "awaiting_review".into();
    w.artifact_version = Some("a".repeat(64));
    w.manifest = Some(TaskManifest {
        version: 1,
        manifest_id: "task-one".into(),
        project_id: w.thread.project_id.to_string(),
        base_revision: w.source_head.clone(),
        request: "change".into(),
        acceptance: vec!["verified".into()],
        scope: vec!["a".into(), "b".into(), "c".into()],
        steps: w.steps.iter().map(|s| s.step.clone()).collect(),
    });
    let review = ReviewManifest {
        version: 1,
        manifest_id: "review-one".into(),
        project_id: w.thread.project_id.to_string(),
        task_id: w.thread.id.to_string(),
        artifact_version: "a".repeat(64),
        verdict: Verdict::Fail,
        summary: "fix a".into(),
        findings: vec!["a is incorrect".into()],
        changes: vec![ReviewChange {
            step_key: "a".into(),
            instruction: "correct a".into(),
        }],
    };
    let before = w.clone();
    let mut bad = review.clone();
    bad.artifact_version = "b".repeat(64);
    assert!(prepare_review(&mut w, &bad).is_err());
    bad = review.clone();
    bad.task_id = HubThreadId::default().to_string();
    assert!(prepare_review(&mut w, &bad).is_err());
    bad = review.clone();
    bad.changes[0].step_key = "unknown".into();
    assert!(prepare_review(&mut w, &bad).is_err());
    assert!(prepare_review(&mut w, &review).unwrap());
    assert_eq!(w.history.len(), 1);
    assert_eq!(w.thread.status, "queued");
    assert!(w.artifact_version.is_none());
    for i in [0, 1] {
        assert_eq!(w.steps[i].status, "pending");
        assert_eq!(w.steps[i].child_id, before.steps[i].child_id);
        assert_ne!(w.steps[i].task_id, before.steps[i].task_id);
        assert!(w.steps[i].verification.is_empty());
    }
    assert_eq!(w.steps[2].status, "completed");
    assert!(prepare_review(&mut w, &review).is_err());
    let mut stopped = before.clone();
    stopped.thread.status = "interrupted".into();
    stopped.steps[0].status = "interrupted".into();
    stopped.steps[1].status = "blocked".into();
    prepare_resume(&mut stopped).unwrap();
    assert_eq!(stopped.steps[0].child_id, before.steps[0].child_id);
    assert_eq!(stopped.steps[0].status, "pending");
    assert_eq!(stopped.steps[2].status, "completed");
    assert!(prepare_resume(&mut stopped).is_err());
    let mut pass = review;
    pass.verdict = Verdict::Pass;
    pass.changes.clear();
    let mut reviewed = before;
    assert!(!prepare_review(&mut reviewed, &pass).unwrap());
    assert_eq!(reviewed.thread.status, "completed");
    reviewed.steps[0].verification = vec![json!({"status":"not_run","success":false})];
    assert!(prepare_review(&mut reviewed, &pass).is_err());
}

#[test]
fn manifest_receipt_is_atomic_and_survives_restart() {
    let d = tempfile::tempdir().unwrap();
    let db = d.path().join("hub.db");
    std::process::Command::new("git")
        .arg("init")
        .arg(d.path())
        .output()
        .unwrap();
    let mut w = workflow(vec![task(step("a", &["a"], &[]))]);
    {
        let mut store = hub_db::Store::open(&db).unwrap();
        let p = store.register_project(d.path()).unwrap();
        w.thread.project_id = p.id;
        store
            .accept_manifest(
                "one",
                "original",
                &w.thread,
                &key(w.thread.id),
                "snapshot-one",
            )
            .unwrap();
    }
    let mut store = hub_db::Store::open(&db).unwrap();
    assert!(store
        .accept_manifest(
            "one",
            "replacement",
            &w.thread,
            &key(w.thread.id),
            "changed"
        )
        .is_err());
    assert_eq!(
        store.setting(&key(w.thread.id)).unwrap().as_deref(),
        Some("snapshot-one")
    );
    assert_eq!(store.threads().unwrap().len(), 1);
    let mut invalid = w.thread.clone();
    invalid.project_id = ProjectId::default();
    invalid.id = HubThreadId::default();
    assert!(store
        .accept_manifest("two", "bad", &invalid, "bad", "bad")
        .is_err());
    assert!(store.setting("manifest_receipt:two").unwrap().is_none());
}

#[test]
fn duplicate_manifest_reopens_deleted_mapping_without_replaying_or_replacing_state() {
    let d = tempfile::tempdir().unwrap();
    std::process::Command::new("git")
        .arg("init")
        .arg(d.path())
        .output()
        .unwrap();
    let mut store = hub_db::Store::open(&d.path().join("hub.db")).unwrap();
    let project = store.register_project(d.path()).unwrap();
    let mut w = workflow(vec![task(step("a", &["a"], &[]))]);
    w.thread.project_id = project.id;
    w.thread.status = "interrupted".into();
    w.stop_requested = true;
    let saved = serde_json::to_string(&w).unwrap();
    let receipt =
        json!({"thread_id": w.thread.id, "manifest": {"manifest_id":"reopen"}}).to_string();
    store
        .accept_manifest("reopen", &receipt, &w.thread, &key(w.thread.id), &saved)
        .unwrap();
    store.delete_thread(w.thread.id).unwrap();
    assert!(store.threads().unwrap().is_empty());
    assert!(
        crate::manifest::existing_task(&store, "reopen", &ProjectId::default().to_string())
            .is_err()
    );
    assert!(store.threads().unwrap().is_empty());
    let restored = crate::manifest::existing_task(&store, "reopen", &project.id.to_string())
        .unwrap()
        .unwrap();
    assert_eq!(restored.id, w.thread.id);
    assert_eq!(restored.status, "interrupted");
    assert_eq!(store.threads().unwrap().len(), 1);
    assert_eq!(store.setting(&key(w.thread.id)).unwrap().unwrap(), saved);
    assert_eq!(
        store.setting("manifest_receipt:reopen").unwrap().unwrap(),
        receipt
    );
    assert!(
        crate::manifest::existing_task(&store, "unknown", &project.id.to_string())
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn progress_diff_uses_worker_then_integrated_tree_including_new_files() {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init"]).await.unwrap();
    std::fs::write(dir.path().join("base.txt"), "original\n").unwrap();
    git(dir.path(), &["add", "base.txt"]).await.unwrap();
    git(
        dir.path(),
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@localhost",
            "commit",
            "-m",
            "fixture",
        ],
    )
    .await
    .unwrap();
    let head = git(dir.path(), &["rev-parse", "HEAD"]).await.unwrap();
    let manager = GitWorktrees::new(dir.path()).unwrap();
    let tree = manager
        .create(TaskId::default(), head.trim())
        .await
        .unwrap();
    std::fs::write(tree.path.join("new.txt"), "new worker content\n").unwrap();
    let mut w = workflow(vec![task(step("worker", &["new.txt"], &[]))]);
    w.source_root = dir.path().to_owned();
    w.steps[0].worktree = Some(tree.clone());
    let changes = worktree_changes(&w).await.unwrap();
    assert_eq!(changes[0].key, "worker");
    assert!(changes[0]
        .diff
        .as_ref()
        .unwrap()
        .contains("+new worker content"));
    assert!(!dir.path().join("new.txt").exists());
    w.artifact = Some(tree.clone());
    assert_eq!(worktree_changes(&w).await.unwrap()[0].key, "integrated");
    w.artifact.as_mut().unwrap().path = dir.path().join("missing");
    let changes = worktree_changes(&w).await.unwrap();
    assert!(changes[0].error.is_some());
    assert!(changes[0].diff.is_none());
}

#[test]
fn progress_history_filters_other_turns_and_stream_placeholders() {
    let dir = tempfile::tempdir().unwrap();
    assert!(std::process::Command::new("git")
        .args(["init", "-q"])
        .arg(dir.path())
        .status()
        .unwrap()
        .success());
    let store = hub_db::Store::open(&dir.path().join("hub.db")).unwrap();
    let project = store.register_project(dir.path()).unwrap();
    let id = HubThreadId::default();
    store
        .save_thread(&ThreadMapping {
            id,
            project_id: project.id,
            provider: "codex".into(),
            provider_thread_id: "activity-fixture".into(),
            title: "worker".into(),
            status: "running".into(),
        })
        .unwrap();
    for (kind, turn, text) in [
        ("message_completed", "old", "old result"),
        ("message_delta", "current", "omitted"),
        ("item_started", "current", "inspect"),
        ("message_completed", "current", "editing file"),
    ] {
        store
            .append_event(
                Some("activity-fixture"),
                kind,
                &json!({"thread_id":"activity-fixture","turn_id":turn,"kind":kind,"text":text})
                    .to_string(),
            )
            .unwrap();
    }
    let events = store.recent_activity(id, Some("current")).unwrap();
    assert_eq!(events.len(), 2);
    assert!(events[0].1.contains("inspect"));
    assert!(events[1].1.contains("editing file"));
}

#[test]
fn context_measurements_are_explicit_estimates_and_soft_targets() {
    let metrics = context_metrics("あ".repeat(100).as_str(), "ああああ", 1, CAPSULE_LIMIT);
    assert_eq!(metrics["after_bytes"], 12);
    assert_eq!(metrics["after_estimated_tokens"], 2);
    assert_eq!(metrics["over_target"], true);
    assert!(bounded("x".repeat(49 * 1024), CAPSULE_LIMIT).is_ok());
    assert!(metrics["provider_context_limit"].is_null());
    let evidence =
        compact_evidence(&json!({"status":"failed","exit_code":1,"output":"x".repeat(3000)}));
    assert_eq!(evidence["status"], "failed");
    assert_eq!(evidence["exit_code"], 1);
    assert_eq!(evidence["output_excerpted"], true);
}
