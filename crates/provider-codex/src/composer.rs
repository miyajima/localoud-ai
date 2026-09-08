use crate::{CodexProvider, ProviderThread, ProviderTurn};
use anyhow::{bail, Context, Result};
use protocol_types::composer::{InputMention, TurnOptions};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::Path};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposerEntry {
    pub kind: String,
    pub name: String,
    pub label: String,
    pub description: String,
    pub path: String,
}
#[derive(Debug, Clone, Default, Serialize)]
pub struct ComposerCatalog {
    pub entries: Vec<ComposerEntry>,
    pub modes: Vec<String>,
    pub warnings: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingQuestion {
    pub request_id: String,
    pub thread_id: String,
    pub turn_id: String,
    pub questions: Vec<Value>,
}
impl CodexProvider {
    pub async fn composer_catalog(&self, root: &Path) -> Result<ComposerCatalog> {
        let mut result = ComposerCatalog::default();
        match self.request("skills/list", json!({"cwds":[root]})).await {
            Ok(value) => {
                for row in value["data"].as_array().context("invalid skill catalog")? {
                    if let Some(errors) = row["errors"].as_array() {
                        for e in errors {
                            result
                                .warnings
                                .push(e["message"].as_str().unwrap_or("スキル読込エラー").into());
                        }
                    }
                    for skill in row["skills"].as_array().context("missing skills")? {
                        if skill["enabled"] != true {
                            continue;
                        }
                        let name = skill["name"].as_str().context("skill name missing")?;
                        result.entries.push(ComposerEntry {
                            kind: "skill".into(),
                            name: name.into(),
                            label: skill["interface"]["displayName"]
                                .as_str()
                                .unwrap_or(name)
                                .into(),
                            description: skill["description"]
                                .as_str()
                                .unwrap_or("")
                                .chars()
                                .take(240)
                                .collect(),
                            path: skill["path"].as_str().context("skill path missing")?.into(),
                        });
                    }
                }
            }
            Err(e) => result.warnings.push(format!("スキル: {e}")),
        }
        match self
            .request("plugin/installed", json!({"cwds":[root]}))
            .await
        {
            Ok(value) => {
                for marketplace in value["marketplaces"]
                    .as_array()
                    .context("invalid plugin catalog")?
                {
                    for plugin in marketplace["plugins"]
                        .as_array()
                        .context("missing plugins")?
                    {
                        if plugin["installed"] != true
                            || plugin["enabled"] != true
                            || plugin["availability"] == "DISABLED_BY_ADMIN"
                        {
                            continue;
                        }
                        let name = plugin["name"].as_str().context("plugin name missing")?;
                        let id = plugin["id"].as_str().context("plugin ID missing")?;
                        result.entries.push(ComposerEntry {
                            kind: "plugin".into(),
                            name: name.into(),
                            label: plugin["interface"]["displayName"]
                                .as_str()
                                .unwrap_or(name)
                                .into(),
                            description: plugin["interface"]["shortDescription"]
                                .as_str()
                                .unwrap_or("インストール済みプラグイン")
                                .into(),
                            path: format!("plugin://{id}"),
                        });
                    }
                }
            }
            Err(e) => result.warnings.push(format!("プラグイン: {e}")),
        }
        match self.request("collaborationMode/list", json!({})).await {
            Ok(v) => {
                result.modes = v["data"]
                    .as_array()
                    .context("invalid mode catalog")?
                    .iter()
                    .filter_map(|m| m["mode"].as_str().map(str::to_owned))
                    .collect()
            }
            Err(e) => result.warnings.push(format!("モード: {e}")),
        }
        for (name, label, description) in [
            ("default", "汎用エージェント", "依頼に合う小さな作業を委任"),
            ("explorer", "調査エージェント", "コード調査を委任"),
            ("worker", "実装エージェント", "実装や修正を委任"),
        ] {
            result.entries.push(ComposerEntry {
                kind: "agent".into(),
                name: name.into(),
                label: label.into(),
                description: description.into(),
                path: String::new(),
            });
        }
        Ok(result)
    }
    pub async fn complete_composer_text(
        &self,
        root: std::path::PathBuf,
        model: &str,
        effort: &str,
        prompt: String,
    ) -> Result<String> {
        use crate::CodingAgentProvider;
        self.validate_model_reasoning(model, Some(effort)).await?;
        let effective = self
            .request("config/read", json!({"includeLayers":false,"cwd":root}))
            .await?;
        let mut config = json!({"features.shell_tool":false,"features.unified_exec":false,"features.apps":false,"features.plugins":false,"features.multi_agent":false,"features.browser_use":false,"features.computer_use":false,"features.view_image":false,"features.image_generation":false,"features.skill_search":false,"features.skip_host_skill_discovery":true,"agents.enabled":false,"web_search":"disabled"});
        if let Some(servers) = effective["config"]["mcp_servers"].as_object() {
            for name in servers.keys() {
                config[format!("mcp_servers.{name}.enabled")] = json!(false);
            }
        }
        let start=self.request("thread/start",json!({"cwd":root,"runtimeWorkspaceRoots":[root],"model":model,"sandbox":"read-only","approvalPolicy":"never","ephemeral":true,"environments":[],"config":config,"developerInstructions":"You only complete unfinished user input. Never answer the request, inspect files, run tools or delegate. Return the requested short JSON suffix."})).await?;
        anyhow::ensure!(
            start["model"] == model,
            "補完モデルが一致しません。入力は送信していません。"
        );
        let thread = ProviderThread {
            id: start["thread"]["id"]
                .as_str()
                .context("missing completion thread")?
                .into(),
        };
        let mut events = self.events();
        let started=self.request("turn/start",json!({"threadId":thread.id,"model":model,"effort":effort,"input":[{"type":"text","text":prompt}],"outputSchema":{"type":"object","properties":{"suffix":{"type":"string","maxLength":320}},"required":["suffix"],"additionalProperties":false}})).await?;
        let turn = ProviderTurn {
            thread_id: thread.id.clone(),
            id: started["turn"]["id"]
                .as_str()
                .context("missing completion turn")?
                .into(),
            status: "inProgress".into(),
        };
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(20);
        let mut output = String::new();
        loop {
            let event = match tokio::time::timeout_at(deadline, events.recv()).await {
                Ok(Ok(e)) => e,
                _ => {
                    let _ = self.interrupt_turn(&turn).await;
                    bail!("補完が時間内に完了しませんでした。");
                }
            };
            if event.thread_id.as_deref() != Some(&thread.id) {
                continue;
            }
            if event.kind == "input_required"
                || (event.kind == "item_started"
                    && !matches!(
                        event.text.as_str(),
                        "agentMessage" | "reasoning" | "userMessage"
                    ))
            {
                let _ = self.interrupt_turn(&turn).await;
                bail!("補完以外の操作が要求されたため停止しました。");
            }
            if event.kind == "message_completed" {
                output = event.text;
            } else if event.kind == "turn_completed" && event.turn_id.as_deref() == Some(&turn.id) {
                anyhow::ensure!(
                    event.text == "completed",
                    "補完が正常に終了しませんでした。"
                );
                let value: Value = serde_json::from_str(&output)?;
                let suffix = value["suffix"].as_str().context("missing suffix")?;
                anyhow::ensure!(suffix.chars().count() <= 320, "補完が長すぎます。");
                return Ok(suffix.into());
            }
        }
    }
    pub async fn goal_state(&self, thread: &str) -> Result<Value> {
        self.request("thread/goal/get", json!({"threadId":thread}))
            .await
    }
    pub async fn pause_goal(&self, thread: &str) -> Result<Value> {
        self.request(
            "thread/goal/set",
            json!({"threadId":thread,"status":"paused"}),
        )
        .await
    }
    pub async fn pending_questions(&self, thread: &str) -> Vec<PendingQuestion> {
        self.questions
            .lock()
            .await
            .values()
            .filter(|q| q.thread_id == thread)
            .cloned()
            .collect()
    }
    pub async fn answer_question(
        &self,
        thread: &str,
        request_id: &str,
        answers: BTreeMap<String, Vec<String>>,
    ) -> Result<()> {
        let mut pending = self.questions.lock().await;
        let q = pending
            .get(request_id)
            .context("この質問は終了済みか、接続が切れています。状態を確認してください。")?;
        anyhow::ensure!(q.thread_id == thread, "質問のタスクが一致しません。");
        anyhow::ensure!(answers.len() <= q.questions.len(), "回答数が不正です。");
        for (key, values) in &answers {
            anyhow::ensure!(
                q.questions.iter().any(|q| q["id"].as_str() == Some(key)),
                "質問IDが一致しません。"
            );
            anyhow::ensure!(
                values.len() <= 5 && values.iter().all(|v| v.chars().count() <= 4000),
                "回答が長すぎます。"
            );
        }
        let wire: BTreeMap<_, _> = answers
            .into_iter()
            .map(|(id, answers)| (id, json!({"answers":answers})))
            .collect();
        let id: Value = serde_json::from_str(request_id)?;
        self.write(json!({"id":id,"result":{"answers":wire}}))
            .await?;
        pending.remove(request_id);
        Ok(())
    }
    pub(crate) async fn send_composer_turn(
        &self,
        thread: &ProviderThread,
        text: String,
        model: &str,
        effort: Option<&str>,
        options: TurnOptions,
    ) -> Result<ProviderTurn> {
        options.validate()?;
        self.validate_model_reasoning(model, effort).await?;
        let mut prefix = Vec::new();
        let mut input = Vec::new();
        for mention in options.mentions {
            match mention {
                InputMention::Skill { name, path } => {
                    prefix.push(format!("${name}"));
                    input.push(json!({"type":"skill","name":name,"path":path}));
                }
                InputMention::Plugin { name, path } => {
                    prefix.push(format!("@{name}"));
                    input.push(json!({"type":"mention","name":name,"path":path}));
                }
                InputMention::File { name, path } => {
                    prefix.push(format!("参照ファイル: {name} ({path})"));
                }
                InputMention::Agent { name } => {
                    prefix.push(format!("サブエージェントへの委任依頼: この依頼のために {name} エージェントを呼び出してください。適用される制約に従い、利用できない場合は理由を報告してください。モデルやReasoningは接続先の設定を引き継いでください。"));
                }
            }
        }
        let text = if prefix.is_empty() {
            text
        } else {
            format!("{}\n\n{}", prefix.join("\n"), text)
        };
        input.insert(0, json!({"type":"text","text":text}));
        let mut params = json!({"threadId":thread.id,"model":model,"effort":effort,"input":input});
        if let Some(mode) = options.mode {
            let mode = serde_json::to_value(mode)?;
            let modes = self.request("collaborationMode/list", json!({})).await?;
            anyhow::ensure!(
                modes["data"]
                    .as_array()
                    .is_some_and(|v| v.iter().any(|m| m["mode"] == mode)),
                "指定したCodexモードは利用できません。"
            );
            params["collaborationMode"] = json!({"mode":mode,"settings":{"model":model,"reasoning_effort":effort,"developer_instructions":null}});
        }
        if let Some(objective) = options.goal {
            self.request(
                "thread/goal/set",
                json!({"threadId":thread.id,"objective":objective,"status":"active"}),
            )
            .await?;
        }
        let v = self.request("turn/start", params).await?;
        if v["turn"]["id"].as_str().is_none() {
            bail!("missing turn ID");
        }
        Ok(ProviderTurn {
            thread_id: thread.id.clone(),
            id: v["turn"]["id"].as_str().unwrap().into(),
            status: v["turn"]["status"].as_str().unwrap_or("unknown").into(),
        })
    }
}
