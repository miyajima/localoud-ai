# Codex / Localoud 手動 UI 実行比較（2026-09-09）

判定: **inconclusive_shared_workspace**。観測された総トークンは Localoud が
34.1% 少ない。ただし同一作業フォルダへの同時書き込みがあり、公平な性能比較には使えない。

## 実測値

両方とも親は `gpt-6-astra / low`、実装子は `gpt-5.6-luna / max`。
セッションログの実モデルと reasoning effort を確認した。

| 指標 | Codex Desktop | Localoud |
| --- | ---: | ---: |
| 親の総トークン | 811,256 | 560,561 |
| 実装子の総トークン | 521,695 | 317,890 |
| 親＋子の総トークン | **1,332,951** | **878,451** |
| 入力（キャッシュ込み） | 1,321,111 | 870,082 |
| キャッシュ済み入力 | 1,232,640 | 773,760 |
| 非キャッシュ入力 | 88,471 | 96,322 |
| 出力 | 11,840 | 8,369 |
| 推論回数 | 26 | 21 |

非キャッシュ入力は Localoud が 8.9% 多い。総トークンの差を、そのまま課金や
サブスクリプション使用枠の差として解釈しない。

集計範囲は今回作成された各親セッションとその実装子。親には既存計画の確認、
委任、待機、検証、レビューを含む。以前の共通計画生成 97,149 トークンと
内部 guardian は含めていない。子に継承された過去イベントを除き、各セッションの
今回の使用量を一度ずつ集計した。

## 比較を成立させられない理由

両実装子が `/private/tmp/localoud-codex-ui-df8o58_6/fixture` を同時に更新した。
ログ上の書き込み要求時刻（UTC）は以下。

- 07:27:17.670: Codex 子が `scheduler.py` と `test_scheduler.py` を追加。
- 07:27:37.260: Localoud 子が同じ `scheduler.py` を追加。独自テストの追加はなく、
  既存の Codex 側テストを使用。

最終状態は Localoud 側の実装と Codex 側のテストの組み合わせになった。
両親の検証・レビューも同じ成果物を読み、`validation.json` と `review.json` を
同じ場所に書いた。両方がテスト成功を報告していても、独立した二候補の品質合格とは扱えない。
比較用の案内で作業フォルダを分離していなかったことが原因である。

## 実行経路と前回との差

今回の Localoud は通常の Codex チャットからネイティブ子エージェントを起動した。
Localoud の DB 上でも `provider=codex`、`task_id=null` の通常チャットであり、
前回測定した Localoud の独立 worker 経路ではない。
両側とも `collaboration.spawn_agent`、`fork_turns=1` を使用していた。

両側の親・子とも Codex バージョンは `0.153.4`。
今回の Codex 子の初回入力は 39,781、推論回数は 10 回。
前回 Luna 比較の 39,620、9 回に対して、UI 経由で無駄が解消されたとは確認できない。
今回と前回は実行条件が異なるため、この差も UI の因果効果とは扱わない。

再比較する場合は、凍結した同一初期状態を二つの別フォルダへ展開し、検証・レビュー出力も
分離する必要がある。また通常チャット同士の比較か、ネイティブ子と独立 worker の比較かを
固定する。今回の観測値は削除せず、判定保留のまま保存する。

## 証拠

- [集計・モデル・実行経路・書き込み時刻](evidence/manual-ui-benchmark-2026-09-09/report.json)
- [Codex 親使用量](evidence/manual-ui-benchmark-2026-09-09/codex_parent-usage.json)
- [Codex 子使用量](evidence/manual-ui-benchmark-2026-09-09/codex_child-usage.json)
- [Localoud 親使用量](evidence/manual-ui-benchmark-2026-09-09/localoud_parent-usage.json)
- [Localoud 子使用量](evidence/manual-ui-benchmark-2026-09-09/localoud_child-usage.json)
- [共有フォルダの最終検証結果](evidence/manual-ui-benchmark-2026-09-09/shared-final-state/validation.json)

元セッションの識別子・ハッシュを使用量ファイルに記録し、会話全文は複製していない。

## アーカイブ可否の経路比較

性能比較とは分離して、完了済みセッションのアーカイブ操作も比較した。

| セッション | 起動経路 | 事前状態 | アーカイブ結果 |
| --- | --- | --- | --- |
| `01a0850d-8b73-7380-bc34-d9f4fdf52b84` | Codex Desktop UI | `notLoaded` / 最新turn `completed` | 成功。`list_archived_threads` で確認 |
| `01a0850e-9002-7333-b362-717434d37430` | Localoud UI（ローカル履歴） | 完了 | 成功。アーカイブ表示で確認 |
| 同じ `01a0850e-9002-7333-b362-717434d37430` のCodex provider | Localoud normal Codex chat | `notLoaded` / 最新turn `completed` | 失敗。`already has an active writer`（Localoud UIでアーカイブ後も同じ） |

したがって、Localoudのローカル履歴アーカイブ自体は成功するが、Localoudが起動した同じCodex providerセッションのアーカイブだけがアクティブwriterガードで拒否された。Codex Desktop UI起動の完了セッションはprovider側もアーカイブ可能だった。これはアーカイブ・ライフサイクルの差を示すが、writerロックの解放漏れという実装原因を確定する証拠ではない。詳細な呼び出し結果は
[archive-comparison.json](evidence/manual-ui-benchmark-2026-09-09/archive-comparison.json)
に保存した。
