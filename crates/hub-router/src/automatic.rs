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
pub struct AgentProfile {
    pub id: String,
    pub name: String,
    pub target: ModelTarget,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteAgentAssignment {
    pub route: String,
    pub agent_id: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyRouteIdentity {
    pub route: String,
    pub role_name: String,
    pub agent_name: String,
}

const ROUTES: [&str; 9] = [
    "classifier",
    "level_1",
    "level_2",
    "level_3",
    "level_4",
    "level_5",
    "planner",
    "reviewer",
    "fallback",
];

fn default_agent_name(route: &str) -> &'static str {
    match route {
        "classifier" => "Router",
        "level_1" => "Quick",
        "level_2" => "Builder",
        "level_3" => "Developer",
        "level_4" => "Engineer",
        "level_5" => "Architect",
        "planner" => "Planner",
        "reviewer" => "Reviewer",
        "fallback" => "Fallback",
        _ => "Agent",
    }
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
    #[serde(default)]
    pub agents: Vec<AgentProfile>,
    #[serde(default)]
    pub route_agents: Vec<RouteAgentAssignment>,
    #[serde(default, rename = "identities", skip_serializing)]
    pub legacy_identities: Vec<LegacyRouteIdentity>,
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
        let mut settings = Self {
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
            agents: Vec::new(),
            route_agents: Vec::new(),
            legacy_identities: Vec::new(),
            confirm_before_run: false,
        };
        settings.ensure_agent_profiles();
        settings
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
        self.validate_agents()?;
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
    fn target_for_route(&self, route: &str) -> Option<ModelTarget> {
        let level = |number| {
            self.levels
                .iter()
                .find(|assignment| assignment.level == number)
                .map(|assignment| assignment.target.clone())
        };
        match route {
            "classifier" => Some(self.classifier.clone()),
            "level_1" => level(1),
            "level_2" => level(2),
            "level_3" => level(3),
            "level_4" => level(4),
            "level_5" => level(5),
            "planner" => self.planner_default.clone().or_else(|| level(5)),
            "reviewer" => self
                .reviewer_default
                .clone()
                .or_else(|| self.planner_default.clone())
                .or_else(|| level(5)),
            "fallback" => self.fallback.clone().or_else(|| level(3)),
            _ => None,
        }
    }
    pub fn ensure_agent_profiles(&mut self) -> bool {
        let complete = !self.agents.is_empty()
            && self.route_agents.len() == ROUTES.len()
            && ROUTES.iter().all(|route| {
                self.route_agents
                    .iter()
                    .filter(|assignment| assignment.route == *route)
                    .count()
                    == 1
            })
            && self.route_agents.iter().all(|assignment| {
                self.agents
                    .iter()
                    .any(|agent| agent.id == assignment.agent_id)
            });
        if complete {
            let migrated = !self.legacy_identities.is_empty();
            self.legacy_identities.clear();
            return migrated;
        }

        let specs: Vec<_> = ROUTES
            .iter()
            .filter_map(|route| {
                let target = self.target_for_route(route)?;
                let name = self
                    .legacy_identities
                    .iter()
                    .find(|identity| identity.route == *route)
                    .map(|identity| identity.agent_name.trim())
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| default_agent_name(route))
                    .to_string();
                Some(((*route).to_string(), name, target))
            })
            .collect();
        self.agents.clear();
        self.route_agents.clear();
        for (route, name, target) in specs {
            let agent_id = self
                .agents
                .iter()
                .find(|agent| agent.name == name && agent.target == target)
                .map(|agent| agent.id.clone())
                .unwrap_or_else(|| {
                    let id = format!("agent-{}", route.replace('_', "-"));
                    self.agents.push(AgentProfile {
                        id: id.clone(),
                        name,
                        target,
                    });
                    id
                });
            self.route_agents
                .push(RouteAgentAssignment { route, agent_id });
        }
        self.legacy_identities.clear();
        true
    }
    pub fn sync_targets_from_agents(&mut self) -> Result<()> {
        self.validate_agents()?;
        self.classifier = self.agent_for_route("classifier")?.target;
        for level in 1..=5 {
            let target = self.agent_for_level(level)?.target;
            self.levels
                .iter_mut()
                .find(|assignment| assignment.level == level)
                .ok_or_else(|| anyhow::anyhow!("難易度は1〜5です。"))?
                .target = target;
        }
        self.planner_default = Some(self.agent_for_route("planner")?.target);
        self.reviewer_default = Some(self.agent_for_route("reviewer")?.target);
        if self.fallback.is_some() {
            self.fallback = Some(self.agent_for_route("fallback")?.target);
        }
        Ok(())
    }
    fn validate_agents(&self) -> Result<()> {
        if self.agents.is_empty() || self.agents.len() > 50 {
            bail!("エージェントを1〜50件で設定してください。");
        }
        for agent in &self.agents {
            if agent.id.trim().is_empty()
                || agent.id.len() > 80
                || !agent
                    .id
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || "-_.".contains(character))
            {
                bail!("エージェントIDが不正です。");
            }
            if agent.name.trim().is_empty()
                || agent.name.chars().count() > 80
                || agent.name.chars().any(char::is_control)
            {
                bail!("エージェント名は1〜80文字で指定してください。");
            }
            agent.target.validate()?;
            if self
                .agents
                .iter()
                .filter(|candidate| candidate.id == agent.id)
                .count()
                != 1
            {
                bail!("エージェントIDは重複できません。");
            }
        }
        if self.route_agents.len() != ROUTES.len()
            || ROUTES.iter().any(|route| {
                self.route_agents
                    .iter()
                    .filter(|assignment| assignment.route == *route)
                    .count()
                    != 1
            })
            || self.route_agents.iter().any(|assignment| {
                !self
                    .agents
                    .iter()
                    .any(|agent| agent.id == assignment.agent_id)
            })
        {
            bail!("判定・難易度1〜5・計画・レビュー・代替実行にエージェントを割り当ててください。");
        }
        Ok(())
    }
    pub fn agent_for_route(&self, route: &str) -> Result<AgentProfile> {
        let assignment = self
            .route_agents
            .iter()
            .find(|assignment| assignment.route == route)
            .ok_or_else(|| anyhow::anyhow!("エージェントの割り当てがありません: {route}"))?;
        self.agents
            .iter()
            .find(|agent| agent.id == assignment.agent_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("割り当てたエージェントがありません: {route}"))
    }
    pub fn agent_for_level(&self, level: u8) -> Result<AgentProfile> {
        if !(1..=5).contains(&level) {
            bail!("難易度は1〜5です。");
        }
        self.agent_for_route(&format!("level_{level}"))
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
    #[test]
    fn agents_are_user_owned_and_legacy_json_gets_profiles() {
        let mut config = AutoSettings::initial("local".into(), Some(target("default")));
        let reviewer = config.agent_for_route("reviewer").unwrap();
        let agent = config
            .agents
            .iter_mut()
            .find(|agent| agent.id == reviewer.id)
            .unwrap();
        agent.name = "赤ペン担当".into();
        assert_eq!(
            config.agent_for_route("reviewer").unwrap().name,
            "赤ペン担当"
        );
        config.validate().unwrap();

        let mut value = serde_json::to_value(&config).unwrap();
        value.as_object_mut().unwrap().remove("agents");
        value.as_object_mut().unwrap().remove("route_agents");
        value.as_object_mut().unwrap().insert(
            "identities".into(),
            serde_json::json!([{
                "route":"reviewer",
                "role_name":"レビュー",
                "agent_name":"Legacy Reviewer"
            }]),
        );
        let mut migrated: AutoSettings = serde_json::from_value(value).unwrap();
        assert!(migrated.ensure_agent_profiles());
        assert_eq!(
            migrated.agent_for_route("reviewer").unwrap().name,
            "Legacy Reviewer"
        );
    }
}
