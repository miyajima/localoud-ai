# ローカルMLXサービス（デフォルト: Spark-X2.5-4B-MLX-8bit）

[English](README.md) | 日本語 | [简体中文版](README.zh-CN.md)

固定した`model-revision.txt`のrevisionにあるコミュニティ製MLX 8bit変換版`abenzerps/Spark-X2.5-4B-MLX-8bit`を使用します。アーキテクチャアダプターはXHTokenのものを`requirements.txt`で固定しています。ダウンロードしたモデルのPythonコードは実行しません。ロード前にモデル重みのSHA-256を確認します。パッケージのstrict tensor loadingと実際の生成テストは別の確認です。

```sh
uv venv services/spark-mlx/.venv --python 3.12
uv pip install --python services/spark-mlx/.venv/bin/python -r services/spark-mlx/requirements.txt
services/spark-mlx/.venv/bin/python services/spark-mlx/download_model.py
services/spark-mlx/.venv/bin/python services/spark-mlx/server.py
```

macOSではMetalが見えるセッションからサービスを起動してください。サービスは`127.0.0.1:8765`だけにbindします。`SPARK_PORT`でポートを、`SPARK_MODEL_PATH`でチェックポイントのディレクトリを変更できます。ブラウザからのhostリクエストとloopback以外のhostは拒否します。

- `GET /health`: 実際にロードしたモデル、設定された量子化、デバイス、revision。
- `POST /v1/generate`: `model`（不一致は推論前に拒否）、`task`、`messages`、JSON `schema`、`max_tokens`、`temperature`。
- 対応task: `difficulty`（5段階評価）、`route`、`draft_context`、`draft_handoff`、`implement`、`summarize`、`review`、`retrieval_query`、`memory_extract`。`draft_handoff` は決定論的検証に渡す会話の原文引用候補を生成します。Reasoningはこのローカルサービスで固定され、リクエストごとの切り替えには対応しません。
- 出力JSONはparseして検証します。これは生成後のschema検証であり、文法制約付きデコードを意味しません。不正な出力は422で拒否します。推論中は429を返します。RustプロバイダーはMLXリクエストの重複を避けるため、プロセス内の呼び出しを直列化します。検証エラーでは、生成したpayloadを公開せず、長さを制限したschema診断だけを返します。
- MLXを所有する専用推論threadは1本です。入力は8192 token、出力は4096 tokenに制限します。
- サービスにはshellやファイル編集のtoolはありません。Rust Hubが、適用前に提案された完全一致の置換と選択されたパスを検証します。

## 検証

```sh
uv pip install --python services/spark-mlx/.venv/bin/python pytest httpx
services/spark-mlx/.venv/bin/python -m pytest services/spark-mlx/test_server.py -q
cargo run -p hub-runtime --example spark_fixture
```

2つ目のコマンドグループには起動中のサービスが必要です。Codexを使わず、一時fixtureでroute → 編集提案 → 一次レビュー → 完全一致のファイル検証 → 進捗要約を実行します。

配布元: [Hugging FaceのSparkモデル](https://huggingface.co/abenzerps/Spark-X2.5-4B-MLX-8bit)、[Spark-MLX-LLM](https://github.com/XHToken/Spark-MLX-LLM)。量子化モデルの性能を、量子化前モデルの公開ベンチマークから推測していません。

## モデルを差し替える

`model_id`、`path`、`loader`（`spark`または`mlx_lm`）、`quantization_bits`を持つJSONファイルを用意し、`LOCAL_MODEL_CONFIG`に設定します。設定しない場合は、固定されたSpark 8bit構成が有効です。手順全体とHTTPアダプターの契約は[モデル設定](../../docs/MODEL_CONFIGURATION.md)を参照してください。追加のMLX-LMモデルではcheckpointのconfig・shard検証とstrict loadingを行いますが、Spark配布元のchecksum検証や品質の根拠は引き継ぎません。
