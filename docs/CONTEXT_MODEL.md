# Context model

A worker capsule contains a goal, acceptance criteria, constraints, and selected source items. There is no implicit parent-conversation field. Any selected conversation excerpt must be a labeled item. Source references and estimated tokens are inspectable; estimates must not be reported as measured provider usage.

When an existing API-provider session moves to a fresh Codex thread, Localoud no longer replays the normalized visible transcript wholesale. The configured local model proposes exact quote spans, section labels, dependencies and corrections. The existing deterministic handoff validator then re-derives speaker and tool provenance, checks every quote byte-for-byte against the stored visible message, rejects confidence below 0.75, verifies correction chronology and mandatory sections, and packs the result within the capsule budget. Any extraction or validation failure stops before `thread/start`; it never falls back to the full transcript.

Budgets are configurable: Spark 2K initial / 8K maximum; Codex normal 6K / 24K; Codex deep 12K / 64K. Explicit retrieval consumes both the per-request and cumulative allowance. Exhaustion must return a visible error rather than truncating unobservably.

Benchmark output must distinguish synthetic capsule-size checks from paired live runs with comparable task success, latency, and provider input/cached/output tokens. No percentage claim is justified yet.

## Manifest workflow handoff measurements

The desktop Manifest path uses independent safety limits: plan input 256 KiB,
worker input 256 KiB, review 512 KiB. Provider limits are not inferred from these.
Soft initial targets are 16,000 estimated tokens for workers and 32,000 for review.
Exceeding a target is recorded, not rejected. The reproducible estimate is
ceil(Unicode scalar count / 3), not a tokenizer measurement. System prompts,
tools, history, output reserve and subsequent reads are outside its scope.
Existing hub-context retrieval budgets above are a separate execution path.

Dependency artifacts are applied to the worker worktree. Capsules carry artifact
paths, outcomes, status counts and non-passing evidence. Command output excerpts
are capped at 2,000 characters and explicitly marked. Review receives command
metadata and these excerpts; absent acceptance evidence requires inconclusive,
not inferred success. Full verification records remain in Localoud history.

New payloads persist context_metrics:<workflow>:<iteration>:<step>, including
strategy version, before/after UTF-8 bytes, estimated tokens, target and timestamp.
Before reconstructs the same envelope with unabridged verification; it is a
counterfactual size measurement, not another model call. Codex metrics include
the injected working directory. Payload reduction is not measured runtime savings.

Export JSON read-only:
python3 scripts/context-metrics.py --workflow <workflow-id>

The export joins attempt status, model target, worker provider usage snapshots
and review. Missing measurements remain missing. Never sum cumulative snapshots.
Final-review provider usage is not joined and remains in model-usage records.
Compare matched inputs/models and accepted outcomes before claiming token,
latency or quality gains. No experiments or replays run automatically.
