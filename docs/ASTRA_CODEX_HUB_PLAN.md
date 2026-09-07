# ASTRA_CODEX_HUB_PLAN.md

## 0. Purpose

Build a local-first desktop coding workspace that preserves the familiar Codex-style UX while replacing Codex Subagent V2 context inheritance with an external orchestration layer.

Primary goals:

1. Use **Tauri** for the desktop UI.
2. Use a **Rust Agent Hub** as the deterministic control plane.
3. Use **Codex app-server** as the main execution/runtime engine for non-trivial implementation work.
4. Run **Spark-X2.5-4B MLX 8bit** locally on this Mac as:
   - task router,
   - context drafter,
   - lightweight implementation worker,
   - progress summarizer,
   - first-pass reviewer,
   - retrieval-query generator.
5. Use **GPT-6 Astra** for:
   - high-level planning,
   - architecture decisions,
   - ambiguous/high-risk decisions,
   - final review,
   - replanning after repeated failures.
6. Use **OrgBrain** as durable organizational memory.
7. Do **not** use Codex Subagent V2 as the default multi-agent mechanism.
8. Do **not** inherit the parent thread context wholesale when spawning workers.
9. Use **selective Context Capsules + pull-based context retrieval** instead.
10. Isolate concurrent implementation tasks with **Git worktrees**.

---

# 1. Non-goals

Do not attempt these in V1:

- Pixel-perfect clone of the proprietary Codex Desktop UI.
- Reimplementation of Codex sandboxing or command execution.
- Replacement of Codex app-server.
- Dependence on undocumented Codex Desktop internals.
- Codex Subagent V2 orchestration as the primary worker system.
- Automatic merging directly into protected branches.
- Full OrgBrain integration before the basic agent lifecycle works.
- Perfect semantic routing in the first version.
- Embedding Spark directly inside the Hub process.

---

# 2. Core design principle

The most important invariant is:

> **Spawning an agent must not imply copying the parent context.**

Bad model:

```text
Parent thread: 80K tokens
        |
        +-- fork --> Worker A: ~80K
        +-- fork --> Worker B: ~80K
        +-- fork --> Worker C: ~80K
```

Target model:

```text
Parent thread: 80K
        |
        v
Context Drafter
        |
        +-- Worker A capsule: 4K
        +-- Worker B capsule: 7K
        +-- Worker C capsule: 3K
```

Workers may retrieve additional context on demand.

---

# 3. High-level architecture

```text
+------------------------------------------------------+
|                  Tauri Desktop UI                    |
|                                                      |
| Projects / Threads / Agents / Diff / Terminal        |
| Plan / Review / Context Inspector / Cost & Tokens    |
+---------------------------+--------------------------+
                            |
                            v
+------------------------------------------------------+
|                 Agent Hub / Rust Core                |
|                                                      |
| Scheduler              Context Manager               |
| Router                 Session Manager               |
| Event Bus              Policy Engine                 |
| Worktree Manager       Token/Cost Accounting         |
| Task DAG               Escalation Engine             |
+---------+--------------------+----------------+-------+
          |                    |                |
          v                    v                v
 +----------------+   +----------------+  +----------------+
 | Spark Provider |   | Codex Adapter  |  | Astra Adapter  |
 | MLX / local    |   | app-server     |  | remote         |
 +----------------+   +--------+-------+  +----------------+
                               |
                        +------+------+
                        |      |      |
                        v      v      v
                     Worker  Worker  Worker
                     Thread  Thread  Thread

                            |
                            v
                    +---------------+
                    |   OrgBrain    |
                    | durable mem   |
                    +---------------+
```

---

# 4. Responsibilities by layer

## 4.1 Rust Hub

The Hub is the source of truth for:

- project registry,
- task graph,
- worker lifecycle,
- routing policy,
- task state,
- worktree ownership,
- context budgets,
- context capsules,
- context retrieval history,
- escalation state,
- model usage,
- approvals,
- Codex thread mappings,
- event journaling.

The Hub must remain deterministic wherever possible.

Do not delegate state correctness to an LLM.

---

## 4.2 Spark-X2.5-4B MLX

Spark is the local low-cost "nervous system".

Responsibilities:

### Router
Classify requests/tasks into:
- `spark`
- `codex`
- `astra_plan`
- `human_required`

### Context Drafter
Create minimal worker context from:
- task request,
- current decisions,
- relevant files,
- relevant symbols,
- unresolved issues,
- acceptance criteria,
- previous failures.

### Lightweight implementer
Use Spark for:
- typo fixes,
- renames,
- config changes,
- small UI changes,
- simple tests,
- boilerplate,
- low-risk single-file refactors,
- low-risk changes estimated below configurable LOC thresholds.

### Progress summarizer
Compress Codex event streams into compact progress state.

### First-pass verifier
Inspect:
- diff,
- tests,
- obvious requirement violations,
- scope drift,
- trivial bugs.

### Retrieval-query generator
When a worker requests context, convert that request into a concise search query for:
- OrgBrain,
- local session store,
- repo index,
- decision store.

---

## 4.3 Codex

Codex is the primary execution engine.

Use it for:

- repository exploration,
- multi-file implementation,
- non-trivial debugging,
- integration work,
- migrations,
- test implementation,
- build/test/fix loops,
- normal coding tasks.

Use Codex app-server as the integration surface.

The Hub must wrap Codex protocol details behind an adapter.

Do not expose Codex wire types directly to:
- UI,
- scheduler,
- task store,
- routing layer.

---

## 4.4 Astra

Astra is the intelligence ceiling, not the default executor.

Responsibilities:

- high-level planning,
- architecture,
- ambiguous requirements,
- security-sensitive reasoning,
- large refactors,
- task decomposition,
- final review,
- conflict resolution,
- replanning after repeated worker failure.

Astra should return structured plans and review results.

Astra must be behind an adapter so the backend can support:

1. ChatGPT/Codex-plan-integrated Astra usage when practical.
2. Direct OpenAI API Astra usage when configured.
3. A fallback planner model.

Do not make the Hub depend on a single Astra access mechanism.

---

## 4.5 OrgBrain

OrgBrain stores durable knowledge, not raw chat history by default.

Persist:

- decisions,
- reasons,
- constraints,
- rejected approaches,
- failures,
- warnings,
- unresolved issues,
- successful implementation patterns,
- next actions.

Do not blindly push every log or model message into OrgBrain.

---

# 5. Repository structure

Recommended monorepo:

```text
astra-codex-hub/
├── Cargo.toml
├── README.md
├── docs/
│   ├── ARCHITECTURE.md
│   ├── PROTOCOLS.md
│   ├── CONTEXT_MODEL.md
│   └── SECURITY.md
│
├── apps/
│   └── desktop/
│       ├── src/
│       ├── src-tauri/
│       └── package.json
│
├── crates/
│   ├── hub-core/
│   ├── hub-db/
│   ├── hub-events/
│   ├── hub-router/
│   ├── hub-context/
│   ├── hub-worktree/
│   ├── hub-scheduler/
│   ├── hub-policy/
│   ├── provider-spark/
│   ├── provider-codex/
│   ├── provider-astra/
│   ├── provider-orgbrain/
│   └── protocol-types/
│
├── services/
│   └── spark-mlx/
│       ├── server.py
│       ├── config.yaml
│       └── README.md
│
├── migrations/
│
├── fixtures/
│   ├── codex-events/
│   ├── routing/
│   └── context/
│
└── tests/
    ├── integration/
    └── e2e/
```

---

# 6. Core Rust domain model

## 6.1 IDs

Use strong typed IDs.

```rust
pub struct ProjectId(Uuid);
pub struct PlanId(Uuid);
pub struct TaskId(Uuid);
pub struct WorkerId(Uuid);
pub struct ContextCapsuleId(Uuid);
pub struct HubThreadId(Uuid);
```

Codex IDs remain provider-specific strings and must not become primary database IDs.

---

## 6.2 Task

```rust
pub struct Task {
    pub id: TaskId,
    pub project_id: ProjectId,
    pub plan_id: Option<PlanId>,
    pub parent_task_id: Option<TaskId>,

    pub title: String,
    pub description: String,

    pub dependencies: Vec<TaskId>,

    pub preferred_executor: ExecutorPreference,
    pub assigned_executor: Option<ExecutorKind>,

    pub risk: RiskLevel,
    pub complexity: Complexity,

    pub status: TaskStatus,

    pub context_capsule_id: Option<ContextCapsuleId>,
    pub worktree_id: Option<WorktreeId>,

    pub attempts: u32,
    pub max_attempts: u32,
}
```

---

## 6.3 ExecutorKind

```rust
pub enum ExecutorKind {
    Spark,
    Codex,
    Astra,
}
```

Astra is normally planner/reviewer but leave it representable for exceptional execution.

---

# 7. Context Capsule

This is a first-class persisted object.

Example:

```yaml
task_id: task_01
project_id: orgbrain

goal: >
  Implement OAuth refresh-token rotation.

acceptance_criteria:
  - Refresh token is single-use.
  - Existing sessions remain valid.
  - Migration is reversible.

constraints:
  - Do not change the public API.
  - Preserve backwards compatibility.

decisions:
  - id: dec_183
    statement: Use rotating refresh tokens.
    reason: Reduces replay risk.

rejected:
  - approach: JWT-only revocation.
    reason: Existing architecture requires server-side session invalidation.

relevant_files:
  - src/auth/token.rs
  - src/db/schema.rs

relevant_symbols:
  - TokenService::rotate
  - SessionRepository

known_failures: []

unresolved_questions: []

repo_state:
  base_sha: abc123
  worktree: .agent-worktrees/task-01

context_budget:
  initial_tokens: 8000
  max_total_tokens: 24000
  max_single_retrieval_tokens: 4000
```

---

# 8. Context Broker

Implement:

```rust
#[async_trait]
pub trait ContextBroker {
    async fn build_initial_capsule(
        &self,
        task: &Task,
        project: &Project,
    ) -> Result<ContextCapsule>;

    async fn retrieve(
        &self,
        request: ContextRequest,
    ) -> Result<ContextRetrievalResult>;
}
```

Context sources:

```rust
pub enum ContextSource {
    Repo,
    SessionStore,
    OrgBrain,
    DecisionStore,
    TaskHistory,
}
```

Rules:

1. Initial context must be minimal.
2. No full parent-conversation inheritance.
3. Retrieval must be explicit and logged.
4. Every retrieval consumes a context budget.
5. Context Inspector UI must show all retrieved sources.
6. Parent thread text should be included only if selected by the drafter.

---

# 9. Spark provider abstraction

```rust
#[async_trait]
pub trait LocalModelProvider {
    async fn classify_task(
        &self,
        request: &RoutingInput,
    ) -> Result<RoutingDecision>;

    async fn draft_context(
        &self,
        request: &ContextDraftRequest,
    ) -> Result<ContextCapsuleDraft>;

    async fn summarize_progress(
        &self,
        events: &[HubEvent],
    ) -> Result<ProgressSummary>;

    async fn review_diff(
        &self,
        request: &ReviewInput,
    ) -> Result<ReviewResult>;

    async fn generate_retrieval_query(
        &self,
        request: &ContextNeed,
    ) -> Result<RetrievalQuery>;
}
```

Do not hardcode Spark-specific assumptions into Hub logic.

---

# 10. Spark MLX service

Run Spark as a separate local process.

Target:

```text
127.0.0.1:<configurable-port>
```

Suggested API:

## Health

```http
GET /health
```

## Structured generation

```http
POST /v1/generate
```

Request:

```json
{
  "task": "route",
  "messages": [],
  "schema": {},
  "max_tokens": 512,
  "temperature": 0.1
}
```

Response:

```json
{
  "output": {},
  "usage": {
    "prompt_tokens": 0,
    "completion_tokens": 0
  },
  "latency_ms": 0
}
```

The service must support schema-constrained structured output where practical.

---

# 11. Routing

## 11.1 Deterministic pre-router

Before Spark, check obvious rules.

Examples:

### Send directly to Spark

- explicit typo fix,
- known file and trivial change,
- formatting-only,
- rename with bounded scope,
- config value adjustment.

### Send directly to Astra planning

- architecture,
- ambiguous cross-cutting request,
- security-sensitive change,
- large migration,
- request spanning many modules,
- explicit user request for design before implementation.

Everything else may go through Spark classification.

---

## 11.2 Spark routing output

```json
{
  "executor": "codex",
  "complexity": "normal",
  "risk": "medium",
  "needs_plan": false,
  "needs_final_astra_review": false,
  "estimated_scope": {
    "files": 4,
    "loc": 180
  },
  "confidence": 0.83,
  "reason": "multi-file change requiring repository exploration"
}
```

---

## 11.3 Default routing policy

### Spark

Prefer Spark when:

- estimated scope <= 1-2 files,
- estimated LOC < 100,
- no schema/API/security changes,
- acceptance criteria are explicit,
- tests are straightforward,
- confidence >= configurable threshold.

### Codex

Prefer Codex when:

- multi-file,
- repo exploration needed,
- debugging required,
- migration,
- integration,
- implementation + test loop,
- Spark confidence is low.

### Astra first

Use Astra before execution when:

- architecture decision,
- ambiguous requirements,
- security implications,
- large refactor,
- more than 3 major modules,
- task decomposition required,
- Codex has failed repeatedly.

---

# 12. Codex adapter

Define:

```rust
#[async_trait]
pub trait CodingAgentProvider {
    async fn start_thread(
        &self,
        req: StartThreadRequest,
    ) -> Result<ProviderThread>;

    async fn resume_thread(
        &self,
        thread: &ProviderThread,
    ) -> Result<()>;

    async fn start_turn(
        &self,
        req: StartTurnRequest,
    ) -> Result<ProviderTurn>;

    async fn steer_turn(
        &self,
        req: SteerTurnRequest,
    ) -> Result<()>;

    async fn interrupt_turn(
        &self,
        req: InterruptTurnRequest,
    ) -> Result<()>;

    async fn read_thread(
        &self,
        thread: &ProviderThread,
    ) -> Result<ProviderThreadState>;

    async fn stream_events(
        &self,
        turn: &ProviderTurn,
    ) -> Result<EventStream>;
}
```

Codex-specific implementation lives only in `provider-codex`.

---

# 13. Codex app-server lifecycle

Expected lifecycle:

```text
Hub
 |
 | initialize
 v
Codex app-server
 |
 | initialized
 |
 | thread/start
 v
thread_id
 |
 | turn/start
 v
turn_id
 |
 +--> streamed events
 |
 | turn/completed
 v
Hub verification
```

Support:

- `thread/start`
- `thread/resume`
- `thread/read`
- `turn/start`
- `turn/steer`
- `turn/interrupt`

Support review APIs if stable, but do not make them the only review path.

Do not use `thread/fork` as the standard worker-spawn path.

Use new independent threads + Context Capsules.

---

# 14. Thread actor / race protection

Create one actor per Codex thread.

```rust
pub struct ThreadActor {
    pub thread: ProviderThread,
    pub state: ThreadState,
    pub active_turn: Option<ProviderTurn>,
    pub queue: VecDeque<ThreadCommand>,
}
```

Rules:

- Only the actor may issue state-changing commands for that thread.
- Serialize `turn/start`, `turn/steer`, and `turn/interrupt`.
- Do not rely on read-then-write atomicity in upstream APIs.
- Persist command intent before provider write where feasible.

---

# 15. Event bus

All external-provider events must normalize into Hub events.

```rust
pub enum HubEvent {
    TaskCreated,
    TaskStarted,
    WorkerAssigned,

    ThreadStarted,
    TurnStarted,

    AgentMessageDelta,
    CommandStarted,
    CommandOutput,
    FileChanged,
    TestProgress,

    ContextRequested,
    ContextRetrieved,

    ProgressUpdated,

    TurnCompleted,
    ReviewCompleted,

    TaskCompleted,
    TaskFailed,

    EscalationTriggered,
}
```

Persist important lifecycle events.

High-volume delta events may be sampled or compacted.

---

# 16. Progress summarization

Do not send the full Codex event stream to Astra.

Periodically aggregate raw events and let Spark produce:

```json
{
  "status": "running",
  "summary": [
    "OAuth middleware implemented",
    "7 files modified",
    "42/44 tests passing",
    "2 failures in refresh-token rotation"
  ],
  "blockers": [],
  "scope_drift": false
}
```

Store both:
- raw provider event journal,
- compact progress summaries.

---

# 17. Worktree manager

Every non-trivial concurrent task gets its own Git worktree.

Suggested layout:

```text
repo/
└── .agent-worktrees/
    ├── task-101/
    ├── task-102/
    └── task-103/
```

Define:

```rust
#[async_trait]
pub trait WorktreeManager {
    async fn create(&self, task: TaskId, base_ref: &str) -> Result<Worktree>;
    async fn status(&self, worktree: &Worktree) -> Result<WorktreeStatus>;
    async fn diff(&self, worktree: &Worktree) -> Result<String>;
    async fn remove(&self, worktree: &Worktree) -> Result<()>;
}
```

V1 should not auto-merge conflicting changes.

---

# 18. Task DAG

Astra plans should compile into a DAG.

Example:

```text
           Plan
            |
      +-----+------+
      |            |
      v            v
 DB migration   Token service
   Codex A        Codex B
      |            |
      +-----+------+
            |
            v
       Integration
          Spark
            |
            v
          Tests
         Codex C
            |
            v
       Astra review
```

Task state:

```rust
pub enum TaskStatus {
    Pending,
    Blocked,
    Ready,
    Running,
    Reviewing,
    Completed,
    Failed,
    Cancelled,
}
```

Scheduler may run independent tasks concurrently.

---

# 19. Astra adapter

Define:

```rust
#[async_trait]
pub trait PlannerReviewer {
    async fn create_plan(
        &self,
        req: PlanningRequest,
    ) -> Result<ExecutionPlan>;

    async fn review_result(
        &self,
        req: FinalReviewRequest,
    ) -> Result<FinalReview>;

    async fn diagnose_failure(
        &self,
        req: FailureDiagnosisRequest,
    ) -> Result<RecoveryPlan>;
}
```

Astra implementation must support structured output.

---

# 20. Astra access modes

Implement an access strategy enum:

```rust
pub enum AstraAccessMode {
    ChatGptIntegrated,
    CodexIntegrated,
    OpenAiApi,
    Disabled,
}
```

Important:

- ChatGPT/Codex plan entitlement is not assumed to be interchangeable with direct API billing.
- Keep API credentials and ChatGPT/Codex authenticated usage separate.
- The Hub must function if `OpenAiApi` Astra access is disabled.
- Prefer to reuse Codex/ChatGPT plan Astra availability where the supported integration surface permits.
- Direct Astra API remains an optional adapter path.

---

# 21. Escalation

Default escalation:

```text
Spark
  |
  +-- success --> verify --> done
  |
  +-- failure
        |
        +-- obvious correction --> Spark retry
        |
        +-- otherwise --> Codex

Codex
  |
  +-- success --> verify
  |
  +-- failure --> Codex retry
                    |
                    +-- repeated failure --> Astra diagnose/replan
```

Architecture-changing worker questions should escalate to Astra.

Low-risk scope corrections may be handled with Spark-generated steering.

---

# 22. Steering

Use same-turn steering when possible.

Example:

```text
Codex is editing migration
        |
        v
Spark monitor detects:
"public API modification"
        |
        v
Policy violation
        |
        v
turn/steer:
"Do not modify the public API.
Continue only with the migration scope."
```

Policy:

- low-risk steering: Spark may generate it,
- architecture-changing steering: Astra must decide,
- destructive changes: require user approval when policy says so.

---

# 23. Context budgets

Default starting budgets:

```text
Spark quick worker
  initial: 2K
  max:     8K

Codex normal worker
  initial: 6K
  max:    24K

Codex deep worker
  initial: 12K
  max:     64K

Astra reviewer
  evidence selected by Context Manager
```

These values must be configurable.

Track:

- initial tokens,
- retrieved tokens,
- current estimated context,
- provider input tokens,
- provider cached tokens when available,
- output tokens,
- estimated cost/credit usage when exposed.

---

# 24. SQLite schema

Initial tables:

```sql
projects
plans
tasks
task_dependencies
workers
provider_threads
turns
context_capsules
context_capsule_items
context_retrievals
decisions
events
worktrees
reviews
model_usage
settings
```

Key table notes:

## provider_threads

```text
id
hub_thread_id
provider
provider_thread_id
project_id
task_id
status
created_at
updated_at
```

## context_retrievals

```text
id
task_id
worker_id
source
query
token_budget
token_estimate
result_ref
created_at
```

## model_usage

```text
id
provider
model
task_id
turn_id
prompt_tokens
cached_tokens
completion_tokens
estimated_cost
latency_ms
created_at
```

---

# 25. OrgBrain integration

Implement behind:

```rust
#[async_trait]
pub trait DurableMemoryProvider {
    async fn search(
        &self,
        req: MemorySearchRequest,
    ) -> Result<Vec<MemoryItem>>;

    async fn store(
        &self,
        items: Vec<DurableMemoryItem>,
    ) -> Result<()>;
}
```

At task completion:

```text
Task result
   |
   v
Spark memory extractor
   |
   +-- ephemeral --> discard
   |
   +-- durable --> OrgBrain
```

Durable item types:

- decision,
- reason,
- rejected approach,
- failure,
- constraint,
- warning,
- result,
- unresolved issue,
- next action.

---

# 26. UI layout

Minimum desktop layout:

```text
+-----------------------------------------------------------+
| Projects | Main workspace                                 |
|----------|------------------------------------------------|
| OrgBrain | Conversation / Task                            |
| Saleside |                                                |
| Mailaura | Agent cards                                    |
|          | - Spark route                                  |
| Agents   | - Codex backend                               |
| backend  | - Codex tests                                 |
| tests    |                                                |
| schema   |                                                |
+----------+------------------------------------------------+
| Ask | Plan | Implement | Review | Stop                    |
+-----------------------------------------------------------+
```

Tabs:

- Chat
- Plan
- Diff
- Agents
- Terminal
- Context
- Usage

---

# 27. Context Inspector UI

This is a required differentiator.

Example:

```text
Worker: backend-2

Initial context       6,240
Retrieved             2,180
Current estimate      8,420
Budget               24,000

Sources
[x] task brief
[x] decision #182
[x] token.rs
[x] schema.rs
[ ] full parent conversation

Retrieval history
- auth decision rationale: 820 tokens
- previous migration failure: 410 tokens
```

The UI must make context duplication visible.

---

# 28. Executor override UI

Allow:

```text
Executor
(*) Auto
( ) Spark
( ) Codex
( ) Astra
```

For Auto, show:

```text
Selected: Codex
Reason:
multi-file change / repository exploration required
```

---

# 29. Security boundaries

- Codex keeps responsibility for its own sandbox/runtime behavior.
- Hub owns policy decisions above provider level.
- Never automatically grant broad filesystem access outside registered project roots.
- Worktrees must remain under controlled paths.
- Store secrets in platform keychain, not SQLite.
- Do not include secrets in Context Capsules.
- Redact tokens/API keys from event journals.
- Require confirmation for destructive operations according to configured policy.

---

# 30. Phase 1 — Codex replacement shell

Goal:

A usable Codex-style desktop client backed by Codex app-server.

Implement:

- [ ] Rust workspace.
- [ ] Tauri app shell.
- [ ] project registration.
- [ ] SQLite DB.
- [ ] Codex process lifecycle.
- [ ] Codex app-server adapter.
- [ ] initialize handshake.
- [ ] thread/start.
- [ ] thread/resume.
- [ ] thread/read.
- [ ] turn/start.
- [ ] event streaming.
- [ ] turn/steer.
- [ ] turn/interrupt.
- [ ] thread actor.
- [ ] event bus.
- [ ] basic conversation view.
- [ ] agent/tool event rows.
- [ ] diff tab.
- [ ] terminal/tool output view.
- [ ] persisted thread mapping.
- [ ] app restart + session recovery.

Acceptance criteria:

- User can register a local repo.
- User can start a Codex task.
- Output streams live.
- File/tool activity is visible.
- User can steer active work.
- User can stop work.
- User can quit/reopen app and resume the same mapped Codex thread.
- No proprietary Codex Desktop automation is required.

---

# 31. Phase 2 — Spark local layer

Goal:

Add Spark as local router and lightweight worker.

Implement:

- [ ] Spark MLX 8bit service.
- [ ] health endpoint.
- [ ] structured generation endpoint.
- [ ] Rust Spark provider.
- [ ] deterministic pre-router.
- [ ] Spark task router.
- [ ] routing confidence.
- [ ] executor override.
- [ ] simple-task Spark execution path.
- [ ] progress summarizer.
- [ ] first-pass diff reviewer.
- [ ] local token/latency metrics.

Acceptance criteria:

- Trivial tasks can complete without Codex.
- Normal tasks automatically route to Codex.
- Routing result and reason are visible.
- Codex event streams can be summarized locally.
- Spark failure does not prevent Codex fallback.

---

# 32. Phase 3 — External workers and context isolation

Goal:

Replace Codex Subagent V2 for normal multi-agent use.

Implement:

- [ ] Task DAG.
- [ ] scheduler.
- [ ] independent Codex worker threads.
- [ ] Git worktree manager.
- [ ] Context Capsule persistence.
- [ ] Spark Context Drafter.
- [ ] context budgets.
- [ ] Context Broker.
- [ ] pull-based retrieval.
- [ ] retrieval event logging.
- [ ] Context Inspector UI.
- [ ] parallel workers.
- [ ] dependency completion logic.
- [ ] no default `thread/fork` for worker spawning.

Acceptance criteria:

- Parent thread can exceed 50K tokens.
- New worker starts with a bounded capsule instead of inherited full context.
- UI proves parent conversation was not automatically copied.
- Multiple Codex workers can execute in isolated worktrees.
- Worker can request missing context.
- Retrieval stays within budget.
- Task DAG dependencies work.

---

# 33. Phase 4 — Astra planner/reviewer

Goal:

Add high-level planning and escalation.

Implement:

- [ ] PlannerReviewer trait.
- [ ] Astra adapter.
- [ ] structured plan schema.
- [ ] plan -> task DAG compiler.
- [ ] final review.
- [ ] repeated-failure diagnosis.
- [ ] replanning.
- [ ] architecture-question escalation.
- [ ] Astra access-mode configuration.
- [ ] provider fallback.

Acceptance criteria:

- Complex request can be planned before implementation.
- Plan compiles into runnable tasks.
- Independent tasks run concurrently.
- Final Astra review can approve or request rework.
- Requested rework goes back to the appropriate existing worker thread where sensible.
- Hub remains usable without direct Astra API credentials.

---

# 34. Phase 5 — OrgBrain

Goal:

Add durable organizational memory.

Implement:

- [ ] OrgBrain provider.
- [ ] memory search.
- [ ] completion-time memory extraction.
- [ ] decision/reason retrieval.
- [ ] rejected-approach retrieval.
- [ ] cross-session context retrieval.
- [ ] next-action persistence.
- [ ] durable vs ephemeral classification.

Acceptance criteria:

- Worker can retrieve a prior architectural decision without parent chat history.
- Failed approaches can be surfaced to prevent repetition.
- Task completion writes only durable memory candidates.
- Raw tool logs are not blindly stored as durable organizational memory.

---

# 35. Test strategy

## Unit

Test:

- routing policy,
- Task DAG state transitions,
- context budgeting,
- capsule serialization,
- provider adapters with fixtures,
- thread actor serialization,
- escalation rules,
- memory classification parsing.

## Integration

Use captured/fake Codex events to test:

- thread lifecycle,
- turn lifecycle,
- reconnect,
- interrupted streams,
- duplicate events,
- stale thread state,
- steering,
- resume.

## E2E

Create fixture repositories.

Scenarios:

1. trivial rename -> Spark.
2. multi-file change -> Codex.
3. ambiguous architecture request -> Astra plan.
4. two parallel Codex tasks -> separate worktrees.
5. Codex failure twice -> Astra replan.
6. worker requests prior decision -> Context Broker -> OrgBrain.
7. parent context very large -> child worker gets small capsule.
8. app restart -> resume.
9. Spark unavailable -> Codex fallback.
10. Astra unavailable -> manual/Codex fallback.

---

# 36. Context-efficiency benchmark

This is mandatory.

Create a benchmark comparing:

## Baseline

Codex Subagent/fork-style context inheritance.

## Hub

New thread + Context Capsule + pull retrieval.

Record:

- parent context size,
- child initial input tokens,
- child total input tokens,
- cached tokens,
- task success,
- task latency,
- total model usage.

Success criterion:

For suitable multi-agent tasks, Hub child-worker initial context should be substantially smaller than parent context without materially reducing completion quality.

Do not hardcode a percentage target until measurements exist.

---

# 37. Observability

Expose:

- routing decisions,
- active workers,
- task DAG,
- context per worker,
- context retrievals,
- token usage,
- model/provider,
- latency,
- retries,
- escalation count,
- test status,
- diff size.

Write structured logs.

Use correlation IDs:

```text
project_id
plan_id
task_id
worker_id
thread_id
turn_id
```

---

# 38. Failure handling

Handle:

- Codex process crash.
- app-server protocol error.
- response stream disconnect.
- Spark service unavailable.
- Spark malformed structured output.
- Astra unavailable.
- worktree conflict.
- test timeout.
- stale active-turn state.
- repeated turn failure.
- app restart.

Persist enough state to reconstruct the Hub lifecycle.

---

# 39. Protocol-version isolation

Codex app-server changes quickly.

Therefore:

```text
UI
 |
Hub domain
 |
CodingAgentProvider trait
 |
Codex protocol adapter
 |
Codex app-server
```

Never:

```text
UI -> raw Codex JSON
```

Add protocol compatibility tests using captured fixtures.

---

# 40. Initial configuration

Example:

```toml
[hub]
db_path = "~/.astra-codex-hub/hub.db"
worktree_root = "~/.astra-codex-hub/worktrees"

[spark]
enabled = true
endpoint = "http://127.0.0.1:8765"
model = "Spark-X2.5-4B-MLX-8bit"

[routing]
spark_confidence_threshold = 0.75
spark_max_estimated_loc = 100
codex_retry_limit = 2

[context.spark]
initial_tokens = 2000
max_tokens = 8000

[context.codex_normal]
initial_tokens = 6000
max_tokens = 24000

[context.codex_deep]
initial_tokens = 12000
max_tokens = 64000

[astra]
mode = "codex_integrated"

[orgbrain]
enabled = false
```

---

# 41. Implementation order for Codex

Codex should execute the project in this strict order.

## Milestone A

- [ ] Create monorepo skeleton.
- [ ] Add CI.
- [ ] Add formatting/linting.
- [ ] Implement domain types.
- [ ] Implement SQLite migrations.
- [ ] Implement basic Tauri shell.

Stop and verify build.

## Milestone B

- [ ] Implement provider-codex.
- [ ] Start app-server.
- [ ] Initialize protocol.
- [ ] Start thread.
- [ ] Start turn.
- [ ] Stream normalized events.
- [ ] Persist mappings.
- [ ] Implement steer/interrupt.
- [ ] Implement resume.

Stop and run E2E against a fixture repo.

## Milestone C

- [ ] Add ThreadActor.
- [ ] Add EventBus.
- [ ] Add UI event stream.
- [ ] Add Diff/Terminal views.
- [ ] Add crash/restart recovery.

Stop and verify Phase 1 acceptance criteria.

## Milestone D

- [ ] Create Spark MLX service.
- [ ] Add Spark provider.
- [ ] Add router.
- [ ] Add simple Spark worker.
- [ ] Add progress summarization.
- [ ] Add first-pass review.

Stop and verify Phase 2 acceptance criteria.

## Milestone E

- [ ] Add worktree manager.
- [ ] Add task DAG.
- [ ] Add scheduler.
- [ ] Add Context Capsule.
- [ ] Add Context Broker.
- [ ] Add pull retrieval.
- [ ] Add Context Inspector.
- [ ] Add concurrent Codex workers.

Stop and run context-efficiency benchmark.

## Milestone F

- [ ] Add Astra adapter.
- [ ] Add planning schema.
- [ ] Compile plans to DAG.
- [ ] Add final review.
- [ ] Add failure diagnosis/replan.
- [ ] Add escalation policies.

Stop and verify Phase 4 acceptance criteria.

## Milestone G

- [ ] Add OrgBrain adapter.
- [ ] Add durable memory extraction.
- [ ] Add memory retrieval.
- [ ] Add memory provenance.
- [ ] Run cross-session tests.

---

# 42. Rules Codex must follow while implementing

1. Do not collapse provider abstractions into app code for convenience.
2. Do not use Codex Subagent V2 as the implementation shortcut.
3. Do not use `thread/fork` as worker spawn by default.
4. Do not copy full parent conversation into child workers.
5. Do not create hidden shared mutable workspaces for concurrent workers.
6. Do not let LLM output directly mutate Hub state without validation.
7. Validate all structured model output.
8. Keep provider-wire schemas isolated.
9. Add tests at every milestone.
10. Keep the application runnable after each milestone.
11. Prefer small commits grouped by milestone/subsystem.
12. Record unresolved upstream Codex protocol issues in `docs/PROTOCOLS.md`.

---

# 43. Definition of V1 done

V1 is complete when this workflow succeeds:

```text
User opens project
      |
      v
asks for substantial change
      |
      v
Hub routes to Astra plan
      |
      v
Astra creates DAG
      |
      +--> Spark small task
      |
      +--> Codex worker A
      |
      +--> Codex worker B
              |
              v
      isolated worktrees
              |
              v
      Spark verification
              |
              v
      Astra final review
              |
              v
      user sees diff + result
```

And:

- workers do not inherit full parent context,
- context usage is inspectable,
- additional context is pulled on demand,
- Codex sessions persist,
- app restart can recover,
- Spark runs locally,
- Astra is optional/fallback-capable,
- OrgBrain can supply durable memory,
- model usage is observable.

---

# 44. Expected architecture outcome

Final conceptual model:

```text
Astra = intelligence ceiling
Codex = execution engine
Spark = local nervous system
Hub   = deterministic control plane
OrgBrain = durable organizational memory
Tauri = operator workspace
```

The architecture should provide Codex-like usability while offering materially better control over:

- context inheritance,
- multi-agent token usage,
- worker isolation,
- routing,
- observability,
- long-term memory,
- planning/review separation.

