# Qwen3.5-4B MTPLX Qualityの同一課題再実行

2026-09-10 JST。ユーザー指定の18094番サーバー、要求・応答モデルID `qwen35-mtplx-quality`。**完成した生成コードは同じテストに不合格。修正やLuna呼び出しは行っていない。**

| 試行 | 結果 | 入力 | 出力 | 合計 | 時間 |
|---|---|---:|---:|---:|---:|
| 初回 | 1024で打ち切り、JSON未完了 | 392 | 1,024 | 1,416 | 8.66秒 |
| 上限修正後 | 正常終了、テスト不合格 | 392 | 2,503 | 2,895 | 23.10秒 |

初回はリクエストのmax_tokens=4096に対し、サーバーのmax_response_tokens=1024が設定されており、finish_reason=lengthで終了した。APIでサーバー上限だけを4096へ一時変更し、同一プロンプトを再送した。再実行はfinish_reason=stop。終了後、上限を1024へ戻したことを読み取り確認済み。初回応答・使用量は `../qwen35-local/` に残している。両試行合計は4,311ローカルトークン、生成時間合計31.76秒（設定変更・テスト時間を含まない）。

完成版は6テスト群で8 failures・1 error（subtestを含む）。入力検証、区間差分、空き時間探索が契約を満たさない。normalizeはNoneなどを空集合扱いし、座標のboolを整数として受け入れる。subtractは複数の除外区間を正しく処理できず、earliest_slotは探索範囲の切り詰めが誤っている。ソースレビューもfail。テストの250ケース照合群は途中のassertで停止しており、250件を完走した結果ではない。

比較用の前回結果: Spark-Xは36.33秒・2,169トークンで不合格、Qwen3.8-27Bは33.67秒・1,555トークンで全合格。1課題・少数試行であり一般的なモデル順位ではない。トークナイザー・サーバー・ウォームアップ状態の違いにも注意。

プロンプトと要求生成設定は前回Spark-X要求をコピーし、modelだけを変更。temperature=0、max_tokens=4096、chat_template_kwargs.enable_thinking=false。サーバー設定ではreasoning=off、モデル参照はYoussofal/Qwen3.5-4B-MTPLX-Optimized-Quality、revision `28fefcaf858a6176dcf536163d4cd3c25e48c7b8`。8bitはユーザーの指定で、checkpointの量子化内容を独立検査してはいない。

証拠: [要求](request.json)、[応答](response.json)、[使用量](metrics.json)、[無修正コード](intervals.py)、[テスト失敗](tests.txt)、[初回使用量](../qwen35-local/metrics.json)、[変更前設定](../qwen35-local/settings-before.json)、[復元後設定](../qwen35-local/settings-restored.json)。
