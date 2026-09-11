# Localoud Desktop UI Design

## Direction

Localoud is a local-first workbench for running, inspecting, and reviewing AI tasks. The main job of the window is to keep the selected project, task state, evidence, and next action visible without forcing the user through extra panels.

The visual direction is **quiet macOS utility**: a compact system toolbar, a tinted navigation sidebar, dense but readable records, and one clear composer at the bottom. The interface uses the shape language of a native Mac work window without pretending that the embedded ChatGPT surface is part of Localoud.

The one deliberate risk is the **status rail**: task stages are rendered as a thin, low-contrast rail instead of a large progress card. It keeps the workflow visible while leaving most of the window for the work itself.

## Design tokens

```css
--ui-canvas: #f5f5f7;       /* window background */
--ui-surface: #ffffff;      /* content and composer */
--ui-sidebar: #f1f1f4;      /* navigation plane */
--ui-sidebar-active: #e5efff;
--ui-text: #1d1d1f;
--ui-muted: #62626a;
--ui-line: #d8d8dc;
--ui-border-strong: #81818a;
--ui-accent: #0067d9;       /* contrast-safe light primary */
--ui-success: #1f7a35;
--ui-warning: #9a6700;
--ui-danger: #d70015;
--ui-radius-sm: 6px;
--ui-radius-md: 9px;
--ui-radius-lg: 12px;
```

Typography is system-first: `-apple-system, BlinkMacSystemFont, "SF Pro Text", "Helvetica Neue", sans-serif`. At the default root size, reading text is 14px with a 1.6 line-height; utility labels are at least 12px. Typography uses `rem` so text enlargement can reflow. Narrow-window input and reading text use 16px. Monospace is reserved for paths, IDs, diffs, and saved command output.

Spacing follows a compact 4/8 rhythm with optical adjustments. The sidebar is 234px by default and can still be resized. The toolbar is at least 64px; the tab rail is at least 44px. The composer has a 72px minimum writing area. Short windows scroll vertically instead of compressing text into overlapping controls.

## Window anatomy

```text
┌─────────────────────────────────────────────────────────────────────────┐
│  Localoud        project / task title              ChatGPT   status       │  toolbar
├───────────────┬─────────────────────────────────────────────────────────┤
│  New task      │  task rail: goal · stage · next action                  │  state
│  search        ├─────────────────────────────────────────────────────────┤
│               │  概要  計画  変更  担当  ログ  参照情報  使用量          │  tabs
│  projects      │                                                         │
│   └ tasks      │  selected task record / diff / worker output            │  content
│               │                                                         │
│  settings      ├─────────────────────────────────────────────────────────┤
│                │  request composer                         send          │  composer
└────────────────┴─────────────────────────────────────────────────────────┘
```

## Copy and interaction rules

- Keep one verb per action: `送信`, `停止`, `再開`, `確認`, `保存`.
- Keep state labels short: `実行中`, `レビュー待ち`, `完了`, `状態確認が必要`.
- Put explanation in a disclosure or tooltip, not in the default path of the task.
- Keep the model name in the compact picker; hide repeated “実行先” helper prose unless a warning needs attention.
- The project path remains available as a compact secondary line and a copy action, because it disambiguates same-named folders.
- Every icon-only control keeps an accessible label and a visible keyboard focus ring.
- `⌘N`, `⌘K`, `⌘B`, `⌘,`, `⌘⇧B`, and `⌘Enter` stay discoverable in the interface.

## Codex-like interaction audit

| Area | Localoud adjustment | Reason |
| --- | --- | --- |
| Task creation | Keep `⌘N` and focus the composer immediately | Matches the fast “new task” loop without opening an extra screen |
| Command search | Use the existing `⌘K` command palette, but render it as a compact native sheet | Same muscle memory as Codex; less visual weight |
| Project navigation | Keep a resizable sidebar, active row tint, task filter, and pinned tasks | Matches the predictable project/task hierarchy of Codex-style workspaces |
| Main toolbar | Keep project, path, ChatGPT handoff, state and sidebar toggle together; reflow on narrow windows | Keeps project identity and navigation reachable |
| Tabs | Use a small segmented tab rail with one active underline/background | Keeps Overview/Plan/Diff/Logs available without nested navigation |
| Composer | Keep the request first; move model, reasoning, routing, and file scope into compact controls | Codex-like “write first, tune when needed” flow |
| Progress | Keep stage and next action in one thin rail, not a large card | Preserves evidence while reducing ceremony |
| Review and diff | Keep colored additions/removals, copy actions, and disclosures; avoid adding stage/rollback buttons without a safe data contract | Visual parity must not imply unsupported operations |
| Embedded ChatGPT | Keep it in a separate sheet/overlay with an explicit `作業に戻る` action | Clear boundary between consultation and local execution |

## Accessibility and motion

- Primary text targets at least 4.5:1 contrast in light and dark themes.
- Focus uses a 2px system-blue ring with an offset; it is never removed to make the UI look cleaner.
- Desktop controls use compact 24–32px minimum targets. Narrow/coarse-pointer buttons, inputs and summaries have a 44 CSS-pixel minimum target; native platform points are not inferred from this browser measurement.
- The sidebar toggle stays in the toolbar. At 650px and below, navigation becomes a dismissible drawer with focus containment, background inertness and `Esc` return to its trigger.
- The skip link comes before navigation. Arrow/Home/End keys move between tabs without changing panels; Enter/Space activates, and Tab reaches the selected panel.
- An empty or pending completion never traps Tab in the composer. A ready suggestion may be accepted with Tab.
- The transcript is not a live region. Status/error regions announce concise changes; controls retain focus across updates.
- Reads distinguish loading, confirmed empty data and failure. Failed refreshes retain the previous data and offer a task-scoped retry. Archive synchronization is not blindly repeated after an uncertain result.
- Invalid fields have individual descriptions and a linked error summary. Settings cannot save until loaded; duplicate submits are blocked. History deletion has an explicit second confirmation naming the task.
- Hover and pressed states do not move layout. Motion is limited to 120–180ms color/elevation transitions and is disabled under `prefers-reduced-motion`.
- Dark mode maps the same semantic tokens rather than reusing light-mode gray values.
- Dark danger text uses `#ff6961`; composed diff/error backgrounds were checked independently of light mode.
- Incoming stream events are coalesced per animation frame. Unchanged project-tree markup is retained; history replay does not redraw for each record.

## Reference material

- [Designing for macOS — Apple Human Interface Guidelines](https://developer.apple.com/design/human-interface-guidelines/designing-for-macos/)
- [Sidebars — Apple Human Interface Guidelines](https://developer.apple.com/design/human-interface-guidelines/sidebars)
- [Toolbars — Apple Human Interface Guidelines](https://developer.apple.com/design/human-interface-guidelines/toolbars)
- [Focus and selection — Apple Human Interface Guidelines](https://developer.apple.com/design/human-interface-guidelines/focus-and-selection/)
- [Keyboards — Apple Human Interface Guidelines](https://developer.apple.com/design/human-interface-guidelines/keyboards)
- Local catalog results from `ui-ux-pro-max`: Minimalism / Swiss Style, high density, subtle motion, system-sans typography, and visible keyboard focus.

## Acceptance checklist

- [x] The default task screen reads as one Mac work window, not a marketing page.
- [x] The selected task and its next action are visible without scanning multiple cards.
- [x] The composer is the strongest action and secondary controls are quiet.
- [x] The checked sidebar, toolbar, tabs, content, dialogs and browser shell share theme tokens.
- [x] Japanese labels remain short at the default desktop width and reflow with enlarged text.
- [x] Browser checks cover light/dark themes, keyboard focus, reduced motion, 375px portrait, 812px landscape and 200% text.
- [ ] Tauri WKWebView, native ChatGPT content, VoiceOver and actual OS-level text scaling require separate device verification.

See [the scoped UI audit](docs/ux/2026-09-11-ui-audit.md) for evidence, remaining limits and reproduction.
