use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CollaborationMode {
    #[default]
    Default,
    Plan,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum InputMention {
    Skill { name: String, path: String },
    Plugin { name: String, path: String },
    File { name: String, path: String },
    Agent { name: String },
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TurnOptions {
    pub mode: Option<CollaborationMode>,
    pub mentions: Vec<InputMention>,
    pub goal: Option<String>,
}
impl TurnOptions {
    pub fn is_empty(&self) -> bool {
        self.mode.is_none() && self.mentions.is_empty() && self.goal.is_none()
    }
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(self.mentions.len() <= 20, "参照は20件までです。");
        if let Some(goal) = &self.goal {
            anyhow::ensure!(
                !goal.trim().is_empty() && goal.chars().count() <= 4000,
                "ゴールは1〜4000文字で指定してください。"
            );
            anyhow::ensure!(
                self.mode != Some(CollaborationMode::Plan),
                "プランモードとゴール開始は同時に指定できません。"
            );
        }
        for mention in &self.mentions {
            let (name, path) = match mention {
                InputMention::Skill { name, path }
                | InputMention::Plugin { name, path }
                | InputMention::File { name, path } => (name, Some(path)),
                InputMention::Agent { name } => (name, None),
            };
            anyhow::ensure!(
                !name.trim().is_empty() && name.len() <= 1000 && !name.contains(['\n', '\r']),
                "参照名が不正です。"
            );
            anyhow::ensure!(
                path.is_none_or(|p| !p.is_empty() && p.len() <= 4096 && !p.contains(['\n', '\r'])),
                "参照先が不正です。"
            );
        }
        Ok(())
    }
}
