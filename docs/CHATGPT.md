# ChatGPTとLocaloudの使い方

プロジェクト一覧の右、Localoud作業ペインの左にChatGPTのページを表示します。計画・レビューの入力、モデル選択、回答のコピーはユーザーが操作します。Localoudは回数の自動記録のため、生成中表示・回答ID・モデル名だけを観測します。本文・Cookieは取得せず、送信・回答取得・モデル選択も自動操作しません。旧Chrome拡張とBridgeは撤去しました。

## 通常の流れ

1. 左側のChatGPTに手動ログインする。専用の永続WKWebView領域を使い、ChromeのCookieは移植しない。
2. Localoudの「接続設定 → 読み取り専用MCP」で、公開する登録済みプロジェクトを選ぶ。
3. 別途配備した接続経路から、ChatGPTに読み取り専用MCPを接続する。下記の配備条件を参照。
4. プロジェクトを選び、開始方法を「ChatGPTで計画」にする。左のChatGPTへ作業内容・対象プロジェクト・条件を入力し、MCPの `project_get` と `handoff_schema` を使って計画を作るよう依頼する。
5. 確定したTask ManifestのJSONをコピーし、上部の「計画を取り込んで実行」を押す。取り込み操作時だけクリップボードを読む。
6. Localoudが依存関係と作業範囲に沿って最大3workerを並列実行し、統合成果物のIDと版を保存して「レビュー待ち」にする。
7. 上部の「レビュー依頼をコピー」で対象Task ID・成果物の版を含む依頼をコピーし、ChatGPTへ貼り付けてレビューを依頼する。`task_get`、`git_diff`、`file_read`、`verification_get` で統合成果物と検証ログを読む。省略された範囲は未レビューとして扱う。
8. Review Manifestをコピーして取り込む。`pass` は保存済み証拠がある場合に完了、`fail`／`inconclusive` は修正・再レビュー待ちとなる。修正対象stepがあれば対応するCodexセッションを再利用し、依存先も再実行して新しい統合成果物を作る。

境界をドラッグ、または境界にフォーカスして矢印キーで幅を変えられます。Localoudの左上の「ChatGPT」ボタンで全体を表示・非表示にできます。狭い画面では同じ領域を切り替えます。左のプロジェクト一覧の幅も境界から調整できます。

## Manifestと実行条件

正確なJSON例はMCPの `handoff_schema` から取得できます。Task Manifestは `kind: task`、`version: 1`、一意の `manifest_id`、登録済み `project_id`、完全なGit `base_revision`、依頼、受け入れ条件、scope、step・依存・難易度を含みます。最大48KB、最大8stepです。

開始時には基準revisionとcleanなソースcheckoutを要求します（Localoud管理の `.agent-worktrees` を除く）。各workerは独立worktreeで作業し、元checkoutへ自動マージしません。5段階ルーティングはworkerだけに適用し、実行開始時のモデル・思考量設定を固定します。ローカルworkerは難易度1・低リスク・既存1〜2ファイル・100変更行未満に限定します。

Review Manifestは `kind: review`、対象project／task、正確な `artifact_version`、判定、指摘、既存stepへの修正要求を含みます。対象違い・古い版・不正な依存関係・範囲逸脱は拒否します。同じManifest IDを再度取り込むと、再実行せず保存済みタスクを開きます。本文が変わっていても変更は適用しません。中断した作業は「再開・状態を確認」から再開します。修正の作業範囲を広げる場合は新しいTask Manifestが必要です。

中断・再起動後は保存済み状態を表示し、未確認の送信を自動再送しません。「再開・状態を確認」の操作で既存workerの停止を確認し、完了済みstepを保持して未完了stepを新しいworktreeで再開します。過去の試行・worktreeは保持します。旧ブラウザタスクは閲覧専用です。

## 読み取り専用MCPの接続

ローカルエンドポイントは `http://127.0.0.1:8792/mcp` です。Streamable HTTPのJSON応答方式で、継続SSEストリームは提供しません。全リクエストでBearer認証を要求し、キーはmacOS Keychainに保存します。アプリにキーを表示せず、明示的な「接続キーをコピー」でクリップボードへ書き出します。これは旧Bridgeの接続キーとは別です。

ChatGPTからlocalhostへ直接接続できるとは扱いません。OpenAIの案内では公開HTTPSエンドポイントまたはSecure MCP Tunnelが必要です。アプリの接続方式欄は既設経路の情報を保存するだけで、トンネル・OAuth・クラウド契約・外部公開を構築しません。Secure MCP TunnelにはPlatform側の設定と実行資格情報が別途必要です。外部経路には認証を設け、ローカル側へBearer認証と正しいHostを伝達するよう配備してください。ChatGPTが任意Bearerヘッダーを直接設定できる前提にはしていません。

参照した公式文書（2026-09-08確認）:

- [ChatGPT developer mode](https://developers.openai.com/api/docs/guides/developer-mode)
- [Connect from ChatGPT](https://developers.openai.com/plugins/deploy/connect-chatgpt)
- [Secure MCP Tunnels](https://developers.openai.com/api/docs/guides/secure-mcp-tunnels)

MCPは公開済みプロジェクトとその保存済みworktreeだけを読みます。ドットファイル、機密候補、シンボリックリンク、ビルド出力を除外し、ファイル256KB・応答64KB・走査5000件などの上限とページングを適用します。機密候補の自動除外は完全な機密判定ではないため、公開プロジェクトを選ぶ際は内容を確認してください。

公開する11ツールは `project_list`、`project_get`、`repo_tree`、`file_read`、`file_search`、`git_status`、`git_diff`、`task_list`、`task_get`、`verification_get`、`handoff_schema` です。ファイル変更、shell、テスト実行、worker制御はありません。

## 検証証拠と利用量

コマンドの結果には終了コード、観測時刻、workerの対象版、作業フォルダ、保存ログ参照を関連付けます。時刻はLocaloudがイベントを受信した時刻です。コマンドの対象版は完了イベントを受信した後のファイル状態の観測値で、実行中の版を厳密に証明する値ではありません。開始イベントがなければ開始時刻はnullです。`verification_get` の `step_key` と `log_index` で保存ログをページングできます。コマンド成功は受け入れ条件全体の成功を意味しません。未実行・失敗・不明はそのまま残します。workerで実行した検証を統合成果物に対する再実行とは扱いません。

ChatGPTの使用量は取得していないため不明です。Codex／ローカルworkerの使用量記録とは区別します。旧token比較は新方式の実機完走後に再設計します。

## 実機確認の境界

内蔵WebViewの表示、ユーザーによる日本語入力・コピーの利用確認、再起動後のログイン済み画面を確認しました。専用の検証データでは2workerの並列実行、統合、認証付きローカルMCP読み取り、古いレビューの拒否、同じworkerセッションへの修正返却、連続再開、合格Review Manifest取り込みまで実機確認しています。検証中に見つけた作業フォルダ再利用の不具合は、各turnのcwdと書き込み範囲を明示する修正後、連続再開で再確認しました。

Secure MCP Tunnelの接続準備完了（ready）と、ChatGPTへのLocaloud登録・11ツールの検出を確認しました。2026-09-09には、ChatGPTが作成したTask Manifestをユーザーが取り込み、2workerの実行・統合後、ChatGPTがMCPで成果物をレビューし、ユーザーが合格Review Manifestを取り込む一連の流れも完了しました。保存状態とアプリ表示の両方で完了を確認しています。先行する修正・再レビューの検証はCodexが用意したテストです。詳細は [実機検証記録](evidence/webview-manifest-validation.json) を参照してください。


## このMacで設定したトンネル

2026-09-08にPersonal組織の専用Secure MCP TunnelとChatGPTのLocaloudプラグインを作成しました。公開対象は `localoud-e2e-research-20260908` だけです。ChatGPT登録画面の「認証なし」は追加OAuthを使わない指定であり、トンネルの組織・ワークスペース認証とLocaloudのBearer認証は維持しています。

ランタイムキーはTunnels Read・Useだけを許可し、macOS Keychainに保存しています。再開はLocaloudのMCPを有効にしたうえで `python3 scripts/mcp-tunnel.py start`、確認は `python3 scripts/mcp-tunnel.py status`、停止は `python3 scripts/mcp-tunnel.py stop` です。`ready: true` を確認してください。Macログイン時の自動起動はまだ設定していません。現在の接続は管理されたプロセスで動作しています。

## 工程と作業記録

Localoudペイン上部に、選択したタスクの「依頼 → 計画 → 実行 → レビュー → 完了」を表示し、現在の工程と次の操作を示します。停止・再開や計画・レビューの取り込みも上部に集約しています。左側ではプロジェクトの下にタスクを表示し、選択中の作業と所属先を確認できます。

「概要」は依頼と全体の進捗、「計画」は取り込んだ計画、「担当」はworkerごとの範囲・状態・最新メッセージ、「ログ」は実行履歴と検証記録を表示します。タブは保存された作業記録を読むための画面です。詳しい責務は [作業フロー](WORKFLOW_UX.md) を参照してください。

「変更」では実際の作業フォルダの差分をファイル別に確認できます。実行中はworkerごと、統合後は統合成果物を参照し、新規ファイルも含めます。取得できない場合はエラーを表示します。進捗取得の一時的な失敗では実行状態を停止へ変更せず、再試行します。

レビュー依頼をコピーしてもレビュー完了にはなりません。対象の成果物に対するReview Manifestを取り込んだ結果で工程を更新します。一覧で選択したタスクと、ChatGPTで開いている会話は自動対応付けしません。

依頼全文や検証記録は詳細に折りたたみ、長い出力・差分だけを枠内でスクロールできます。開閉状態とスクロール位置は更新やタブの往復で保持します（アプリ終了時にはリセット）。Localoud左上の「ChatGPT」トグルは、WebViewとその列全体を切り替えます。

設定内にAstra＋Solの週合計とSolの日別の参考カウンターがあります。内蔵ChatGPTで生成開始から完了を検知すると、回答側のモデル情報（取得できなければ開始時の選択モデル）を確認して自動加算します。同じ回答IDは再計上せず、読み込み時の過去履歴は対象外です。モデルが分からない回答は「モデル未取得」として分けます。週200回・Sol日170回は参考枠で、他での利用を含まず、正式な残量ではありません。月曜0時／毎日0時の端末時刻で区切ります。過去の手動記録は引き継ぎます。

通常の「使用量」タブは選択中のセッションだけを表示し、全体の推論履歴は設定内に表示します。時間はローカルLLMの報告する処理時間、Codexの開始・完了イベント間の経過時間です。ChatGPTの観測時間も設定内に表示します。ブラウザを閉じていた区間や開始を観測できなかった回答は計測できません。ChatGPT側の画面構造変更により検知できなくなる場合があります。
