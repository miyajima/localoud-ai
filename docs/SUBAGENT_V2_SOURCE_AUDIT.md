# Subagent v2 ソース確認（2026-09-09）

対象は公開 Codex `rust-v0.153.4`、commit
`3d2ee51ca2d5db578f328aa75e20aa22c0197c9a`。
一時 checkout: `/private/tmp/codex-subagent-audit-01534`。
実行済みベンチマークの版番号と一致するが、配布バイナリとのビルド同一性は未検証。
ソースと保存済みログの読み取りによる確認であり、ビルド・再実験は行っていない。

## 判定

確認した起動・履歴選択・待機経路に、無駄なモデル呼び出しを自動反復する不具合は
見つかっていない。総トークンが大きいことだけでは実装上の浪費を証明できない。
履歴継承の選択、共通指示・ツールの量、モデルが選ぶ追加作業・待機回数は改善候補。

## ソースで確認したこと

- [spawn.rs](https://github.com/openai/codex/blob/3d2ee51ca2d5db578f328aa75e20aa22c0197c9a/codex-rs/core/src/tools/handlers/multi_agents_v2/spawn.rs#L290):
  `fork_turns` 未指定は `all`。`none` と正整数指定を処理できる。
- [履歴の選別](https://github.com/openai/codex/blob/3d2ee51ca2d5db578f328aa75e20aa22c0197c9a/codex-rs/core/src/agent/control/spawn.rs#L63):
  通常の履歴項目では system/developer/user と assistant の最終回答を残し、
  reasoning、通常のツール呼び出し・対応結果、エージェント間メッセージ等を除く。
  `call_id` のない独立ツール出力など例外はある。圧縮済み履歴にも別途 sanitization がある。
- [継承処理](https://github.com/openai/codex/blob/3d2ee51ca2d5db578f328aa75e20aa22c0197c9a/codex-rs/core/src/agent/control/spawn.rs#L880):
  last-N に切り詰めた後で不要項目を除く。全履歴 fork は親の参照コンテキストを
  保持してキャッシュ用 prefix の再利用を狙い、部分 fork は初回に文脈を再構築する。
  「履歴を短くすれば常に安くなる」とは言えない。
- [設定継承](https://github.com/openai/codex/blob/3d2ee51ca2d5db578f328aa75e20aa22c0197c9a/codex-rs/core/src/tools/handlers/multi_agents_common.rs#L177):
  親の設定を clone し、基本指示やモデル等を設定する。履歴を `none` にしても、
  指示・ツール等をすべて除いた最小環境になるわけではない。
- [待機](https://github.com/openai/codex/blob/3d2ee51ca2d5db578f328aa75e20aa22c0197c9a/codex-rs/core/src/tools/handlers/multi_agents_v2/wait.rs#L187):
  `tokio::sync::watch` と `timeout_at` で活動通知または期限を待つ。
  待機中に推論を連続実行する処理ではない。ただし待機から戻り、モデルが再び待機を
  選ぶ際には次の推論が発生する。

## 今回の実行との照合

手動 UI 比較の Codex 親ログでは `wait_agent` を6回生成した推論の使用量は以下。
これは待機時間への課金ではなく、待機呼び出しを出力した推論の集計である。

| 指標 | トークン |
| --- | ---: |
| 入力 | 310,270 |
| キャッシュ済み入力 | 308,224 |
| 非キャッシュ入力 | 2,046 |
| 出力 | 359 |

待機指定は最初の3回が10秒、その後3回が60秒。長く待てば推論回数を減らせる可能性が
あるが、ユーザー応答・進捗報告の制約もあるため、全量を削減可能とは扱わない。

実装子は `fork_turns=1` でも初回入力39,781トークン。
これは履歴以外の基本指示・ツール定義等を含む総量であり、今回の観測だけでは
それぞれの内訳や不要分は確定できない。親と子による指示ファイルの再読や、
追加チェック等は実行ログで確認できるが、安全性・品質に必要な作業との切り分けが必要。

使用量の保存先は [手動UI比較](MANUAL_UI_BENCHMARK.md)を参照。
共有フォルダの干渉により、Codex と Localoud の相対的な効率判定は引き続き保留。
