# Security boundaries

- Projects must resolve to the canonical root of a local Git repository.
- Registering a project does not grant access to sibling directories.
- Codex owns its sandbox and approval protocol. The Hub must not silently accept provider approval requests.
- OrgBrain credentials use the platform keychain. The MCP bearer token is stored in macOS Keychain. ChatGPT uses a separate persistent WKWebView data store with no Localoud native capabilities. The retired browser Bridge and Chrome extension are removed. Do not share these runtime files.
- MCP file creation and literal edits require a separate per-project opt-in (`write_project_ids`, a subset of published `project_ids`); old configurations default to no write permissions. Writes remain bearer-authenticated and loopback-bound. Task artifacts, hidden/internal paths, sensitive candidates and agent instruction files are excluded. Capability-relative no-follow opens reject symlinks; edits reject hard links and non-text/oversized/read-only/redacted files. Hash checks and serialized MCP writes reject stale changes; external writers are not covered by a filesystem compare-and-swap guarantee. New files never overwrite an existing destination, and edits replace atomically while preserving permissions. No shell, deletion, Git mutation or worker-control tools are exposed. The A2A read-evidence endpoint still rejects all write tools.
- Events require secret redaction before durable persistence or broadcast.
- Worktree paths must be generated from typed IDs under a controlled root; cleanup must refuse unowned or dirty paths.
- The desktop loads bundled assets under a restrictive content security policy; arbitrary remote scripts are not allowed.
- No automatic merge into protected branches.

Provider execution, journal redaction and worktree ownership checks are implemented and exercised by fixture tests. Provider approval/input requests fail visibly; a general approval-forwarding UI remains unimplemented. OrgBrain auto-storage is off by default and additionally requires Astra approval bound to the exact task goal and current diff. Keychain credentials are never included in capsule or usage records.
