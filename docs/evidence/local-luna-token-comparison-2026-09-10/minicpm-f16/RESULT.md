# MiniCPM5-2B F16の同一課題実行

2026-09-10 JST。`local-llm-launch` スキルの固定revision手順でMiniCPM5-2B F16を取得し、スキル指定のllama.cppで起動した。**生成は完了したが、同じ受け入れテストに不合格。修正やLuna呼び出しは行っていない。**

| 指標 | 結果 |
|---|---:|
| モデル | `minicpm5-2b-f16` |
| エンドポイント | `http://127.0.0.1:18089/v1` |
| 入力トークン | 403（うちキャッシュ2） |
| 出力トークン | 832 |
| 合計トークン | 1,235 |
| 生成時間 | 16.78秒 |
| finish reason | `stop` |
| テスト | 6群中4 failures・1 error |

モデルファイルは `openbmb/MiniCPM5-2B-GGUF@f0c9de8f8e1bbffc5abf14e64ccb61bb99e4219d` の `MiniCPM5-2B-F16.gguf`。サイズ5,039,006,688 bytes、SHA-256 `0ffba3682a853295566b98bc38c0ab755d6b2d91b994bc031ed727a08cfcdf25` を照合した。起動設定は `-ngl 99 -c 16384 -np 1 --jinja --reasoning off --reasoning-budget 0`。ready応答とモデルIDも確認済み。

## 失敗内容

- `normalize` と `earliest_slot` が `bool` を整数として受け入れた。
- `subtract([], exclusions)` で、exclusionsの検証後に空結果を返すべき契約に反した。
- 複数の除外区間を差し引く処理が不完全で、`[(0,10)] - [(2,4),(6,20)]` を正しく分割できなかった。
- 250ケースのoracle検証では、上記の例外で途中停止したため全ケース完走ではない。

応答はJSONではなく ```python フェンスで返ったため、保存済み応答を抽出してテストした。モデルの再呼び出しはしていない。生成本体のusageは [metrics.json](metrics.json)、完全応答は [response.json](response.json)、抽出後コードは [intervals.py](intervals.py)、テストログは [tests.txt](tests.txt) に保存した。

## 直前の小型モデルとの比較

同じ1課題の参考値では、Spark-X 8bitは36.33秒・2,169トークンで不合格、Qwen3.8-27Bは33.67秒・1,555トークンで合格、Qwen3.5-4B Qualityは23.10秒・2,895トークンで不合格、MiniCPM F16は16.78秒・1,235トークンで不合格だった。1課題・各1回なので、実運用全般の判定やモデル順位には使わない。

Qwen3.5の18094番リスナーは実行後に不在、MiniCPMの18089番だけが稼働中である。
