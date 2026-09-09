# コンテキスト比較の状態

2026-09-09追記: 単一workerについては[実際のCodex native subagentとの比較](NATIVE_AGENT_BENCHMARK.md)を実施済み。以下の計画・レビューを含むライブ比較とは別の実験で、Localoudの累計入力削減は確認できなかった。

2026-09-08: ChatGPTを内蔵WebViewの手動操作と読み取り専用MCPへ移行したため、旧ブラウザ連携を前提とした比較手順は廃止しました。旧方式の試行はworker開始前に停止しており、比較の成功結果ではありません。

新方式でMCP計画 → Task Manifest → 独立worker → 統合・検証 → MCPレビュー → 修正・再レビューを実機で完走した後、比較実験を再設計します。現在のライブ比較は未完了です。

実装と操作は [CHATGPT.md](CHATGPT.md) を参照してください。`research_benchmark` の過去のfork-control用ハーネスは実験資料として残していますが、新方式の完走・ネイティブsubagent比較を検証するものではありません。

使用量は入力・キャッシュ・非キャッシュ入力・出力を分けます。ChatGPTで取得できない使用量はnullとし、workerのtokenから全ワークフローやサブスクリプション利用枠の削減率を推定しません。
