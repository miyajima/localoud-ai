# Architecture

The Tauri IPC boundary exposes Hub domain objects, never provider JSON. `hub-core` owns typed IDs and contracts; `hub-db` owns the SQLite schema, canonical project registry, and provider mappings. Provider IDs are attributes of Hub threads, never primary Hub identifiers.

Each provider adapter owns its wire protocol. A per-thread actor serializes turn mutations and persists intent before dispatch. The event journal precedes UI broadcast so reconnect can replay durable records. Concurrent workers receive independent threads and controlled worktrees.

Implementation proceeds through the acceptance gates in `ASTRA_CODEX_HUB_PLAN.md`. The desktop exposes project/thread lifecycle, Spark local tasks, Astra planning/review, DAG execution/recovery, diff/terminal evidence, context budgets, memory provenance and usage. `hub-runtime` owns worker lifecycle; `hub-scheduler` owns dependencies; `hub-context` bounds retrieval. `hub-memory` owns the optional OrgBrain MCP boundary.
