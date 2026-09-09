# Astra計画 → Luna実装 → Astraレビューの実比較

2026-09-09。**小さな固定ケースでは両経路とも最終品質はpass。利用枠削減はinconclusive。**
Localoudは初回で合格し、ネイティブ側はAstraレビューで見つけた不具合をLunaが1回修正した。
総トークンの差を利用料金・サブスクリプション利用枠の削減率とは解釈しない。

## 実行条件と結果

同じAstra/low親が共通計画を作り、それぞれの実装子をLuna/maxで実行し、同じ親がレビューした。
実際のセッション `turn_context` で親・子のモデルとeffortを確認済み。
ネイティブは本環境の `collaboration.spawn_agent`、`fork_turns=1`。
Localoudは実装した `ExecutionPlan → Workers → Sessions → CodexProvider` の独立worker。
`thread/fork` は使っていない。desktop UIの操作比較ではない。

| 指標 | ネイティブ | Localoud |
| --- | ---: | ---: |
| 実装モデル | Luna / max | Luna / max |
| 初回の固定6ケース | pass | pass |
| 初回Astraレビュー | fail | pass |
| Lunaへの修正往復 | 1 | 0 |
| 最終の固定6ケース＋レビュー反例2件 | pass | pass |
| Lunaの推論応答数（修正含む） | 13 | 7 |
| 親＋子の区間内総トークン | 2,137,107 | 1,186,925 |
| 親＋子の区間内非キャッシュ入力 | 56,975 | 54,013 |
| Lunaのみの非キャッシュ入力 | 48,956 | 48,941 |
| 親の最終レビューまでの計測区間 | 225秒 | 139秒 |
| 週次利用枠表示 | 56% → 56% | 56% → 56% |
| 条件に帰属できる利用枠消費 | 不明 | 不明 |

時刻にはツール待ち、許可処理、親の検証と記録を含む。子だけの実行速度ではない。
どちらも変更は `slug.py` だけで、各初期HEADは維持。条件は別フォルダ・別Git repositoryで、
初期ファイルのSHA-256は一致した。Localoudはそのrepositoryからさらに独立worktreeを作成した。
検証出力・差分・レビューも条件別に保存し、共有書き込みはない。

ネイティブは全出典を含むbriefファイルを読み、Localoudは追加LLMなしの決定的抽出capsuleを
受け取った。どちらも同じ目的、仕様、制約、訂正、終了条件を与えたが、渡し方と起動経路は異なる。
履歴抽出だけの因果効果を分離した実験ではない。逐次実行の順序も固定で、各条件1ケースの校正。

## レビューが防いだ不具合

仕様は「ASCII大文字だけを小文字化し、ASCII英小文字・数字以外を区切りにする」。
ネイティブ初版の `text.lower()` はUnicode文字も変換するため、`İABC → i-abc`、
`K → k` になった。本来は `abc` と空文字である。
固定6ケースだけでは見逃したが、Astraのコードレビューで検出した。

初版・失敗レビュー・検証結果を保持し、同じLuna子へ1点だけ修正依頼した。
新しい子を起動したり、fixtureを作り直したりしていない。修正後のASCII限定 `translate` を
Astraが再レビューし、固定ケースと反例が通過した。Localoudは初版からASCII範囲を明示判定し、
同じ反例も通った。Localoudにはネイティブの失敗や修正コードを渡していない。

これは今回の観測であり、Lunaの品質やLocaloudの優位性を一般化する結果ではない。

## 利用量の計測範囲と判定限界

上表の親＋子は、各条件開始からAstraの最終レビューまでのモデル応答を合算した。
親の委任、待機、検証、レビュー、ネイティブの修正往復を含む。
Localoudの抽出はこの区間で実worker経路が実行し、追加LLMは0回。

共通計画と実装基盤の準備は同じ親の別区間で **1,332,131総トークン、23,914非キャッシュ入力**。
両条件に共通の準備として別記し、無料扱いしていない。この区間にはモデル指定の実装作業も
含まれるため、通常運用時の計画コストだけには分解できない。
実行後の集計・報告作業もreport.jsonのcheckpointに別記している。最終回答分は未確定。
プラットフォームの関連する承認検査の使用量も別記し、その利用枠への課金有無は不明とした。

`token_usage_record` のresponse IDを一度ずつ集計し、所有thread IDを照合した。
累積 `token_count` を再加算せず、子の合計を最後の累積値と照合した。
reasoningはoutputの内数なので二重加算しない。Localoudのprovider使用量イベントも整合した。

利用枠APIは両区間とも56%で、観測値は整数。精度・丸め規則・更新遅延は不明。
さらに両区間に別タスク由来の使用量イベントがある。通常作業は停止していない。
したがって表示差0は消費ゼロを意味せず、区間内トークンが少ないことから
サブスクリプション利用枠が減りにくくなったとも断定できない。

## Localoudへの変更

`WorkerBrief.execution` に工程のモデルを明示できる。

```json
{
  "execution": {
    "model": "gpt-5.6-luna",
    "reasoning": "max"
  },
  "handoff": { "sources": [], "groups": [], "outcomes": [], "max_bytes": 6000 }
}
```

上はフィールド説明用で、空のhandoffは必須区分検査により拒否される。
実行可能な完全入力は [plan.json](evidence/astra-luna-route-comparison-2026-09-09/plan.json)。
親の計画・レビューはAstra/lowに保ち、実装workerだけを上のモデルへ固定する。

providerのモデル一覧で組み合わせを検証し、`thread/start` の返却モデルが一致しなければ
指示を送らず停止する。モデルとreasoningをDBに保存し、`turn/start` へ反映する。
再開時にbriefと保存値が異なれば停止し、別モデルへフォールバックしない。
フィールド未指定の既存計画は従来経路を維持する。

ローカルテストではモデル・effortの送信、dynamic toolの保持、DBへの固定、再開時の不一致拒否を
偽app-serverで確認。関連テスト23件、examplesのcargo check、diff checkが通過した。
実接続の事前確認はモデル一覧のみで行い、Luna/maxの利用可能性を確認してから起動した。
初回は制限環境でapp-serverが切断し、通常の許可経路で事前確認を完了した。実装の再試行ではない。

実験起動用exampleは `handoff_worker_benchmark`。設定変更・デプロイ・購入・利用枠リセットは
行っていない。元からあった未コミット変更と過去の不成立実験は保持した。

## 証拠

- [使用量・実モデル・帰属限界](evidence/astra-luna-route-comparison-2026-09-09/report.json)
- [固定初期状態](evidence/astra-luna-route-comparison-2026-09-09/freeze.json)
- [ネイティブ初版の失敗レビュー](evidence/astra-luna-route-comparison-2026-09-09/native/review-initial.json)
- [ネイティブ最終レビュー](evidence/astra-luna-route-comparison-2026-09-09/native/review-final.json)
- [Localoud最終レビュー](evidence/astra-luna-route-comparison-2026-09-09/localoud/review-final.json)

生のprovider journalとDBは `/private/tmp/localoud-astra-luna-compare-b6af-20260909/localoud/evidence`
に保持し、会話全文はこの証拠ディレクトリへ複製していない。
