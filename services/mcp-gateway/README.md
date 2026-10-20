# Localoud public MCP gateway (Mac first)

This is the public HTTPS transport for the standalone Localoud MCP. It uses
Cloudflare Workers, SQLite-backed Durable Objects, and Workers KV. It does not
use OpenAI Secure MCP Tunnel. Deploying this endpoint does not publish a listing
in the ChatGPT Plugin Directory or establish approval for public distribution.

```text
ChatGPT Web -- HTTPS + OAuth --> Worker /mcp
                                    |
                              owner Durable Object
                                    |
                         authenticated reverse WebSocket
                                    |
                         Mac agent -> Rust stdio MCP
                                    |
                         local approval -> OS operation
```

The Mac opens the outbound connection. No inbound router configuration or public
Mac HTTP endpoint is needed. The loopback approval app is local to the Mac.

## Build and local verification

From the repository root:

```sh
cargo build --release -p localoud-mcp
npm --prefix plugins/localoud-mcp/agent ci
npm --prefix services/mcp-gateway ci
npm --prefix plugins/localoud-mcp/agent test
npm --prefix services/mcp-gateway test
```

Gateway tests run in workerd/Miniflare. The relay test launches two actual Rust
MCP subprocesses against separate temporary projects and checks ownership,
approval, denial, duplicate requests, and independent filesystem effects.
Those tests do not establish Cloudflare deployment or ChatGPT Web acceptance.

## Dedicated Cloudflare deployment

Use an explicitly selected Cloudflare account. Do not reuse the OrgBrain Worker
or its credentials, storage, or deployed resources. The nested OrgBrain worktree
provided an OAuth-provider reference, not a Localoud reverse relay.

1. Authenticate Wrangler with the intended account. Workers Scripts and KV write
   permissions are needed, along with account/user lookup.
2. Create a dedicated KV namespace and bind it as **OAUTH_KV**. It stores hashed
   OAuth credentials and encrypted grant properties through the provider library.
3. Set `account_id` and the canonical `vars.PUBLIC_ORIGIN` in `wrangler.jsonc`.
   The origin must match the actual stable HTTPS hostname exactly.
4. Keep all three Durable Object bindings and the initial SQLite migration.
5. Run `npx wrangler deploy --dry-run`, inspect bindings, then deploy.
6. Verify `/healthz`, protected resource / authorization metadata, unauthenticated
   `/mcp` returning 401, and actual authenticated owner-isolated operations.

The checked-in deployment must contain a real OAUTH_KV binding before use; a
successful bundle/dry-run alone cannot detect a missing runtime OAuth store.

## Account and device authentication

This first version uses possession of a paired Mac as the login identity. It
does not ask users for OpenAI API keys or Cloudflare credentials. Installing
the local agent provisions an opaque owner/device identity and a device token.
Each owner currently has one registered Mac; a shared multi-device account and
email recovery are not implemented.

ChatGPT uses OAuth 2.1 authorization code + S256 PKCE. The gateway checks token
identity, audience and scopes. Mac device tokens are separate from ChatGPT OAuth
tokens. An OAuth connection requires both local pairing approval on the Mac and
browser-bound consent for the verified client/redirect and selected scopes.
Neither a copied owner ID nor an arbitrary model-supplied device ID grants access.

## Operations and approval

The read tools match the standalone MCP. `file_create`, `file_edit`, and
`command_run` additionally require a UUID `request_id`, unique per intended
operation. A retry with the same ID and same input returns the same operation;
reusing the ID with different input is rejected. These tools return
`pending_approval` without executing. The owner reviews the exact arguments in
the local approval app. Use **operation_get** after approval to read the result;
do not repeat the write to check its status.

The local gate persists the complete request digest and a running marker before
execution. A recovered running operation becomes indeterminate and is not
executed again. Request expiry is ten minutes. Agent offline requests are not
queued for future automatic execution. Read calls wait at most fifteen seconds.

Commands run with the Mac user's OS permissions. They are not sandboxed and can
affect files/network outside a project. Only selected config roots are exposed
by the file tools; those file boundaries do not restrict an approved OS command.
The local Rust child receives a minimal environment without gateway credentials.

## Data and lifecycle

File snippets, edit content and command output pass through the gateway and
ChatGPT. Never describe this as all-local processing or end-to-end encryption.
Application logging is disabled by default; OAuth and operation records are
still stored for their documented lifecycle. Cloud job receipts expire after
roughly twenty minutes; local private approval history is retained, capped at
256 operations, and rejects new requests at capacity rather than forgetting
operation IDs. Login pairing expires after ten minutes. Device enrollment and
dynamic client registration are rate limited per Cloudflare client-IP bucket.

Device revocation (`POST /agent/revoke`, authenticated by that device token)
closes its connection and denies subsequent owner operations. Stop the Mac
agent to make the device unavailable. Do not delete or reset its private state
to retry an uncertain operation. Review the local receipt and side effects first.

Public-directory review, Mac signing/notarization, a bundled Node runtime,
account recovery, and broader platform support remain separate release work.
