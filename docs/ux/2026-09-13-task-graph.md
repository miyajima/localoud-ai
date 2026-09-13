# Task graph implementation and scoped review

This extends the existing quiet macOS utility UI. The graph uses saved task dependencies; it is not a new scheduler. Review was performed in this parent task under the repository's no-subagent policy.

## Runtime contract

`autonomous.rs::parse_plan` topologically orders steps and adds serial dependencies for overlapping owned paths. `ready_batch` selects at most three eligible workers; the runner drains the whole batch before dispatching the next. Provider capacity may further delay work. Manifest parsing now returns the normalized steps so the confirmation view and executed plan agree.

The same graph is shown before manifest confirmation and in the running task's Overview and Plan. Nodes expose full titles, goals, dependency wait reasons, paths, and saved models. Actual states, progress totals, and errors come from the existing snapshot/polling path. User selection and horizontal scroll persist across refreshes. Dependency layers indicate possible parallel work, not guaranteed execution batches. Direct single-conversation requests have no invented DAG.

## Verification

- Frontend build passed.
- Ten targeted frontend tests passed: fork/join, invalid graphs, status semantics, escaping, node selection, clipboard confirmation, and frozen model overrides.
- Four Rust manifest tests passed, including equality between preview dependencies and the execution parser for overlapping ownership.
- Browser fixture intercepts every native command and blocks external network access. It makes no model calls or repository changes.
- Seven inspected views: Overview light/dark, narrow dark (390 px), 200% text, Plan, desktop/narrow manifest preview. Automated WCAG checks, minimum text-size checks, and page overflow checks passed. Polling retained the selected join node; preview did not import or dispatch.
- Evidence: `/private/tmp/localoud-task-graph-ui-final/graph-report.json` and adjacent screenshots. Component captures use a taller viewport at the same width to include content normally inside the window's scroll pane.

## Finishing review

Disposition: fix (one scoped layout adjustment before final handoff).

- Persistence: existing DESIGN.md and native-ui.css remain authoritative; no new visual world. PRODUCT.md is absent; no context documents were fabricated. Impeccable's launcher was not executable, so context and craft checks were read directly; its detector did not run.
- Fidelity: the graph, execution states, actual dependency edges, task details, confirmation gate, and refresh persistence match the request. Semantic colors, system typography, native buttons, and focus treatment match the incumbent system.
- Ceiling: real Tauri WKWebView, VoiceOver, and a real multi-worker model run remain unverified. Browser fixtures are not native execution evidence.
- Material fix: the existing generic dialog maximum width constrains desktop preview to about 510 px, hiding a three-column graph despite available screen space. Override the task-preview width within the viewport, then confirm only that layout and its narrow-window fallback.
- Keep: exact scheduler dependencies, text state labels, bounded horizontal scrolling, and frozen confirmation behavior.

## Design documentation check

No changes to DESIGN.md. Checked task-graph.css, workflow-ui.css, and native-ui.css against it. Existing neutral surfaces and blue action/state accent are reused; completed states use success and errors use danger. System text remains 12–15 px with rem scaling. The graph adds no new global tokens, typefaces, animations, or brand rules. Missing PRODUCT.md and the older documentation format were not repaired as a side effect.

## Final verdict

Disposition: ship, scoped to the reviewed graph UI and listed layout fix.

- Resolved: desktop task preview is now 900 px wide within the available viewport, and the three dependency columns fit without horizontal scrolling. The 390 px fallback remains within the viewport and scrolls the diagram internally. Both recaptures passed accessibility and overflow checks (`preview-verdict.json`).
- Remaining: no open fix in the reviewed scope. Native WKWebView/VoiceOver and live model execution remain unverified as described above; no deployed or installed-app claim is made.

Reproduce the isolated browser check from `apps/desktop` with `node tests/task-graph-ui.mjs`; the default creates a temporary evidence directory. `GRAPH_PREVIEW_ONLY=1` limits the check to the reviewed dialog fix. No additional build polishing was performed after this verdict.
