# Implementation status — 2026-09-08 JST

Spark uses **8bit**, per the user's amendment. This document separates implemented features from real-provider and native UI evidence.

| Milestone | Evidence |
|---|---|
| A: foundation | Rust/Tauri/SQLite/project registry, native macOS bundle, TypeScript build, tests and clippy passed. CI configured; no remote CI run claimed. |
| B: Codex adapter | Codex 0.153.4 handshake and real fixture edit passed; 54 normalized events; same provider thread resumed after app-server restart. Lifecycle/race fixtures passed. |
| C: desktop lifecycle | Native UI project registration, streamed edit, steering, interruption, restart/resume and diff verified. |
| D: Spark | Pinned Spark MLX runtime and full 8bit weights loaded on Metal GPU. Real routing, edit, review and summary passed; native UI changed fixture through Spark with token/latency records. Legacy Spark-unavailable routing fallback tested; the current configurable Auto behavior is documented below. |
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

Rust workspace: 31 tests passed; subsequent runtime recovery checks: 7 passed. Workspace clippy with warnings denied passed. TypeScript/Vite and the macOS debug `.app` bundle built successfully. Spark service tests: 2 passed. Latest native visual confirmation completed after unlocking the Mac: Plan/restart controls, restored worker history and Context ledger (195 initial + 71 retrieved / 4000), and the OrgBrain settings dialog were verified. An old plan without a runtime envelope returned a visible no-recoverable-plan error without dispatch. Fixed clipped dialog actions and viewport overflow, rebuilt the macOS bundle, and verified all dialog fields/actions fit. Auto-storage remained unchecked; no credentials were entered or saved. Live crash recovery of an active whole DAG remains distinct from these UI checks.

## Configurable Auto and Reasoning

[Auto settings](AUTO_ROUTING.md) support one classifier, five execution model/Reasoning assignments, explicit stop-or-configured-fallback behavior, and optional preview with manual adjustment. Risk/planning and local edit limits remain separate from difficulty. Settings and per-thread explicit effort persist; pending routes validate their input, settings revision and one-time consumption. Codex reasoning choices come from the live catalog, with `model` and `effort` sent on each new turn, including after resume.

Latest checks: 42 Rust tests, 9 frontend tests, 4 Python service tests passed; workspace clippy with warnings denied passed; TypeScript/Vite and native macOS debug bundle built. Native Spark 8bit test classified a typo as level 1, showed the configured cloud assignment after changing to level 2, returned to level 1 without reclassification, and applied the exact expected local edit only after confirmation. Settings save/reopen and live cloud Reasoning options were checked. Actual cloud effort payload and durable resume are covered by deterministic wire fixtures; no live all-model performance or reasoning-quality comparison is claimed.

## Desktop UX follow-up

[UX comparison and verification](UX_COMPARISON.md): native project/file chooser, keyboard commands, search/pin/rename, local drafts/workspace restoration, sanitized Markdown, diff colors/copy, plan preview and interaction feedback are implemented. Four frontend safety/rendering tests and three hub-db tests passed; workspace clippy and the macOS bundle build passed. Native UI checks include registration/cancel, search, rename/pin, restart/draft restoration and final layout. Codex Desktop's comparison baseline is official documentation; its UI could not be controlled under this environment's Computer Use policy.
