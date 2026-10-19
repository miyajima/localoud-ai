# Sites–Mac WebSocket 実証結果（2026-10-19 JST）

結論: 本人限定SitesのWorkerと、このMac上のNodeクライアントの間で、Macから開始するWebSocket常時接続は成立した。CloudflareアカウントのWorkerやTailscaleを新規作成せず実施した。Sites自体の実行基盤はCloudflare Workersである。

## 実機結果

- Site: https://localoud-sites-mac-proof.miyazima.chatgpt.site
- 試験開始: 2026-10-18T18:02:25.652Z / 終了: 2026-10-18T18:03:32.947Z
- 認証付きHTTP health: 200、認証なし: 401。
- 他のテスト端末用トークンによる接続: 403。
- 独立した2本の実WebSocket接続: HTTP 101。
- 同じ接続の維持: 60.67秒。5秒ごとにnonce応答を確認。
- 切断後の新規接続と、他方の接続の継続: PASS。
- MacからSiteへのnonce往復: 32件、中央値48.33ms、p95 96.10ms、最大105.48ms。
- 異なる端末・接続ID・未処理nonceに一致しない応答は失敗とする。これは固定した2つの診断用capabilityの分離試験であり、一般ユーザーのOAuth分離や大量接続試験ではない。
- Sitesが提供したMCPのtransport_healthを本タスクの正式コネクター経由で実行し、transport_only / production_ready:falseを確認。ChatGPT Webの会話からの呼出試験とは区別する。

## 未成立の部分

最初に試した通常Workerの共有メモリ上のソケットを別HTTP要求から操作する方式は、ローカルworkerdで「Cannot perform I/O on behalf of a different request」により503となった。relay-failure.logと旧テスト原本を保存した。接続試験へ範囲を限定し、HTTP中継の失敗を成功扱いしていない。

したがって、今回確認したのはWebSocketの通信路。MCP要求→接続中のMacへの配送、本人と端末の登録・失効、操作結果の保存、ファイル編集、Mac側承認付きコマンド実行はこの試作では未実装。Cloudflare版と既存Macアプリは変更していない。

本実装へ進むには、独立したHTTP要求と永続接続の間をつなぐ共有状態・配送の設計が必要。SitesでDurable Objectsを使えるかは未確認。利用可能なD1要求キューをMacまたは接続ハンドラーが取得する案は候補だが、取得間隔の待ち時間・負荷・所有者確認・重複防止を別途検証する必要がある。MCP Eventsの採用や自動的な会話再開を、この結果から保証しない。

## 再現と検証の境界

- 実装: worker/index.js。SHA-256: a61c7a9078c0633016a154eb603568decfa1ecdb71afba7bb286f667a9a72857
- 実行: client/run-probe.mjs。秘密値は非表示の標準入力のみで受け付け、ヘッダーで送る。URLや結果ファイルに保存しない。
- 実機クライアントの認証は同一Site用のOAI-Sites-Authorizationと、診断用端末capability。これを一般配布用Macログイン方式として再利用しない。
- ローカル: transport 4テスト、client contract 4テストPASS。ESM build/validate PASS。Worker独立レビューPASS。
- 最初の実機測定はlive-probe-initial.json。判定修正後の測定はlive-probe.json。測定後の追加変更はnull/非オブジェクト応答を安全に失敗報告するためのガードのみで、追加の境界テスト4件PASS。最終ソースそのものの通信再測定とは区別する。
- dev用Miniflare依存にnpm audit highが3件残る。Worker成果物は単一ESMで依存ライブラリを同梱していない。
- 再利用対象: Sitesでの外向きWebSocket接続・維持可否を小さく切り分けたいとき。製品の中継や編集・実行の検証を代替しない。
