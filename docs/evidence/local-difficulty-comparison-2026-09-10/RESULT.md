# Localoud 難易度判定プローブ

2026-09-10 に、同じ Localoud の `LocalModelProvider::assess_difficulty` 経路から、OpenAI 互換の `/v1/chat/completions` を使う 2 つの Q8_0 GGUF を比較した。各モデルに同じ固定 10 ケース（難易度 1〜5 を各 2 ケース）を順番に送り、モデル照合、量子化検出、JSON 変換、`DifficultyAssessment::validate`、推論時間を記録した。

## 結果

| モデル | 契約 valid | 難易度 exact | ±1（JSON 解析済み） | risk 一致 | valid の平均 latency |
|---|---:|---:|---:|---:|---:|
| Spark-X2.5 4B Q8_0 | 8/10 | 7/10 | 10/10 | 8/10 | 2,128.4 ms |
| MiniCPM5 2B Q8_0 | 5/10 | 5/10 | 8/10 | 7/10 | 1,255.8 ms |

`契約 valid` は JSON を `DifficultyAssessment` に変換でき、`validate()`（確信度 0.75 以上を含む）を通過したケースである。`難易度 exact` はその valid ケースのうち期待レベルと一致した数である。`±1` は数値を返したケースを対象にした補助指標で、契約 invalid の出力も含む。平均 latency は valid ケースだけの算術平均で、並列化せず、各モデル 1 回の測定である。

Spark は 10 ケースすべてで難易度の数値を返し、8 件が契約を通過した。失敗した 2 件は不確実性を正しく低く出したため `validate()` が拒否した。MiniCPM は短いケースでは速く、レベル 1〜2 は全件 exact だった。一方で、レベル 3 の 1 件は前置き付き JSON を返して変換に失敗し、認証監査は `estimated_scope.loc: null` で型変換に失敗した。レベル 4〜5 では確信度が 0.5〜0.7 に留まり、契約を通過しなかった。

## 再現条件

- プローブ: `cargo run -p provider-spark --example difficulty_probe -- ENDPOINT MODEL_ID DISPLAY_NAME`
- 共通サーバー: `llama-server`、`-ngl 99 -c 16384 -np 1 --jinja --reasoning off --reasoning-budget 0`
- Spark endpoint: `http://127.0.0.1:18085`、model ID `spark-x2.5-q8`
- MiniCPM endpoint: `http://127.0.0.1:18088`、model ID `minicpm5-2b-q8`
- どちらも `/health` の ready/ok と `/v1/models` の ID・`meta.ftype: Q8_0` を確認した
- 入力と個別応答は [spark-x2.5-q8.json](./spark-x2.5-q8.json) と [minicpm5-q8.json](./minicpm5-q8.json) に保存した

使用した重みは次の固定 revision である。

- [openbmb/MiniCPM5-2B-GGUF](https://huggingface.co/openbmb/MiniCPM5-2B-GGUF/tree/f0c9de8f8e1bbffc5abf14e64ccb61bb99e4219d): `MiniCPM5-2B-Q8_0.gguf`, 2,679,710,688 bytes, SHA-256 `c5415f8989bf88a8288f1b55a3cc371af53c07b0faa220a63bd7a990cfaba078`
- [stornic56/Spark-X2.5-4B-GGUF](https://huggingface.co/stornic56/Spark-X2.5-4B-GGUF/tree/9b8e3e55d502caec60e971f4ac1291827c5b0e37): `Spark-X2.5-4B-Q8_0.gguf`, 4,375,021,056 bytes, SHA-256 `092a263df8c891cdddd98b14b9ed71e44bb84643049fbfe656fb682b71d316c6`

これは固定入力 10 件の単一実行であり、一般的なモデル順位や再現分散を示すベンチマークではない。`±1` や risk 一致は参考値で、実運用の採用には複数回測定と実タスクの受入れ確認が必要である。
