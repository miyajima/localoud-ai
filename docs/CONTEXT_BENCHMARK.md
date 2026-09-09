# Context inheritance benchmark — 2026-09-08 JST

For the subsequent **real native `spawn_agent` comparison**, see [the 2026-09-09 results](NATIVE_AGENT_BENCHMARK.md). That single pair showed 8.2% higher total input for Localoud, despite lower first input. The synthetic fork result below must not be described as native-subagent savings.

A real Codex app-server fork baseline and a new independent thread implemented the same Python interval utility (normalize/subtract/contains), from the same Git commit in separate worktrees. Both pulled the same interval policy through `hub_context`; both passed the same 9 fixed acceptance tests. The parent received a synthetic, irrelevant historical archive; its measured initial input was 167,886 tokens.

| Observed quantity | Fork baseline | Capsule + pull |
|---|---:|---:|
| Worker first input tokens | 174,886 | 28,121 |
| Worker total input tokens | 895,666 | 127,904 |
| Cached input tokens | 712,832 | 106,752 |
| Output tokens | 1,901 | 2,052 |
| Model inference count | 5 | 4 |
| Worker elapsed | 81.147 s | 78.300 s |
| Nonzero command exits | 1 | 0 |
| Fixed acceptance | 9/9 | 9/9 |
| Selected capsule estimate | 269 | 269 |
| Pulled context estimate | 109 | 109 |

In this pair, the independent worker's first input was 83.9% smaller and total input was 85.7% smaller. This is **one stochastic pair on a synthetic archive**, not a general savings estimate. Different command counts contribute to total-input differences. Cached tokens are included in input tokens and must not be added again. No subscription or API cost claim is made.

The reported model was `gpt-6-astra` for both workers. Worker creation did not override the user's configured Codex model. Parent archive setup consumed 167,886 input / 8 output tokens and 9.888 seconds; that setup is excluded from both child totals. Capsule estimates use UTF-8 bytes / 3 and are **not provider-tokenizer counts**. The 28,121-token first request includes provider system instructions/tool definitions and other provider context; it is not the capsule size.

Evidence: [machine-readable report](evidence/context-benchmark.json). Reproduce explicitly with `ASTRA_LIVE_FIXTURE=1 cargo run -p hub-runtime --example context_benchmark`. This creates and retains a new temporary fixture, makes real model calls, and runs generated Python code inside that fixture. Production worker dispatch uses `thread/start`; `benchmark_fork` is guarded to opt-in temporary fixtures only.
