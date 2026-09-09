# Localoud AI

[日本語版](README.ja.md) | [简体中文版](README.zh-CN.md)

Localoud combines Local + Cloud. Localoud AI is an open-source desktop coding workspace.

Local-first desktop coding workspace. Rust owns lifecycle and persistence; Tauri provides the workspace UI. Codex app-server executes coding tasks in isolated Git worktrees. [Spark-X2.5-4B-MLX-8bit](https://huggingface.co/abenzerps/Spark-X2.5-4B-MLX-8bit) handles local routing, small edits, summaries and extraction; optional Astra plans and reviews the result. Workers start from explicit context capsules, never an implicit copy of the parent conversation.


## Why Localoud

Localoud keeps routine, bounded coding work close to your machine while giving larger or more involved work a clear path to Codex and Astra. You can see which model is selected, keep changes inside an isolated worktree, preserve the task state locally, and review the boundary before a change is applied.

## Who it is for

- Developers who want local inference for routing, small edits, summaries, and extraction.
- Teams that need explicit task scope, durable history, worktree isolation, and reviewable changes.
- Mac users who want a practical local coding model: [Spark-X2.5-4B-MLX-8bit](https://huggingface.co/abenzerps/Spark-X2.5-4B-MLX-8bit) is the current recommendation.

## Main features

- Chat for direct tasks, Plan for dependency-aware DAG dispatch and restart, and Agents for review and rework.
- Context capsules with bounded pulls and provenance, plus Usage records for observed model tokens.
- Explicit model and Reasoning selection, five-level Auto routing, and replaceable local model configuration.
- Git worktree isolation for Codex tasks, exact-path and exact-replacement validation for local edits, and visible approval boundaries.
- Local SQLite persistence for projects, sessions, drafts, plans, usage, and restart state, with Markdown-safe rendering and readable diffs.

## Quick install (macOS)

After installing Rust stable, Node 22.22.2+ (or 24.15+ / 26+), and the Xcode command line tools, run this one command from a checkout:

```sh
./scripts/install.sh
```

The script installs the frontend dependencies and builds an unsigned debug `Localoud AI.app` at `target/debug/bundle/macos/Localoud AI.app`. It does not download the Spark model weights; start the [Spark service](services/spark-mlx/README.md) separately when you want local inference. To clone and install in one shell line:

```sh
git clone https://github.com/miyajima/localoud-ai.git && cd localoud-ai && ./scripts/install.sh
```

## Development

Requires Rust stable, Node 22.22.2+ (or 24.15+ / 26+), Xcode command line tools on macOS.

```sh
cd apps/desktop
npm ci
npm run tauri dev
```

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

[Spark-X2.5-4B-MLX-8bit](https://huggingface.co/abenzerps/Spark-X2.5-4B-MLX-8bit) is configured for **8bit**. Model/runtime installation and real provider execution are separate acceptance checks.

See [implementation status](docs/IMPLEMENTATION_STATUS.md) and [the amended plan](docs/ASTRA_CODEX_HUB_PLAN.md).

## Run the desktop app

The verified debug bundle is `target/debug/bundle/macos/Localoud AI.app`. Open it, register a Git repository, and set the absolute Codex binary path in Settings. Start the [Spark service](services/spark-mlx/README.md) separately for local execution. Astra is opt-in in Settings; OrgBrain credentials are optional and stored through the macOS Keychain. No real OrgBrain connection is preconfigured.

Use Chat for a task, Plan for DAG dispatch/restart, Agents for review/rework, Context for capsule/pull budgets and memory provenance, and Usage for observed model tokens. `config.example.toml` is illustrative; active settings are saved through the UI in SQLite. Review-required provider operations currently stop with a visible error. Branch integration remains a user action.

The [measured context benchmark](docs/CONTEXT_BENCHMARK.md) contains one real paired comparison and its limits. Full acceptance and optional/unverified integrations are listed in [implementation status](docs/IMPLEMENTATION_STATUS.md).

ローカルLLMの実測値は [ローカルLLM コーディング評価](docs/LOCAL_LLM_BENCHMARK.md)（[简体中文版](docs/LOCAL_LLM_BENCHMARK.zh-CN.md)）を参照してください。独自のコーディング専用20問に絞った結果で、reasoning有無でトークン・時間上限が異なるため、平均時間は同一条件の直接比較ではありません。

現時点のイチオシは **[Spark-X2.5-4B-MLX-8bit](https://huggingface.co/abenzerps/Spark-X2.5-4B-MLX-8bit)**。測定環境は **M5 MacBook Pro（メモリ48GB）** です。16GBメモリのMacBookでも十分動作する見込みですが、16GB環境では未検証です。

## Desktop UX

Open a project folder with **⌘O** or the sidebar ＋. **⌘N** starts a new task, **⌘K** searches tasks/projects/actions, **⌘,** opens settings, and **⌘B** toggles the sidebar. Send with **⌘Enter**; Enter inserts a newline. Drafts, the last workspace and pinned tasks are restored locally.

[The UX comparison](docs/UX_COMPARISON.md) lists 24 differences and the 19 adopted or partially adopted improvements, including native folder/file selection, rename/search/pin, safe Markdown, readable diffs and plan previews. Project registration still requires an existing Git repository.

モデル名・Reasoning の選択とローカルモデルの差し替えは [MODEL_CONFIGURATION.md](docs/MODEL_CONFIGURATION.md)、5段階の自動振り分けは [AUTO_ROUTING.md](docs/AUTO_ROUTING.md) を参照してください。

入力欄のスラッシュコマンド、メンション、プランモード、履歴補完は [docs/COMPOSER.md](docs/COMPOSER.md) を参照してください。

See [ChatGPT connection and session management](docs/CHATGPT.md) for the separate browser provider, setup, quota boundaries and remaining live verification.

## Local settings and credentials

`config.example.toml` and `services/spark-mlx/config.yaml` contain public examples/defaults, not personal settings. Configure your own providers through the app. Runtime settings, sessions and drafts stay in the local application database; OrgBrain credentials use the macOS Keychain. The browser bridge pairing token stays in the local database and Chrome extension storage. Do not commit database files, local configuration, provider credentials or browser pairing tokens.

## License

Localoud AI is licensed under the [MIT License](LICENSE). The vendored ChatGPT DOM helpers retain their [original MIT copyright notice](extensions/chatgpt/LICENSE.chat-on-steroids); see [NOTICE](extensions/chatgpt/NOTICE.md). Third-party dependencies and separately downloaded model weights remain subject to their respective licenses.
