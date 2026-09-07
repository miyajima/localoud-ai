# Security boundaries

- Projects must resolve to the canonical root of a local Git repository.
- Registering a project does not grant access to sibling directories.
- Codex owns its sandbox and approval protocol. The Hub must not silently accept provider approval requests.
- Credentials belong in the platform keychain, never SQLite or capsules.
- Events require secret redaction before durable persistence or broadcast.
- Worktree paths must be generated from typed IDs under a controlled root; cleanup must refuse unowned or dirty paths.
- The desktop loads bundled assets under a restrictive content security policy; arbitrary remote scripts are not allowed.
- No automatic merge into protected branches.

Provider execution, journal redaction and worktree ownership checks are implemented and exercised by fixture tests. Provider approval/input requests fail visibly; a general approval-forwarding UI remains unimplemented. OrgBrain auto-storage is off by default and additionally requires Astra approval bound to the exact task goal and current diff. Keychain credentials are never included in capsule or usage records.
