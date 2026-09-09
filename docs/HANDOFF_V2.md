# 出典付き引き継ぎとネイティブsubagent比較

2026-09-09。実装・モデルを呼ばない接続検証は完了。当初のネイティブ3条件は
モデル制約から実行せず、その後ユーザー指定のAstra → Luna → Astraに揃えて
[ネイティブとLocaloudの実比較](ASTRA_LUNA_ROUTE_COMPARISON.md)を実施した。
利用枠削減の判定は **inconclusive**。

## 実装と適用範囲

`WorkerBrief.handoff` を指定した `ExecutionPlan → run_plan → Workers` に適用する。
`hub-context::handoff` が原文、出典ID、発言者、参照先、訂正・依存関係を検証して
`ContextItem` にし、既存の永続capsuleから実workerの `turn/start` に送る。
この経路ではローカルモデルの `draft_context` を呼ばない。引き継ぎの選択に
追加LLM呼び出しはない。worker終了後の既存memory captureは別機能として残る。

新フィールドを省略した既存計画は従来どおり動作する。新しい抽出は通常チャット、
ネイティブ `spawn_agent`、research capsule、desktop autonomous workflowには
自動適用されない。ネイティブ向けには下記のオフラインexporterを利用できる。
アプリの設定変更、デプロイ、親モデル変更は行っていない。

```sh
cargo run --quiet -p hub-runtime --example prepare_handoff -- execution-plan.json
```

入力例は [plan.json](evidence/handoff-v2-2026-09-09/plan.json)。入力元の役割・順序・
参照と分類は呼び出し側が明示する。全文会話から分類を推測するLLM要約器ではない。
`sources` は時系列順の原文ブロック、`groups` は目的・制約・決定・現在状態・
未解決点・終了条件と任意の背景情報。情報が不明な区分にも「不明」の出典を渡す。
必須事項を `background` と誤分類していないかは呼び出し側のレビューが必要。

- 6区分を必須とし、訂正と依存先を閉包として丸ごと保持する。訂正前も消さず、
  訂正は後のユーザー発言からのみ関連付ける。未明示の意味上の矛盾は解決しない。
- 原文ブロックを分割・書き換えない。条件、禁止、例外、コード、URLを同じブロックに
  保持する。関連する別ブロックは `depends_on` / `corrects` で結ぶ。
- 必須閉包がバイト予算に収まらなければ失敗する。任意背景の省略IDは出力する。
  capsule全体も既存の推定トークン予算で検査し、worktree作成・provider呼び出し前に止める。
  推定は既存のUTF-8 bytes/3で、provider tokenizerや課金量ではない。
- 出典ID重複・未解決参照・発言者不一致を拒否。`verified_outcome` は同じcall IDの
  completed / exit 0を持つtool出典だけを許す。これはコマンド終了の証拠であり、
  意味上の品質合格ではない。参照先ファイルの存在・真正性を自動認証する機能はない。
- 古いユーザー発言・tool結果はhistorical evidenceとして渡す。実行権限を生成せず、
  現在のgoal / constraints / acceptanceと既存sandboxが実行を制御する。

## OrgBrainからの適応

読み取り専用で指定worktreeの `splitCoverageSentences`、
`buildCoverageEvidenceGroups`、`selectCoverageEvidence`、`packCoverageGroups`、
verifier、provider contract、review signalsを確認した。
検証時のファイルSHA-256は [report.json](evidence/handoff-v2-2026-09-09/report.json) に保存。

原文と出典の対応、依存グループの不可分選択、予算検査、ユーザーへの帰属、
実行結果のcall ID照合をRustへ最小限に適応した。文境界の推測はコードを壊す可能性が
あるため原文ブロック単位にした。長期記憶の一時情報除外、最大3候補、LLMの2回目呼び出し、
quarantineや正式memoryへの保存は移植していない。

OrgBrain最新 `a-plus/v1` run9は合成8件、人手ラベルなし、`evaluation_incomplete`。
そのprecision/recallを本機能の品質実績として流用しない。

## 検証

`cargo test -p hub-context -p hub-runtime --lib --tests` の既存・抽出テスト20件に加え、
`cargo test -p hub-runtime --test handoff` の接続テスト2件を確認した。
接続テストは一時Gitリポジトリと偽app-serverを使い、計画からDB保存したcapsuleが
実際のworker送信プロンプトに含まれることを検証する。外部モデル・ネットワークは使わない。
初回の接続テストではfixtureのplan行不足によるFKエラーがあり、実経路同様にplanを保存して修正した。
レビューで不正な訂正先を参照した場合のpanicを発見し、全出典を先に検査するよう修正した。
回帰テストを追加し、抽出テスト5件が通過した（既存分・接続テストを含む計23件）。

3ターン訂正、条件・禁止・例外とコードのバイト保持、必須予算超過、任意背景の省略、
依存先の保持、出典・発言者・call IDの誤り、全capsule予算、旧JSON互換を検証する。
合成ケースによる機械的契約検証であり、人手による要点抽出品質の正式合格ではない。

## 比較準備と利用枠の限界

固定ケースはslugifyの小修正。`scripts/prepare-handoff-comparison.py` は新規ディレクトリ
だけに同一初期状態の `all/fixture`、`one/fixture`、`extracted-none/fixture` を作る。
各条件に独立した `evidence` を置く。既存runへの上書きを拒否する。
今回の準備先は `/private/tmp/localoud-handoff-v2-b6af-20260909`。
検証実行は `python3 -B acceptance.py <condition>/fixture` とし、bytecodeを作らない。
初期ファイル、受け入れ検証、計画のhashは
[freeze.json](evidence/handoff-v2-2026-09-09/freeze.json) に固定済み。

共通原文の文字列は18,097 UTF-8 bytes、抽出JSONは2,413 bytes。
7出典を保持し、無関係な背景g7だけを省略した。これは合成入力のサイズ測定であり、
ネイティブ入力トークンや利用枠の削減率ではない。

現在の親は実セッション `turn_context` で `gpt-6-astra / low`。
`all` はモデル・effort上書きを許さず親を継承する。元指定の実装 `gpt-5.6-luna / max`
と両立しないため、勝手に `all` を `1` に変えたり、モデルを変更したりしていない。
このため当初の3条件は実行しなかった。その後、ユーザーの目的に合わせ、親Astra/low・
実装Luna/maxを維持した2経路の比較に変更した。以降の記述とこのページのreport.jsonは
当初の準備時点の記録で、実比較の結果は上記リンクを参照。

利用枠APIを2回読んだ値は週次55%→55%。reset時刻は1789435445→1789435446と1秒変化。
APIの型は浮動小数だが今回観測は整数値だけで、丸め精度と更新遅延は不明。
自タスクの `token_count` イベントにもaccount-wide rate_limitsがあり、thread token使用量と
別物であることを確認した。Localoud providerの `thread/tokenUsage/updated` は入力・cache・
出力トークンを保持するが、タスク別利用枠の消費量には変換できない。
55%→55%は実装・準備中の観測で、条件別測定ではない。差0を消費ゼロと解釈しない。

次の実行でも、親の抽出・委任、子の作業、親の検証・レビュー・再作業をすべて範囲に含める。
抽出時間と親の推論も無料扱いしない。トークン記録は補助指標として、継承イベントの重複を除く。
条件開始前とレビュー完了後のusage snapshot、model/effort、thread ID、実行結果と
artifact hashを条件別evidenceに保存する。全条件の品質・scope合格が必要。

他タスク・別端末の消費を除外できない、resetや丸め・更新遅延が混入する、同じ親の
逐次実行で履歴量・cache状態が変わる、条件別の親使用量を分離できない場合は
**inconclusive**。通常作業を止めて測定環境を作ることはしない。
初回各1ケースを校正に留め、不鮮明な差を得るために大量実行や無断再試行を行わない。
`thread/fork` やLocaloud独立workerをネイティブ `spawn_agent` の代替とはしない。

同じ親による最終レビュー: ローカル契約と接続範囲は **pass**。意味上の抽出品質と
利用枠改善の評価は **inconclusive**。既存の未コミット作業は保持している。
