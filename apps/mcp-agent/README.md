# Localoud Agent app (Mac first)

A dedicated Electron app around the existing Node agent and Rust stdio MCP.
Electron supplies its Node runtime; end users do not install Node, Rust, npm,
Wrangler or a tunnel client. The full Localoud AI desktop is a separate product.
This app uses the public Localoud gateway, not Secure MCP Tunnel.

## Install and first connection

1. Open the DMG and drag **Localoud Agent** to **Applications**.
2. Open the app and choose **プロジェクトを追加**. Select one or multiple folders
   (up to 100 registrations). Each project has its own editing and command
   execution switches, both initially off for new registrations. Save the choices
   and start the connection. The existing single-project registration keeps its
   project ID and permissions after upgrading.
3. Copy the displayed MCP URL and open ChatGPT's Plugins page. Add a custom MCP
   with server URL and OAuth. Availability depends on the user's ChatGPT plan and
   workspace settings; this app cannot grant that entitlement.
4. Start its OAuth connection. Open **接続・承認を開く** in Localoud, expand
   **ChatGPTと接続（初回・追加の接続）**, enter the pairing
   code from the browser, review ChatGPT's identity/redirect/scopes, approve on
   the Mac, then finish consent in the browser.
5. For edits/commands, review and approve each exact request in Localoud. A
   pending receipt is not execution; ChatGPT reads the result with operation_get.

Pending requests appear first. Long operation details scroll within their own
area, with approval controls outside it. Expired or denied requests are retained
in collapsed history and cannot execute. If a request expires, confirm its final
status and absence of effects before creating a new request ID; do not resend an
uncertain operation just to retrieve its result.

The local session code is handled privately by the app; no copy/paste of ui-token
is required. OAuth approval is never automatic. No account credentials are entered
into the app. Files and command output pass through the Localoud Cloudflare
service and ChatGPT. Commands run with the Mac user's permissions and are not
sandboxed by the selected file-tool root.

The app remembers the explicitly saved project list and per-project permissions,
resuming that configuration on later launches. Stop the agent before adding or
removing projects or changing their permissions; save before restarting. Removing
a registration does not delete its files. Duplicate, symlink-alias and parent/child
roots are rejected to avoid overlapping permission scopes. An unavailable folder
remains visible and blocks startup until repaired or removed. After restoring a
folder or reconnecting its disk, use **フォルダを再確認** to preserve its ID and
permissions without automatically starting. Saving an empty
list clears all registrations. Changed scope invalidates old pending approvals
when the runtime restarts; the device pairing is retained.
Closing a window keeps the menu-bar agent running. **終了（接続を停止）** closes the
relay, approval window and Rust child. Launch at login is a separate, initially
off opt-in. Local configuration and receipts stay under the app's user-data
`private/` directory (0700, regular files 0600). The private directory and the
published root must be disjoint. Do not delete state to retry uncertain work.

## Build the evaluation installer

On a matching Mac architecture, from the repository root:

```sh
cargo build --release -p localoud-mcp
npm --prefix plugins/localoud-mcp/agent ci
npm --prefix apps/mcp-agent ci
npm --prefix apps/mcp-agent run install:runtime
npm --prefix apps/mcp-agent test
npm --prefix apps/mcp-agent run package:host
```

Output: `target/localoud-agent-app/Localoud-Agent-0.1.0-ARCH-unsigned.dmg`
and ZIP, plus the `.app`. The evaluation build is ad-hoc signed where needed;
it is **not Developer ID signed or Apple notarized**. Do not present it as a
frictionless public release or tell users to bypass Gatekeeper warnings.
The packaging hook rejects a Rust executable for the wrong architecture.
Intel builds require a native Intel binary and Intel Electron build; the ARM
artifact is not an Intel or universal installer.

## Signed and notarized public release

The signing Mac needs:

- Apple Developer membership and a **Developer ID Application** identity with its
  matching private key in Keychain. A certificate without its private key cannot
  sign. A DMG distribution does not require Developer ID Installer.
- A working `notarytool` Keychain profile, previously set up by the account owner.
  Supply only its profile name to this pipeline. Alternatively run the pipeline
  on the existing signing Mac/CI where those credentials are already configured.
- Current Xcode command-line tools and the same architecture as the built Rust
  executable. Do not revoke or replace other apps' certificates.

Set `MAC_SIGN_IDENTITY` to the existing Developer ID Application identity name
and `MAC_NOTARY_PROFILE` to the existing Keychain profile name in the signing
machine's environment, then run:

```sh
npm --prefix apps/mcp-agent run release:mac
```

The release script refuses missing credentials before packaging. It signs the
app, verifies its Developer ID authority, submits it to Apple, waits for
**Accepted**, staples the app, creates/signs/notarizes/staples the DMG, and runs
Gatekeeper assessments. After every check passes, the `-signed.dmg` / `-signed.zip`
pair and a SHA-256 manifest appear together in a new directory under
`target/localoud-agent-app/releases/`. Failed attempts publish no partial pair.
Only that validated output is a public-release candidate. No certificate, private key, password, Apple ID
credential or API key belongs in Git or chat. This pipeline does not upload a
GitHub Release or submit the ChatGPT Directory listing.

## OS support boundary

| Platform | Current status |
| --- | --- |
| macOS 13+ Apple Silicon | Primary local build and verification target |
| macOS 13+ Intel | Packaging path and architecture checks provided; requires its own build and runtime verification |
| Linux | Existing agent uses Unix APIs; runtime/controller tests can run in CI. No Linux GUI installer or end-to-end acceptance established |
| Windows | Unsupported. Node private-state checks require Unix ownership/modes; Rust commands and file operations need Windows permission/process implementations |

Electron itself is cross-platform. That does not make the Unix-only agent and
Rust operations portable. The packager fails closed for non-Mac targets.
See [gateway contracts](../../services/mcp-gateway/README.md) and
[agent contracts](../../plugins/localoud-mcp/agent/README.md).

## Multi-project verification

`npm test` runs the app tests. To include the real Rust stdio/runtime approval
checks, first build `target/release/localoud-mcp`, then run from this directory:

```sh
LOCALOUD_MCP_TEST_BINARY="$PWD/../../target/release/localoud-mcp" npm test
```

The integration tests create disposable folders, use an in-process fake relay
without remote traffic, execute the real MCP binary, and read back filesystem
effects. They separately verify edit-only and execution-only permissions,
no effects before approval, rejection before approval for disabled operations,
and invalidation of pending approvals after a capability change. These tests do
not by themselves prove ChatGPT Web connectivity.
