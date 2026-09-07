# Implementation status — 2026-09-08 JST

Spark uses **8bit**, per the user's amendment. This document separates implemented features from real-provider and native UI evidence.

| Milestone | Evidence |
|---|---|
| A: foundation | Rust/Tauri/SQLite/project registry, native macOS bundle, TypeScript build, tests and clippy passed. CI configured; no remote CI run claimed. |
| B: Codex adapter | Codex 0.153.4 handshake and real fixture edit passed; 54 normalized events; same provider thread resumed after app-server restart. Lifecycle/race fixtures passed. |
| C: desktop lifecycle | Native UI project registration, streamed edit, steering, interruption, restart/resume and diff verified. |
| D: Spark | Pinned Spark MLX runtime and full 8bit weights loaded on Metal GPU. Real routing, edit, review and summary passed; native UI changed fixture through Spark with token/latency records. Spark-unavailable Auto fallback tested. |
| E: orchestration | Two parallel real Codex workers plus dependent integration worker completed in separate worktrees; independent 18-test acceptance passed. Pull retrieval, budgets and paths tested. Native Plan dispatch and Context Inspector verified (195 initial + 36 retrieved / 4000 budget). Paired context benchmark passed: both modes 9/9, see report. |
| F: Astra | Real structured 3-step plan, known-defect review and repeated-failure diagnosis passed. Native review detected an injected fixture defect; rework returned to the same thread/worktree and restored exact expected content. Exact-goal/diff final review approved the retained full workflow (8 fixed + 13 worker tests). |
| G: durable memory | OrgBrain MCP adapter, scoped retrieval, Spark extraction, provenance, cross-session fixture persistence and retrieval passed. Automatic storage requires Astra approval of the exact goal/diff and is disabled by default. No real OrgBrain account writes made. |

## Evidence and limits

- [Measured context comparison](CONTEXT_BENCHMARK.md) and [raw metrics](evidence/context-benchmark.json).
- Original live fixture: `/var/folders/lm/s5hgvkpd7451pbw9pj889h780000gn/T/astra-hub-fixture-1788790212629`.
- Parallel integration fixture: `/var/folders/lm/s5hgvkpd7451pbw9pj889h780000gn/T/astra-workers-fixture-LmZHcr`.
- Planner fixture: `/var/folders/lm/s5hgvkpd7451pbw9pj889h780000gn/T/astra-planner-fixture-biNXpf`.
- Provider raw delta bodies are omitted from durable journals; completed items are sanitized and retained. Usage is measured provider data when present; missing values stay unknown. Capsule estimates are separate from provider overhead.
- Astra defaults to disabled. Codex-integrated access is implemented and checked against the advertised model catalog. Direct ChatGPT and direct API adapters are optional/unconfigured, and are not silently substituted or billed.
- Provider approval/input requests currently fail visibly rather than being automatically granted. Broad approval UI is not implemented.
- No automatic branch merge, production deployment, App Store submission, or remote CI claim.
- `config.example.toml` documents concepts; active desktop settings are managed in the application's settings UI and SQLite. It is not a loaded TOML configuration file.

## Restart behavior

Provider threads, worktrees, capsules, retrieval budgets, usage and plan dispatch state are persisted. The Plan tab offers **保存済みの計画を再開**: it validates completed patch snapshots, preserves completed task IDs, reconstructs the dependency graph, and resumes interrupted workers on their original thread/worktree. An active provider turn is observed; an idle interrupted thread gets a continuation request. A worktree created before its thread was durably bound requires manual inspection instead of duplicate dispatch. Failed/cancelled tasks are not automatically retried. The new DAG recovery is covered by deterministic recovery tests; earlier live restart evidence covers provider/thread recovery, not a simulated process crash of the entire DAG.

## Full workflow fixture

[Full workflow evidence](evidence/full-workflow.json) records Astra planning, two parallel Codex workers, a dependent integration worker, three durable-memory pulls, fixed acceptance, worker tests and scoped Astra approval. Spark ran local context drafting and completion extraction; the separate Spark fixture/native edit establishes the small-edit lane. OrgBrain transport in the full workflow was a file-backed loopback MCP server, not a connected customer account.

Reproduce a new full fixture with `ASTRA_LIVE_FIXTURE=1 cargo run -p hub-planner --example full_fixture` while the local Spark service is running. This makes real Codex/Astra calls and runs fixture code. Finish a retained completed fixture with `ASTRA_LIVE_FIXTURE=1 ASTRA_REUSE_FIXTURE=/absolute/temp/astra-full-fixture-… cargo run -p hub-planner --example finish_fixture`; held inputs and existing review evidence are checked and reused. Never treat an old approval as approval of a changed goal or patch.

## Final local checks

Rust workspace: 31 tests passed; subsequent runtime recovery checks: 7 passed. Workspace clippy with warnings denied passed. TypeScript/Vite and the macOS debug `.app` bundle built successfully. Spark service tests: 2 passed. Latest post-memory/recovery native visual confirmation is pending because the Mac was locked; native lifecycle, Plan/Context and Astra review/rework were verified on earlier builds.
