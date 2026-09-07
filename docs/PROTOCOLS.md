# Provider protocols

Codex integration uses the public app-server stdio JSON-RPC surface:
https://learn.chatgpt.com/docs/app-server

The locally installed binary supports `codex app-server generate-json-schema --out <directory>`. Generate compatibility fixtures from the binary actually used; do not rely on undocumented desktop state.

Required lifecycle: initialize → initialized → thread/start or thread/resume → turn/start; turn/steer and turn/interrupt require the active turn identity. A worker uses thread/start, not thread/fork.

Verified against Codex 0.153.4: handshake, normalized streaming events, lifecycle, serialized turn changes, interrupt/steer, process restart/resume, worker dynamic tools and usage. Raw schemas were generated from the installed binary. Unknown notifications do not imply completion. Provider approval/input requests currently return a visible denial; implementing a general approval UI remains open. Transport loss or timeout requires reconciliation before further mutations; there is no blind retry of ambiguous requests.

Spark is a separate loopback MLX service and uses **8bit** quantization. Exact model repository and checkpoint metadata must be verified before inference. Unavailable or malformed Spark output must remain visible and route to a supported fallback.

Astra integrated access and direct API billing are distinct. Do not infer that a ChatGPT/Codex entitlement grants API credentials. OrgBrain MCP initialization, tools/list and tools/call are implemented with bounded responses and project/tenant scoping; production credentials remain unconfigured. File-backed loopback fixtures verify persistence and cross-session retrieval.
