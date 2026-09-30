# Localoud MCP file editing validation — 2026-09-30

Implementation on base HEAD `8083073`, with uncommitted changes. Added authenticated `file_create` / `file_edit` tools and separate per-project `write_project_ids` permission. No runtime publication or write permission was enabled by this implementation run.

## Actual validation

- Test-first: seven new MCP tests initially failed because write tools/configuration were absent; both new settings UI tests failed because edit controls were absent. Implementation followed these failures.
- `cargo test -p astra-desktop read_mcp::tests`: 20 passed. Includes actual loopback HTTP JSON-RPC create → edit → read, independent byte/hash checks, HTTP 413 body-size rejection, permissions, conflicts, links, sensitive paths/content, binary/read-only/oversized text and A2A write denial.
- `npm test`: all 82 frontend tests passed, including two new MCP permission interaction/persistence tests.
- `npm run build`: TypeScript and Vite build passed.
- `npm run tauri -- build --debug --no-sign --bundles app`: final macOS debug bundle built successfully at `target/debug/bundle/macos/Localoud AI.app`. Not installed, restarted, signed or remotely published in this run.
- `git diff --check`: passed.

## Existing suite failures

The full desktop Rust suite at the earlier implementation checkpoint passed 55/57 tests. `autonomous::tests::saved_routes_preserve_every_model_and_exact_effort` expected `Builder` but observed `Developer`; `autonomous::tests::duplicate_manifest_reopens_deleted_mapping_without_replaying_or_replacing_state` failed with `保存済みタスクが見つかりません`.

Both failures reproduced with the exact same causes on an untouched archive of base HEAD, running the 23 autonomous tests serially (21 passed / 2 failed). They were not changed as part of MCP editing. The final HTTP transport test was added after the full-suite checkpoint; the final 20 MCP tests all passed.

## Follow-up: both stale test expectations corrected

After the user's request to fix these failures, the two original failures were reproduced again. The agent test used an obsolete level-3 default name (`Builder`; current `Developer`). It now verifies serialization and freezing of all five routes' user-defined agent names, model IDs and reasoning choices through the actual agent-profile synchronization path, including later settings changes.

The duplicate-Manifest test called full task deletion, which intentionally deletes the `autonomous:` snapshot, then expected that deleted state to reopen. It now verifies that archiving can be reversed through duplicate import without replacing the interrupted workflow or receipt, and separately verifies that full deletion removes the snapshot while retaining the import receipt to reject replay and resurrection. A wrong-project import must leave the archive untouched.

Only `apps/desktop/src-tauri/src/autonomous_tests.rs` was changed for this follow-up; production deletion, routing and import behavior were already consistent with these contracts. `cargo test --workspace` now exits 0: **145 passed, 0 failed**, including **58 desktop tests** and the MCP regressions. No frontend or production binary code changed in this follow-up, so no further app rebuild was needed.

## Scope of evidence

This validates the Localoud implementation and local HTTP transport. It does not claim a new end-to-end call from ChatGPT Web against this updated Localoud bundle. The earlier disposable custom-MCP Web test established that this account could invoke write tools, but remains a separate experiment.

Use MCP settings to explicitly allow editing a published project, refresh the Localoud tools in ChatGPT, verify `project_list.file_edit_enabled`, and use an exact project UUID. UI selection does not automatically change MCP scope. Read the file first, supply its SHA-256 and one unique literal match, then read back and inspect the diff. Atomic replacement and serialized MCP writes do not provide an OS-level compare-and-swap against independent external editors.

## Original final test output

```text
Finished `test` profile [unoptimized + debuginfo] target(s) in 3.90s
     Running unittests src/main.rs (target/debug/deps/astra_desktop-ec37ac50eade6214)

running 20 tests
test read_mcp::tests::mcp_edit_config_defaults_to_read_only_and_rejects_unpublished_permissions ... ok
test read_mcp::tests::a2a_card_and_read_message_use_v1_envelopes ... ok
test read_mcp::tests::bounded_reads_deny_escape_secrets_and_symlinks_without_writes ... ok
test read_mcp::tests::redaction_preserves_json_and_pagination_is_explicit ... ok
test read_mcp::tests::mcp_edit_creates_and_replaces_exact_text_with_receipts ... ok
test read_mcp::tests::sensitive_diff_cannot_smuggle_a_fake_header_in_file_content ... ok
test read_mcp::tests::mcp_edit_serializes_updates_and_requires_authentication ... ok
test read_mcp::tests::mcp_edit_is_advertised_as_write_and_a2a_stays_read_only ... ok
test read_mcp::tests::mcp_edit_denies_escapes_links_sensitive_paths_and_non_text ... ok
test read_mcp::tests::mcp_edit_requires_explicit_project_permission_and_source_scope ... ok
test read_mcp::tests::authenticated_rpc_advertises_bounded_tools_and_rejects_execution ... ok
test read_mcp::tests::mcp_edit_http_transport_creates_edits_reads_and_enforces_body_limit ... ok
test read_mcp::tests::a2a_rejects_missing_version_and_authentication ... ok
test read_mcp::tests::mcp_edit_concurrent_creates_never_replace_an_existing_destination ... ok
test read_mcp::tests::mcp_edit_rejects_conflicts_ambiguous_replacements_and_overwrites ... ok
test read_mcp::tests::modern_discovery_and_stateless_requests_are_supported ... ok
test read_mcp::tests::file_read_accepts_files_above_legacy_limit_but_enforces_new_limit ... ok
test read_mcp::tests::modern_request_headers_and_versions_are_validated ... ok
test read_mcp::tests::mcp_edit_refuses_redacted_readonly_and_oversized_files_and_overlapping_matches ... ok
test read_mcp::tests::review_diff_matches_integrated_hash_and_does_not_change_index ... ok

test result: ok. 20 passed; 0 failed; 0 ignored; 0 measured; 38 filtered out; finished in 0.86s
```

## Original final bundle output excerpt

```text
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 16.96s
    Bundling Localoud AI.app (target/debug/bundle/macos/Localoud AI.app)
        Warn Skipping signing due to --no-sign flag.
    Finished 1 bundle at:
```
