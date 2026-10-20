# localoud-mcp plugin

Standalone local MCP server extracted from the Localoud desktop app
(`apps/desktop/src-tauri/src/read_mcp.rs` + `mcp_files.rs`), runnable without
Tauri, the desktop DB, or model credentials. For Codex CLI, Claude Code, and
any MCP-compatible client over **stdio** (default) or loopback **HTTP**.

## Tools (10)

Read-only by default: `project_list`, `project_get`, `repo_tree`, `file_read`,
`file_search`, `git_status`, `git_diff`. Opt-in per project:
`file_create`, `file_edit` (exact-match + `sha256` from `file_read`,
atomic write, permission-preserving).
Separately opt-in: `command_run` (literal program/args, working directory,
exit status, capped stdout/stderr, timeout and process-group cleanup on Unix).

File-tool boundaries are inherited from the desktop server: 1 MiB file cap,
64 KiB response cap, dotfiles / secrets / symlinks / build outputs excluded,
`AGENTS.md` / `CLAUDE.md` / `SKILL.md` never writable through the file tools.
These restrictions do not confine an explicitly enabled host command.

Not included: `task_list`, `task_get`, `verification_get`, `handoff_schema`.
Those need the desktop app's SQLite workflow state and stay in the desktop
MCP (`http://127.0.0.1:8792/mcp` when the app runs).

## Install

```sh
./scripts/install.sh
```

Builds `crates/localoud-mcp` in release mode, links `~/.local/bin/localoud-mcp`,
and seeds `~/.config/localoud/mcp-projects.json` from
`mcp-config.example.json` on first run. Edit the config: absolute repo
`root`s, stable `id`s, `write_enabled: true` only for projects where an
agent may edit. Set `execution_enabled: true` separately to permit host
commands; old configs and the example keep execution disabled.

Config resolution order: `--config PATH` > `$LOCALOUD_MCP_CONFIG` >
`~/.config/localoud/mcp-projects.json`. Quick single-project mode without a
config file:

```sh
localoud-mcp --project-root /absolute/repo [--allow-write] [--allow-exec]
```

`--allow-exec` only applies to `--project-root`; for a config file, enable
each intended project with `execution_enabled`. This is independent of
`write_enabled`: commands can change files even if the file-edit tools are off.

## Command execution

After verifying `command_execution_enabled` in `project_list`, call:

```json
{
  "name": "command_run",
  "arguments": {
    "project_id": "my-project",
    "program": "cargo",
    "args": ["test", "-p", "localoud-mcp"],
    "cwd": ".",
    "timeout_ms": 120000
  }
}
```

`program` is an executable name or path; `args` are literal arguments.
No shell expansion or pipeline is added. A shell can be explicitly invoked,
for example `program: "/bin/sh"` with `args: ["-c", "..."]`.
The default cwd is the project root. A supplied cwd must be a relative,
publishable directory without symlinks (or `.`).

This runs with the server user's OS permissions, **not in a filesystem or
network sandbox**. Commands can access files outside the project, credentials
stored on disk, the network, and perform deletion or other side effects.
Only enable execution for projects and MCP clients you trust. The server
clears inherited environment variables, then passes only `PATH`, `HOME`,
`TMPDIR`, `LANG`, and `LC_ALL` when present; it does not forward the MCP token
or other ambient credential variables. Tools may still read normal HOME config.

Defaults/limits: timeout 30 seconds, configurable from 1 to 120000 ms; at most
4 active commands per server; at most 256 args and 32KiB combined program/arg
bytes. stdin is closed. Each output pipe retains at most 16KiB, continues
draining, and applies best-effort credential redaction. JSON escaping can
reduce retained output further to keep the response bounded. Inspect
`output_truncated` and `output_redacted`; missing output is not proof of success.

Receipts contain `operation_id`, `project_id`, `cwd`, `exit_code`, `signal`,
`success`, `timed_out`, `timeout_ms`, `duration_ms`, `stdout`, and `stderr`.
`exit_code` is null when the process terminates by signal. A nonzero exit or
timeout is returned as a receipt with `success: false`; configuration/launch
errors are MCP tool errors. Never blindly retry a command with side effects.
On timeout, cancellation or completion the server kills its Unix process group;
a deliberately detached process can escape this cleanup. Windows command
execution is currently unsupported. This tool is in the standalone plugin;
the desktop MCP retains its existing file-only behavior.

Send `notifications/cancelled` with `params.requestId` matching an active
command request to cancel it. stdio continues accepting input while commands
run; responses may arrive out of order and must be matched by `id`.
Cancellation returns JSON-RPC error `-32800` for the original request.
Stopping a process does not undo side effects already performed.

## Codex CLI

Append to `~/.codex/config.toml` (see `codex-config.example.toml`):

```toml
[mcp_servers.localoud]
command = "localoud-mcp"
args = ["--config", "/Users/YOU/.config/localoud/mcp-projects.json"]
```

## Claude Code

Project scope: copy `.mcp.json` into the project root and approve the server
when prompted. User scope:

```sh
claude mcp add localoud -- localoud-mcp --config ~/.config/localoud/mcp-projects.json
```

## HTTP (optional)

Loopback only: the resolved bind address must be loopback, and every request
must carry a loopback `Host` (foreign `Origin`s are rejected, blocking
browser / DNS-rebinding requests). HTTP requires a Bearer token via
`LOCALOUD_MCP_TOKEN`; pass `--insecure-no-token` to serve without auth
(loopback only, not recommended). A token is always required for HTTP if any
project enables command execution, even with `--insecure-no-token`.
stdio clients never need a token:

```sh
LOCALOUD_MCP_TOKEN="$(openssl rand -hex 32)" \
  localoud-mcp --config ~/.config/localoud/mcp-projects.json --http 127.0.0.1:8792
```

Then `POST http://127.0.0.1:8792/mcp` with
`Accept: application/json` + `Authorization: Bearer <token>`.
`--http ADDR --stdio` serves both transports at once. Sending
`MCP-Protocol-Version: 2026-07-28` (or the matching `params._meta` marker over
stdio) adds the 2026-07-28 result envelope (`resultType` / `ttlMs` /
`cacheScope` / `_meta`).

HTTP `initialize` returns `Mcp-Session-Id`; include that header on subsequent
requests and cancellation notifications. Cancellation is isolated to that
session. Unknown or expired IDs return 404; initialize again. Up to 128 HTTP
sessions are retained, expiring after 15 idle minutes. Stateless probes are
accepted, but cancellation requires an initialized HTTP session.

## ChatGPT Web through Secure MCP Tunnel

Use OpenAI's [Secure MCP Tunnel](https://developers.openai.com/api/docs/guides/secure-mcp-tunnels)
with this server as a **stdio** target. No Cloudflare tunnel or public local
HTTP endpoint is needed. Install the official `tunnel-client`, create/select
the remote tunnel, and connect that tunnel to the intended ChatGPT workspace.
Use a runtime key with tunnel Read + Use access; keep an administrator key out
of the runtime environment.

The macOS helper reuses the installed client and release binary:

```sh
cargo build --release -p localoud-mcp
python3 plugins/localoud-mcp/scripts/tunnel.py start \
  --config /absolute/path/mcp-projects.json \
  --tunnel-id YOUR_EXISTING_TUNNEL_ID
python3 plugins/localoud-mcp/scripts/tunnel.py status
python3 plugins/localoud-mcp/scripts/tunnel.py doctor
# When finished:
python3 plugins/localoud-mcp/scripts/tunnel.py stop
```

Run these commands from the repository root. By default the helper uses
`~/Library/Application Support/LocaloudTunnel/tunnel-client`, alias
`localoud-mcp-plugin`, profiles under `target/localoud-mcp-tunnel/profiles`,
and the loopback health/admin port 8794. Override the client, binary, alias,
profile directory (under this repository's `target`), or health port with the
corresponding flags shown by `start --help`.

The helper reads `CONTROL_PLANE_API_KEY` from its own environment, or privately
reads the existing macOS Keychain entry with service `dev.localoud.mcp-tunnel`
and account `localoud-tunnel-runtime`. It passes the key only in the child
environment and stores an `env:CONTROL_PLANE_API_KEY` reference in the profile.
It never accepts a key on the command line. Status output is limited to runtime
state fields; doctor prints a summary and uses an ephemeral health port so it
can run alongside the active runtime. Stop independently checks status.

Keep any other runtime targeting the **same remote tunnel** stopped, including
the legacy `localoud-readonly` desktop profile. The new stdio target exposes
this plugin's ten tools; desktop-only task/manifest tools are not included.
After changing tools, use **Update tools** in the existing ChatGPT plugin and
approve the intended capabilities. Verify `project_list`, then an edit with
the latest file hash, then a command and file readback in a dedicated test
project. Runtime readiness alone does not prove a successful ChatGPT tool call.

The verified 2026-10-18 setup uses only project `localoud-web-e2e`, configured in
`target/localoud-mcp-tunnel/mcp-projects.json`, rooted at
`/private/tmp/localoud-mcp-web-e2e-20261018`. Both opt-ins are enabled for that
fixture. Its temporary root must be recreated if macOS removes it. The runtime
is managed by `tunnel-client`; automatic startup after reboot is not configured.
See [verification evidence](../../docs/evidence/mcp-web-tunnel-2026-10-18.md).

## Public HTTPS gateway and Mac approvals

The public connection uses [the Worker gateway](../../services/mcp-gateway/README.md)
and [the Mac agent](agent/README.md). It is independent of Secure MCP Tunnel.
ChatGPT authenticates through OAuth; each Mac opens an outbound WebSocket.
The gateway routes requests by the verified owner identity. Edits and commands
return an operation ID and wait for approval on the Mac. Use `operation_get`
after approval instead of retrying a write.

The Mac agent's private state and published project roots must be disjoint.
Commands run with the Mac user's privileges; approval does not sandbox them.
The current Mac distribution requires Node.js 22+ and the Rust release binary.
An authenticated deployed endpoint does not establish Plugin Directory approval.

## Verify

```sh
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2026-07-28"}}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
  | localoud-mcp --project-root /absolute/repo
# expect protocolVersion + 10 tools
```

## Layout

- `../../crates/localoud-mcp/` — the extracted server (`src/lib.rs`,
  `src/main.rs`). Workspace member, covered by `cargo test --workspace`.
- `.claude-plugin/plugin.json` — Claude Code plugin manifest.
- `.mcp.json` — Claude Code project-scope server entry (example).
- `codex-config.example.toml` — Codex CLI snippet.
- `mcp-config.example.json` — project allow-list (example).
- `scripts/install.sh` — build + registration helper.
- `scripts/tunnel.py` — macOS managed Secure MCP Tunnel helper.
- `tests/test_tunnel.py` — launcher behavior and secret-handling regressions.
