# Codexサブエージェント1体とLocaloud worker1体の比較

同日追加の [Astra計画付き比較](PLANNED_AGENT_BENCHMARK.md) は別の新規課題で実施した。以下の計画なし比較とは条件を分けて参照する。

2026-09-09 JST。実際のCodex `spawn_agent` とLocaloudの独立workerを各1回実行した。この条件では、Localoudの初回入力は27.5%少なかったが、累計入力は8.2%多く、入力と出力の合計は8.4%多かった。トークン節約を示す結果ではない。

| 指標 | Codex native subagent | Localoud worker |
|---|---:|---:|
| 初回入力 | 35,648 | 25,833 |
| 累計入力（キャッシュを含む） | 110,348 | 119,428 |
| うちキャッシュ済み入力 | 91,904 | 86,144 |
| キャッシュを除く入力 | 18,444 | 33,284 |
| 出力 | 1,719 | 2,105 |
| 入力＋出力 | 112,067 | 121,533 |
| モデル呼び出し回数 | 3 | 4 |
| 実行時間（概数） | 72.5秒 | 86.9秒 |
| 固定テスト | 9/9 | 9/9 |
| 各workerが追加したテスト | 8/8 | 10/10 |
| 実行後の共通ランダム検証 | 2,000/2,000 | 2,000/2,000 |

両方とも実行ログで `gpt-6-astra` / `low` を確認した。同じGitコミットから作った別ディレクトリで、Pythonの区間正規化・差集合・所属判定を実装した。課題本文、policy.md、固定テストは共通。固定ファイルのハッシュと変更範囲も確認した。共通検証は固定seedで生成した区間について49点ずつ照合し、入力不変性、正規化結果の形、逆転区間の例外も検証した。

Codex側はこの会話から `fork_turns=all` で1体を起動した。引き継ぐのは圧縮後の利用可能な親コンテキストであり、会話開始以来の全履歴がそのまま渡るわけではない。Localoud側は本体の `CodexProvider::start_worker` と `start_turn_in_worktree` を使用し、独立した `thread/start` に課題を渡した。実験用runnerは [native_agent_benchmark.rs](../crates/hub-runtime/examples/native_agent_benchmark.rs)。今回は両方ともファイルから同じ方針を取得し、Localoud側に公開される `hub_context` の検索効果は検証していない。

Localoud側はモデル呼び出しが1回多く、途中で追加の指示ファイルも読み込んだ。Codex側は初回から20,224トークンがキャッシュ済みだったのに対し、Localoud側の初回キャッシュは0だった。これらが実測値に含まれるため、差をコンテキストの渡し方だけの効果とは断定できない。キャッシュを除く入力はLocaloud側が80.5%多かった。

集計は子worker自身の推論のみ。親による実験準備・指示・集計、プラットフォーム内部のguardian、ChatGPTでの計画・レビューは含まない。親の累積使用量を子に加算せず、各子のusageイベントで増分合計と最終累計の一致を確認した。キャッシュは入力の内数で、出力の内部内訳も重ねて加算していない。セッション全体の節約率やサブスクリプション消費量を示すものではない。

Localoudの最初の起動は、サンドボックスから既存Codex状態DBへ書けず、worker作成前に失敗した。失敗記録を残し、承認された権限で起動した1回のみを測定した。Codex側の完了結果は再実行していない。時間はLocaloudのビルド・起動待ちを除き、native側はspawnから最終応答までなので概数として扱う。

これは1課題・1ペアの実測で、標準Codex全般との優劣を確定するものではない。前回の人工的な長文履歴によるfork比較の「85.7%減」を、native subagentに対する一般的な削減率として使用しない。

証拠は [集計JSON](evidence/native-agent-benchmark-2026-09-09/report.json)、[Codex usage](evidence/native-agent-benchmark-2026-09-09/native/usage.json)、[Localoud usage](evidence/native-agent-benchmark-2026-09-09/localoud/usage.json)。同じディレクトリに両実装、テスト、検証出力、共通oracleを保存した。親の私的な会話本文は証拠ファイルへコピーしていない。
