# Localoud Mac agent

Mac-first sidecar for the public Localoud MCP gateway. Requires Node.js 22+ and
the standalone `localoud-mcp` release binary. This first version uses a loopback
browser approval screen; it is not yet a signed/notarized `.app` installer.

From the repository root, build the binary and install the locked dependency:

```sh
cargo build --release -p localoud-mcp
npm --prefix plugins/localoud-mcp/agent ci
node plugins/localoud-mcp/agent/main.mjs \
  --gateway https://YOUR-LOCALOUD-GATEWAY \
  --config /absolute/private/mcp-projects.json \
  --state-dir /absolute/private/localoud-agent-state
```

The config must be a private regular file (0600). Choose explicit project IDs
and absolute roots; `write_enabled` and `execution_enabled` are separate opt-ins.
The selected state directory must be owned by you with mode 0700. The agent
refuses unsafe existing files instead of changing their permissions for you.
The private state directory and every published root must be disjoint: neither
may contain the other. This prevents even read-only file tools from exposing
device credentials or local UI session codes. Changing the canonical roots,
directory identities, or capability flags invalidates prior pending operations.
Use a dedicated temporary project for initial verification.

The default approval URL is `http://127.0.0.1:8795`. Use `--port` to change it.
Open that URL and enter the local session code saved privately in `ui-token`
inside the selected state directory. The code authenticates only to the Mac's
loopback approval app. Do not share `ui-token`, `device.json`, or approval history.
No credentials are printed by the launcher.

In ChatGPT Web, add the gateway's stable HTTPS `/mcp` as a custom MCP plugin with
OAuth. In its connection screen, copy the pairing code into the Mac approval
app. Review the client name/domain, redirect host and scopes, approve locally,
then finish consent in the ChatGPT connection screen. Every new pairing is
explicit; there is no automatic broad authorization.

For each edit or command, review the project, path/program, arguments, expiry
and SHA-256 request digest. Choose **承認して実行** or **拒否**. Before approval,
no mutating request is forwarded to Rust. ChatGPT must use `operation_get` to
read the result after approval, rather than retrying the operation.

The approval app enforces a private session cookie, exact loopback Host,
same-origin requests, CSRF token, and exact request digest. OS commands are
still executed with your Mac user's privileges; local approval is not a sandbox.
For example, approving `/bin/sh -c ...` permits everything that shell command
can do as you. File-tool project boundaries do not confine that command.

Stop with SIGINT/SIGTERM. The agent closes its WebSocket, local approval screen,
and stdio child, sending cancellation before shutdown. A recovered operation
that was running is marked indeterminate and is never automatically replayed.
Only one agent may use a state directory at a time. Automatic login startup,
multi-device account linking and signed distribution are not configured here.

```sh
npm --prefix plugins/localoud-mcp/agent test
```

See the [gateway guide](../../../services/mcp-gateway/README.md) for Cloudflare
deployment, authentication boundaries, integration tests and receipt retention.
