# Context model

A worker capsule contains a goal, acceptance criteria, constraints, and selected source items. There is no implicit parent-conversation field. Any selected conversation excerpt must be a labeled item. Source references and estimated tokens are inspectable; estimates must not be reported as measured provider usage.

Budgets are configurable: Spark 2K initial / 8K maximum; Codex normal 6K / 24K; Codex deep 12K / 64K. Explicit retrieval consumes both the per-request and cumulative allowance. Exhaustion must return a visible error rather than truncating unobservably.

Benchmark output must distinguish synthetic capsule-size checks from paired live runs with comparable task success, latency, and provider input/cached/output tokens. No percentage claim is justified yet.
