//! User-owned five-level mapping. Classification never chooses the execution model.
use anyhow::{bail, Result};
use hub_core::RiskLevel;
use protocol_types::local::{DifficultyAssessment, RoutingInput};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelProvider {
    Local,
    Codex,
    Api,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelTarget {
    pub provider: ModelProvider,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    pub model: String,
    pub reasoning: Option<String>,
}
impl ModelTarget {
    pub fn validate(&self) -> Result<()> {
        if self.model.trim().is_empty()
            || self.model.len() > 300
            || self
                .reasoning
                .as_ref()
                .is_some_and(|e| e.trim().is_empty() || e.len() > 50)
        {
            bail!("モデル・reasoning の指定が不正です。");
        }
        match self.provider {
            ModelProvider::Local if self.profile_id.as_deref().is_some_and(|id| id != "spark") => {
                bail!("従来のローカルモデルは spark profile を使用してください。");
            }
            ModelProvider::Codex if self.profile_id.as_deref().is_some_and(|id| id != "codex") => {
                bail!("Codexモデルは codex profile を使用してください。");
            }
            ModelProvider::Api
                if self
                    .profile_id
                    .as_ref()
                    .is_none_or(|id| id.trim().is_empty()) =>
            {
                bail!("APIモデルにはprovider profileが必要です。");
            }
            _ => {}
        }
        if self.provider == ModelProvider::Local && self.reasoning.is_some() {
            bail!("現在のローカルサービスは reasoning の切り替えに対応していません。");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LevelAssignment {
    pub level: u8,
    pub target: ModelTarget,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutoSettings {
    pub classifier: ModelTarget,
    pub levels: Vec<LevelAssignment>,
    pub fallback: Option<ModelTarget>,
    #[serde(default)]
    pub planner_default: Option<ModelTarget>,
    #[serde(default)]
    pub reviewer_default: Option<ModelTarget>,
    pub confirm_before_run: bool,
}
impl AutoSettings {
    pub fn initial(local_model: String, default_cloud: Option<ModelTarget>) -> Self {
        let local = ModelTarget {
            provider: ModelProvider::Local,
            profile_id: Some("spark".into()),
            model: local_model,
            reasoning: None,
        };
        Self {
            classifier: local.clone(),
            levels: (1..=5)
                .map(|level| LevelAssignment {
                    level,
                    target: if level == 1 {
                        local.clone()
                    } else {
                        default_cloud.clone().unwrap_or_else(|| local.clone())
                    },
                })
                .collect(),
            fallback: None,
            planner_default: default_cloud.clone(),
            reviewer_default: default_cloud,
            confirm_before_run: false,
        }
    }
    pub fn validate(&self) -> Result<()> {
        self.classifier.validate()?;
        if self.levels.len() != 5
            || (1..=5).any(|level| self.levels.iter().filter(|v| v.level == level).count() != 1)
        {
            bail!("難易度1〜5に、それぞれ1つのモデルを設定してください。");
        }
        for level in &self.levels {
            level.target.validate()?;
        }
        if let Some(fallback) = &self.fallback {
            fallback.validate()?;
        }
        if let Some(planner) = &self.planner_default {
            planner.validate()?;
        }
        if let Some(reviewer) = &self.reviewer_default {
            reviewer.validate()?;
        }
        Ok(())
    }
    pub fn target(&self, level: u8) -> Result<ModelTarget> {
        self.validate()?;
        self.levels
            .iter()
            .find(|v| v.level == level)
            .map(|v| v.target.clone())
            .ok_or_else(|| anyhow::anyhow!("難易度は1〜5です。"))
    }
}
/// Changing a difficulty level never removes the independent planning gate.
pub fn planning_reason(
    input: &RoutingInput,
    assessment: Option<&DifficultyAssessment>,
) -> Option<String> {
    let text = input.request.to_lowercase();
    let reserved = [
        "architecture",
        "security",
        "authentication",
        "migration",
        "public api",
        "権限",
        "認証",
        "セキュリティ",
        "設計",
        "移行",
        "本番データ削除",
        "drop database",
    ];
    if assessment.is_some_and(|a| a.needs_plan || a.risk == RiskLevel::High)
        || reserved.iter().any(|word| text.contains(word))
    {
        Some("この依頼は計画の確認が必要です。操作を「計画する」に切り替えてください。難易度やモデルを変更しても、この条件は解除されません。".into())
    } else {
        None
    }
}
pub fn check_execution_scope(
    target: &ModelTarget,
    input: &RoutingInput,
    assessment: Option<&DifficultyAssessment>,
) -> Result<()> {
    target.validate()?;
    if target.provider == ModelProvider::Local
        && (input.known_files.is_empty()
            || input.known_files.len() > 2
            || input.estimated_loc.is_some_and(|v| v >= 100)
            || assessment.is_none_or(|a| {
                a.risk != RiskLevel::Low
                    || a.estimated_scope.files > 2
                    || a.estimated_scope.loc >= 100
            }))
    {
        bail!("選択されたローカルモデルの編集範囲を超えています。低リスクの小さな変更で、対象ファイルを1〜2件指定してください。");
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use protocol_types::local::EstimatedScope;
    fn target(model: &str) -> ModelTarget {
        ModelTarget {
            provider: ModelProvider::Codex,
            profile_id: Some("codex".into()),
            model: model.into(),
            reasoning: Some("high".into()),
        }
    }
    fn assessment() -> DifficultyAssessment {
        DifficultyAssessment {
            difficulty: 3,
            confidence: 0.9,
            reason: "multiple files".into(),
            risk: RiskLevel::Low,
            needs_plan: false,
            estimated_scope: EstimatedScope { files: 3, loc: 50 },
        }
    }
    #[test]
    fn each_level_uses_its_saved_model_and_reasoning() {
        let mut config = AutoSettings::initial("local".into(), Some(target("default")));
        for row in &mut config.levels {
            row.target = target(&format!("level-{}", row.level));
        }
        for level in 1..=5 {
            assert_eq!(
                config.target(level).unwrap(),
                target(&format!("level-{level}"))
            );
        }
        config.levels[4].level = 1;
        assert!(config.validate().is_err());
    }
    #[test]
    fn low_confidence_and_out_of_range_do_not_get_an_invented_level() {
        let mut value = assessment();
        value.confidence = 0.2;
        assert!(value.validate().is_err());
        value.confidence = 1.0;
        value.difficulty = 6;
        assert!(value.validate().is_err());
    }
    #[test]
    fn overriding_level_does_not_bypass_scope_or_planning() {
        let input = RoutingInput {
            request: "fix".into(),
            known_files: vec!["a".into()],
            estimated_loc: None,
        };
        let mut value = assessment();
        value.difficulty = 1;
        let local = ModelTarget {
            provider: ModelProvider::Local,
            profile_id: Some("spark".into()),
            model: "local".into(),
            reasoning: None,
        };
        assert!(check_execution_scope(&local, &input, Some(&value)).is_err());
        value.risk = RiskLevel::High;
        assert!(planning_reason(&input, Some(&value)).is_some());
        assert!(check_execution_scope(&target("remote"), &input, Some(&value)).is_ok());
    }
}
