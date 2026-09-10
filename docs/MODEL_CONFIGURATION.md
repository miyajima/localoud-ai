# モデルの選択と差し替え

## 入力欄

「操作」は **実装する／プランモード（Codex）／ゴールを設定／実行計画を作る**、「モデル」は接続先が返すモデル名から選択します。ローカルモデルには「ローカル」と補足します。モデル一覧の更新ボタンはモデル情報を取得するだけで、タスクの推論を始めません。

- 明示したモデルはそのまま使用します。利用不可・モデル不一致なら停止し、別モデルへ置換しません。
- 「自動」のときだけルーティングします。設計・認証など計画が必要な依頼では、実装を始めず計画作成を案内します。
- ローカル編集はモデルを差し替えても1〜2個の指定ファイルへの小規模な置換に制限されます。モデルを大きくしても自動的に権限は増えません。
- 「実行計画を作る」では取得したクラウドモデルを選び、読み取り専用の構造化計画を作成します。実装の開始は計画を確認してからです。計画作成に選んだモデルは、その後の DAG worker のモデル指定ではありません（worker は既存の Codex 設定を使用）。
- 設定画面のレビュー・診断モデルは、最終レビュー・診断の既定値です。入力欄で明示する計画モデルとは別です。
- 新規タスクはモデル ID を保存します。Codex の再開時も同じ ID を要求・照合します。ローカルモデルを差し替えた後、以前のモデルのタスクを続けるには設定を戻すか新規タスクを作成します。以前の版のクラウドタスクで ID が未記録の場合は「モデル未取得」と表示し、現在のモデル名で過去を上書きしません。

モデル一覧は「Codex」「ローカル」「ChatGPT（未接続）」に分けて表示します。**GPT-6-Astra** は Codex 経由、**GPT-6-Astra(ChatGPT)** は ChatGPT 直接接続用の名称です。現在、ChatGPT 直接接続は未実装のため後者は選択できません。

## Reasoning

モデルの隣で、接続先がそのモデルに対応すると返した Reasoning を選択できます。`既定（medium）` などの表示はプロバイダーの既定値、`high` などの明示値は保存する指定値です。対応値はモデルごとに変わります。モデルや操作を切り替えると既定値へ戻ります。現在のローカルサービスは切り替え非対応のため「非対応（固定）」と表示します。

新規タスクのモデルと明示した Reasoning は保存され、再開後の `turn/start` にも同じ `model` / `effort` を送ります。作成済みタスクでは固定表示です。変更したい場合は新規タスクを作成します。計画作成、および接続設定のレビュー・診断にも Reasoning を指定できます。計画の設定は DAG worker の設定へは引き継ぎません。

「自動」では [Auto の振り分け設定](AUTO_ROUTING.md) に保存した難易度ごとのモデル・Reasoning を使います。

## 現在の設定

| 項目 | 初期値 |
|---|---|
| 表示名 | Spark X-2.5 8bit |
| モデル ID | `abenzerps/Spark-X2.5-4B-MLX-8bit` |
| 接続先 | `http://127.0.0.1:8765` |
| 量子化ビット数 | 8 |

Localoud AI の「接続設定 → ローカルモデル」で変更します。保存時に専用 API または OpenAI 互換 API を検出し、モデル ID と ready 状態を照合して量子化ビット数を自動取得します。ビット数は手入力しません。取得値は実行時のモデル照合に使用し、量子化や推論品質を変更する設定ではありません。設定はアプリの SQLite に保存され、アプリ再起動後に反映されます。推論結果でもモデル ID とビット数を照合し、使用量は設定された実モデル ID で記録します。

ループバック HTTP 接続のみ対応します。URL に認証情報は入れられません。認証情報を使う別形式のプロバイダーには専用アダプターが必要です。

## 同梱サービスのモデルを差し替える

既存の Spark は追加設定なしで動きます。8bit と固定 revision・SHA-256 の確認も維持しています。

別の **MLX-LM 対応の量子化済みモデル**は、重みをローカルに配置し、任意の場所に以下の JSON を用意します。これは構成例であり、特定のモデルの動作保証ではありません。

```json
{
  "model_id": "organization/replacement-model",
  "path": "/absolute/path/to/local/weights",
  "loader": "mlx_lm",
  "quantization_bits": 8
}
```

モデル ID はこのチェックポイントを識別する値です。アプリにも同じ値を指定します。`path` が相対パスなら、この JSON の置き場所を基準に解決します。`loader` は `spark` または `mlx_lm` のみです。Spark 専用の構成では元の固定チェックポイント照合も必要です。

プロジェクトルートで、既存サービスを終了してから起動します。

```sh
LOCAL_MODEL_CONFIG=/absolute/path/to/model.json \
  services/spark-mlx/.venv/bin/python services/spark-mlx/server.py
```

サービスが ready になったら、アプリ側の表示名・モデル IDを合わせて保存し、Localoud AI を再起動します。別ポートで並行して準備する場合は `SPARK_PORT` を指定し、アプリの接続先も合わせます。

汎用 MLX-LM 経路は config.json のビット数、重み shard の存在、strict load を確認します。Spark のような配布元 checksum 検証は追加モデルには自動で付きません。モデルコードの `trust_remote_code` は無効です。新しいモデルの生成品質・JSON 応答適合性・メモリ使用量は別途確認が必要です。OpenAI 互換経路で実測した難易度判定の固定 10 ケースは [`docs/evidence/local-difficulty-comparison-2026-09-10/RESULT.md`](evidence/local-difficulty-comparison-2026-09-10/RESULT.md) に記録しています。

## 別の推論サービスを使う場合

アプリの Rust 側は `LocalModelProvider` に依存します。同梱 HTTP アダプターは、設定で接続先とモデル情報を差し替えられます。以下の契約に対応するサービスならアプリのコード変更は不要です。

- 専用 API: `GET /health` → `status: "ready"`, `model`, `quantization_bits`; `POST /v1/generate` ← `model`, `task`, `messages`, `schema`, `max_tokens`, `temperature`; 成功応答 → `output`（要求 schema を満たす JSON）, `usage.prompt_tokens`, `usage.completion_tokens`, `latency_ms`, `model`, `quantization_bits`
- OpenAI 互換 API: `GET /health` が `status: "ok"` または `"ready"`、`GET /v1/models` に指定したモデル ID（`meta.ftype` または ID の `Q8_0` / `q8` / `f16` などから量子化を検出）、`POST /v1/chat/completions` に構造化 JSON を要求
- どちらの経路も `difficulty`, `route`, `draft_context`, `implement`, `summarize`, `review`, `retrieval_query`, `memory_extract`, `input_completion` を同じ `LocalModelProvider` 操作へ変換します。OpenAI Responses API（`/v1/responses`）だけのサービスは対象外です。互換性のため内部 crate 名・永続 provider key は `spark` を保持しますが、表示・照合・使用量のモデル名は固定していません。

## 今回の確認

Rust のモデル指定・再開テストでは、既定と異なるモデルを要求し、勝手に既定へ戻らないことを fixture で検証しました。ローカルサービスでは不一致 ID の拒否、差し替え設定、ビット数不一致・重み不足の拒否を検証しました。

macOS アプリでは実モデル一覧、操作切り替え、ローカル設定表示、不一致時の保存拒否を確認しました。クラウドのモデル一覧は取得確認であり、各モデルで実装・計画を生成した証拠ではありません。

更新したサービスと Rust アダプターの実接続でも、Spark X-2.5 8bit による一時リポジトリの誤字修正・一次レビュー・要約が成功し、使用量に同じ実モデル ID が記録されることを確認済みです。
