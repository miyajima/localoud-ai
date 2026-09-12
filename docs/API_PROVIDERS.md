# API provider profiles

Localoud v1 supports four execution families:

- Codex app-server (`codex_app_server`)
- multiple OpenAI-compatible Chat Completions profiles (`open_ai_chat`)
- OpenAI Responses profiles (`open_ai_responses`)
- Anthropic Messages profiles (`anthropic_messages`)

The dedicated Spark service remains the built-in local profile. Provider profiles are identified by profile ID, so two endpoints may expose the same model ID without becoming ambiguous.

Agent-to-agent handoffs use an A2A v1 Message envelope with a structured Localoud data Part. Provider adapters translate that envelope into Codex, OpenAI, Anthropic, or local-model native input; raw A2A JSON is not assumed to be accepted by an LLM provider. See [A2A support](A2A.md).

## Credentials and billing

Profiles store only an environment-variable name such as `OPENAI_API_KEY` or `ANTHROPIC_API_KEY`. Secret values are never returned to the UI or written to SQLite, settings JSON, events, or child-process environments. The settings screen reports only whether the named variable was present when Localoud started.

After adding or changing an environment variable, fully quit and relaunch Localoud from an environment that contains it. Apps opened from Finder do not normally inherit an interactive shell's environment. Localoud does not import keys from Claude Code, Claude.ai, or a shell profile.

Claude.ai Pro, Max, Team, and Enterprise subscriptions are separate from Anthropic Console/API billing. A Claude subscription or Claude Code login is not an Anthropic Messages API credential. See [Anthropic's billing explanation](https://support.claude.com/en/articles/9876003-i-have-a-paid-claude-subscription-pro-max-team-or-enterprise-plans-why-do-i-have-to-pay-separately-to-use-the-claude-api-and-console).

## Connection rules

- Cloud profiles require HTTPS.
- Plain HTTP is accepted only for an unauthenticated loopback endpoint.
- URLs containing credentials, query strings, fragments, or a redirect are rejected.
- Profile save performs a non-generating model-list request. It does not prove tool-call compatibility.
- A generic OpenAI-compatible model becomes eligible for the full worker only after the user explicitly runs the billed tool canary and it succeeds for that exact profile revision and model.
- Per-profile concurrency is configurable from 1 to 8. Different profiles can run concurrently; turns within one logical session remain serialized.

## Sessions and model changes

The selected `profile_id`, model ID, reasoning effort, and profile revision are persisted before each provider POST. A model/profile change applies to the next turn and starts a new provider segment. Visible text, tool calls, and tool results are carried forward as normalized history.

Provider-specific hidden state is replayed only while the exact target and segment continue. Anthropic thinking, signatures, and redacted-thinking blocks are preserved for that same-segment replay, but are omitted across provider/model changes, including a later change back to the original model.

Refusal, rate limiting, authentication failure, context overflow, stream interruption, oversized response, malformed response, and transport failure are recorded separately. Localoud does not automatically replay an ambiguous billed POST or silently substitute another model after a turn has started.

## Write and review boundary

Unscoped Codex and API conversations are read-only. When one or more target files are explicitly selected, Localoud converts the request into a reviewed worktree task. Larger or initially unbounded changes must use **Plan and implement** so that scope, dependencies, and acceptance criteria are confirmed first.

Every enabled write path uses an isolated worktree. Completion requires a new reviewer session that receives the goal, acceptance criteria, exact diff, and verification evidence without executor history. The reviewer may use the same model, but its logical and provider session IDs must differ. A pass is accepted only while the artifact hash is unchanged. Rework returns to the same implementation session and invalidates the old review; unavailable or inconclusive review stops at `needs_attention`.

The retired direct-local and legacy Plan writers are fail-closed because they cannot establish this review contract. Existing records remain readable and their worktrees are not deleted or replayed.

## Verification boundary

The repository tests use deterministic local HTTP/SSE fixtures. They cover request shapes, split streams, tool calls, usage, refusal, common failure classes, concurrency limits, history segmentation, one-shot command approval, sandbox/path escapes, and review state transitions. These fixtures do not prove that a real account has a usable key, billing, model entitlement, or production-quality output. A real-provider acceptance run requires explicitly supplied environment variables and a separately authorized billed canary/task.
