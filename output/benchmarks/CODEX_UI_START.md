# Codex UIから開始する比較実験

状態: UIでの手動開始待ち。Computer UseはCodexアプリ自体の操作を拒否したため、まだ実行していません。

Codexの新規タスクで親モデルを **GPT-6 Astra / low** に設定し、以下を送信してください。既存のlocaloudプロジェクトのローカル実行でも、指定のfixture外を変更しません。

```text
Codexの実際のUIから起動した場合の比較実験を実行してください。親はAstra/low、実装サブエージェント1体はLuna/max、親のレビューはAstra/lowです。この実験に限り1体のサブエージェント利用を許可します。

次のファイルを実験指示として読み、最後のレビューまで実施してください。
/private/tmp/localoud-codex-ui-df8o58_6/INSTRUCTIONS.md

計測対象は localoud-codex-ui-df8o58_6 です。
```

[詳細指示](/private/tmp/localoud-codex-ui-df8o58_6/INSTRUCTIONS.md) / [送信文](/private/tmp/localoud-codex-ui-df8o58_6/UI_PROMPT.txt) / [実験設定](/private/tmp/localoud-codex-ui-df8o58_6/setup.json)

公式のUI起動方法: [Subagents](https://learn.chatgpt.com/docs/agent-configuration/subagents#availability)。UI起動でトークンの無駄が解消済みかどうかは未確認です。
