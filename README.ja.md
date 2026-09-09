# Localoud AI

[English](README.md) | 日本語 | [简体中文版](README.zh-CN.md)

LocaloudはLocalとCloudを組み合わせた名前です。Localoud AIはオープンソースのデスクトップ向けコーディングワークスペースです。

ローカル優先のデスクトップ向けコーディングワークスペースです。Rustがライフサイクルと永続化を担当し、TauriがワークスペースUIを提供します。Codex app-serverは分離したGit worktreeでコーディングタスクを実行します。[Spark-X2.5-4B-MLX-8bit](https://huggingface.co/abenzerps/Spark-X2.5-4B-MLX-8bit)はローカルの振り分け、小規模な編集、要約、抽出を担当します。必要に応じてAstraが結果を計画・レビューします。workerは親の会話を暗黙にコピーせず、明示的なコンテキストカプセルから開始します。

## 開発を応援する ☕

Localoud AIは無料のオープンソースソフトウェアです。気に入っていただけたら、[コーヒー1杯分の支援](https://buymeacoffee.com/miyajima)をいただけるとうれしいです。いただいた支援は、開発・モデルの検証・継続的な改善に充てます。支援は任意です。

## Localoudが良い理由

Localoudは、日常的で範囲の決まったコーディング作業を手元で素早く処理し、規模の大きい作業や複雑な作業はCodexとAstraへ明確に引き継げます。選択中のモデルを確認でき、変更を分離したworktreeに収め、タスクの状態をローカルに保持し、変更を適用する前に境界をレビューできます。

## 向いている人

- 振り分け、小規模な編集、要約、抽出をローカル推論で行いたい開発者。
- タスクの範囲、履歴、worktree、レビュー可能な変更を明示的に管理したいチーム。
- 実用的なローカルコーディングモデルを使いたいMacユーザー。現時点の推奨は[Spark-X2.5-4B-MLX-8bit](https://huggingface.co/abenzerps/Spark-X2.5-4B-MLX-8bit)です。

## 主な機能

- Chatで直接タスク、Planで依存関係を持つDAGのディスパッチと再起動、Agentsでレビューと手直し。
- 上限付きpullと出所情報を持つコンテキストカプセル、実測モデルtokenを記録するUsage。
- モデルとReasoningの明示選択、5段階の自動振り分け、差し替え可能なローカルモデル設定。
- CodexタスクのGit worktree分離、ローカル編集の完全一致パス・置換検証、画面で確認できる承認境界。
- プロジェクト、セッション、下書き、計画、使用量、再起動状態をSQLiteに保存し、安全なMarkdown表示と読みやすいdiffを提供。

## 1コマンドでインストール（macOS）

Rust stable、Node 22.22.2以上（または24.15以上 / 26以上）、Xcode Command Line Toolsをインストールした後、チェックアウトしたディレクトリで次の1コマンドを実行します。

```sh
./scripts/install.sh
```

このスクリプトはフロントエンド依存関係をインストールし、署名なしのデバッグ版`Localoud AI.app`を`target/debug/bundle/macos/Localoud AI.app`にビルドします。Sparkのモデル重みはダウンロードしないため、ローカル推論を使う場合は[Sparkサービス](services/spark-mlx/README.ja.md)を別途起動してください。GitHubからcloneしてインストールまでを1行で行う場合は次を実行します。

```sh
git clone https://github.com/miyajima/localoud-ai.git && cd localoud-ai && ./scripts/install.sh
```

## 開発

Rust stable、Node 22.22.2以上（または24.15以上 / 26以上）、macOSではXcode Command Line Toolsが必要です。

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

[Spark-X2.5-4B-MLX-8bit](https://huggingface.co/abenzerps/Spark-X2.5-4B-MLX-8bit)は**8bit**に設定されています。モデル・ランタイムのインストールと、実プロバイダーの実行は別の受け入れ確認です。

[実装状況](docs/IMPLEMENTATION_STATUS.md)と[改訂版計画](docs/ASTRA_CODEX_HUB_PLAN.md)を参照してください。

## デスクトップアプリを実行する

検証済みのデバッグバンドルは`target/debug/bundle/macos/Localoud AI.app`です。これを開いてGitリポジトリを登録し、SettingsでCodexバイナリの絶対パスを設定してください。ローカル実行には[Sparkサービス](services/spark-mlx/README.ja.md)を別途起動します。AstraはSettingsでオプトインできます。OrgBrainの認証情報は任意で、macOS Keychainに保存されます。実際のOrgBrain接続は事前設定されていません。

Chatではタスク、PlanではDAGのディスパッチと再起動、Agentsではレビューと手直し、Contextではカプセル・pull予算とメモリの出所、Usageでは実測したモデルtokenを扱います。`config.example.toml`は説明用で、実際の設定はUI経由でSQLiteに保存されます。レビューが必要なプロバイダー操作は、現在は画面にエラーを表示して停止します。ブランチの統合はユーザーが行います。

[コンテキスト実測ベンチマーク](docs/CONTEXT_BENCHMARK.md)には、実際に行った1組の比較とその限界を記載しています。完全な受け入れ状況と、任意・未検証の連携は[実装状況](docs/IMPLEMENTATION_STATUS.md)にまとめています。

ローカルLLMの実測値は[ローカルLLMコーディング評価](docs/LOCAL_LLM_BENCHMARK.md)（[简体中文版](docs/LOCAL_LLM_BENCHMARK.zh-CN.md)）を参照してください。独自のコーディング専用20問に絞った結果で、reasoning有無でtoken・時間上限が異なるため、平均時間は同一条件の直接比較ではありません。

現時点のイチオシは**[Spark-X2.5-4B-MLX-8bit](https://huggingface.co/abenzerps/Spark-X2.5-4B-MLX-8bit)**です。測定環境は**M5 MacBook Pro（メモリ48GB）**です。16GBメモリのMacBookでも十分動作する見込みですが、16GB環境では未検証です。

## デスクトップUX

プロジェクトフォルダは**⌘O**またはサイドバーの＋で開きます。**⌘N**で新しいタスク、**⌘K**でタスク・プロジェクト・アクションの検索、**⌘,**でSettings、**⌘B**でサイドバーの表示を切り替えます。**⌘Enter**で送信し、Enterは改行に使います。下書き、最後に開いたワークスペース、ピン留めしたタスクはローカルに復元されます。

[UX比較](docs/UX_COMPARISON.md)には24項目の差分と、採用または一部採用した19項目の改善をまとめています。ネイティブのフォルダ・ファイル選択、名前変更・検索・ピン留め、安全なMarkdown、読みやすいdiff、計画プレビューなどが含まれます。プロジェクト登録には既存のGitリポジトリが必要です。

モデル名・Reasoningの選択とローカルモデルの差し替えは[MODEL_CONFIGURATION.md](docs/MODEL_CONFIGURATION.md)、5段階の自動振り分けは[AUTO_ROUTING.md](docs/AUTO_ROUTING.md)を参照してください。

入力欄のスラッシュコマンド、メンション、プランモード、履歴補完は[COMPOSER.md](docs/COMPOSER.md)にまとめています。

[ChatGPT接続とセッション管理](docs/CHATGPT.md)には、別ブラウザプロバイダーの設定、利用枠の境界、残っているライブ検証を記載しています。

## ローカル設定と認証情報

`config.example.toml`と`services/spark-mlx/config.yaml`は公開用の例・デフォルトであり、個人の設定ではありません。プロバイダーはアプリから自分で設定してください。ランタイム設定、セッション、下書きはローカルのアプリケーションデータベースに保存されます。OrgBrainの認証情報はmacOS Keychainを使用します。読み取り専用MCPのBearer tokenはmacOS Keychainに保存されます。ChatGPTは専用の永続WKWebView領域を使用します。データベース、ローカル設定、プロバイダー認証情報、MCPのBearer tokenはコミットしないでください。

## ライセンス

Localoud AIは[MIT License](LICENSE)でライセンスされています。サードパーティ依存関係と別途ダウンロードするモデル重みには、それぞれのライセンスが適用されます。
