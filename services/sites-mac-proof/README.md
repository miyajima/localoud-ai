# Localoud Sites–Mac transport proof

本人限定のSitesとMacのWebSocket通信路だけを検証する試作。ファイル編集・シェル実行・HTTP要求からMacへの中継は含まない。実機結果と未確認事項は[REPORT.md](REPORT.md)を参照。

## ローカル検証

`npm ci` の後 `npm test`、`npm run build`。Node 22、Miniflare/workerdの固定版を使用。Worker自体は依存なしのESM。

## 実機検証

`node client/run-probe.mjs https://localoud-sites-mac-proof.miyazima.chatgpt.site /absolute/path/report.json`

起動後、同一Site用アクセス認証と2端末のcapabilityを、隠された標準入力へ渡す。値はリポジトリ・引数・URL・ログへ保存しない。プロパティ名はsiteAccess、a、b。WebSocketは60秒以上維持する。結果は秘密値を含まないJSON。一般配布向けの端末ペアリング認証ではない。

## 配置

.openai/hosting.jsonの既存project_idを維持し、Sites公式site-workflow.mjsでsourceをpush/build/package後、本人限定のsave_version_and_deploy_privateを使用する。Workerと同じソース状態のcommit_sha/archiveを渡す。PROOF_DEVICE_A_TOKENとPROOF_DEVICE_B_TOKENはSitesの秘密環境変数で設定する。

通常Workerの共有ソケットを別HTTP要求から操作する方式の失敗はevidence/に残してある。現在のtransport-onlyテスト成功で中継実装が成立したとは判断しない。
