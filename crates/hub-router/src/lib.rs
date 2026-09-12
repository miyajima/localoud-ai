pub mod automatic;
use anyhow::{bail, Result};
use hub_core::{Complexity, ExecutorKind, ExecutorPreference, RiskLevel};
use protocol_types::local::*;

pub struct RoutingPolicy {
    pub spark_confidence_threshold: f64,
    pub spark_max_estimated_loc: u32,
}
impl Default for RoutingPolicy {
    fn default() -> Self {
        Self {
            spark_confidence_threshold: 0.75,
            spark_max_estimated_loc: 100,
        }
    }
}
fn decision(executor: ExecutorKind, reason: &str, input: &RoutingInput) -> RoutingDecision {
    RoutingDecision {
        executor,
        complexity: if executor == ExecutorKind::Spark {
            Complexity::Trivial
        } else {
            Complexity::Normal
        },
        risk: RiskLevel::Low,
        needs_plan: executor == ExecutorKind::Astra,
        needs_final_astra_review: executor == ExecutorKind::Astra,
        estimated_scope: EstimatedScope {
            files: input.known_files.len() as u32,
            loc: input.estimated_loc.unwrap_or(0),
        },
        confidence: 1.0,
        reason: reason.into(),
    }
}
impl RoutingPolicy {
    pub async fn route(
        &self,
        input: &RoutingInput,
        preference: ExecutorPreference,
        provider: &dyn LocalModelProvider,
    ) -> Result<RouteReport> {
        if let Some(d) = self.pre_route(input, preference)? {
            if d.executor == ExecutorKind::Spark {
                if let Err(e) = provider.available().await {
                    if preference == ExecutorPreference::Spark {
                        return Err(e);
                    }
                    return Ok(RouteReport {
                        decision: self.codex_fallback(input, &e.to_string()),
                        usage: None,
                        fallback: true,
                    });
                }
            }
            return Ok(RouteReport {
                decision: d,
                usage: None,
                fallback: false,
            });
        }
        match provider.classify_task(input).await {
            Ok(g) => Ok(RouteReport {
                decision: self.validate_model_route(g.output, input)?,
                usage: Some(g.usage),
                fallback: false,
            }),
            Err(e) => Ok(RouteReport {
                decision: self.codex_fallback(input, &e.to_string()),
                usage: None,
                fallback: true,
            }),
        }
    }
    pub fn pre_route(
        &self,
        input: &RoutingInput,
        preference: ExecutorPreference,
    ) -> Result<Option<RoutingDecision>> {
        if input.request.trim().is_empty() {
            bail!("empty request");
        }
        if preference == ExecutorPreference::Codex {
            return Ok(Some(decision(
                ExecutorKind::Codex,
                "user selected model",
                input,
            )));
        }
        let text = input.request.to_lowercase();
        if [
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
        ]
        .iter()
        .any(|w| text.contains(w))
        {
            if preference == ExecutorPreference::Spark {
                bail!("この依頼には計画が必要です。モデルは切り替えていません。「プランモード」で確認してください。");
            }
            let mut d = decision(
                ExecutorKind::Astra,
                "architecture / security / migration requires planning",
                input,
            );
            d.risk = RiskLevel::High;
            d.complexity = Complexity::Deep;
            return Ok(Some(d));
        }
        match preference {
            ExecutorPreference::Codex => {
                return Ok(Some(decision(
                    ExecutorKind::Codex,
                    "user selected Codex",
                    input,
                )))
            }
            ExecutorPreference::Api => {
                return Ok(Some(decision(
                    ExecutorKind::Api,
                    "user selected API provider",
                    input,
                )))
            }
            ExecutorPreference::Astra => {
                return Ok(Some(decision(
                    ExecutorKind::Astra,
                    "user selected Astra",
                    input,
                )))
            }
            ExecutorPreference::Spark => {
                if input.known_files.is_empty() || input.known_files.len() > 2 {
                    bail!("Spark requires one or two explicitly selected files");
                }
                return Ok(Some(decision(
                    ExecutorKind::Spark,
                    "user selected Spark for bounded files",
                    input,
                )));
            }
            ExecutorPreference::Auto => {}
        }
        if input.known_files.is_empty()
            || input.known_files.len() > 2
            || input
                .estimated_loc
                .is_some_and(|v| v >= self.spark_max_estimated_loc)
        {
            return Ok(Some(decision(
                ExecutorKind::Codex,
                "repository exploration or multi-file scope",
                input,
            )));
        }
        if [
            "typo",
            "rename",
            "format only",
            "誤字",
            "名前を変更",
            "置換",
        ]
        .iter()
        .any(|w| text.contains(w))
        {
            return Ok(Some(decision(
                ExecutorKind::Spark,
                "bounded trivial edit in selected files",
                input,
            )));
        }
        Ok(None)
    }
    pub fn validate_model_route(
        &self,
        mut d: RoutingDecision,
        input: &RoutingInput,
    ) -> Result<RoutingDecision> {
        if !d.confidence.is_finite()
            || !(0.0..=1.0).contains(&d.confidence)
            || d.reason.trim().is_empty()
        {
            bail!("invalid routing result");
        }
        if d.risk == RiskLevel::High || d.needs_plan {
            d.executor = ExecutorKind::Astra;
            d.needs_plan = true;
            return Ok(d);
        }
        if d.executor == ExecutorKind::Spark
            && (d.confidence < self.spark_confidence_threshold
                || d.estimated_scope.files > 2
                || d.estimated_scope.loc >= self.spark_max_estimated_loc
                || input.known_files.is_empty()
                || input.known_files.len() > 2
                || d.risk != RiskLevel::Low)
        {
            d.executor = ExecutorKind::Codex;
            d.reason = format!("Spark route rejected by policy: {}", d.reason);
        }
        Ok(d)
    }
    pub fn codex_fallback(&self, input: &RoutingInput, reason: &str) -> RoutingDecision {
        decision(
            ExecutorKind::Codex,
            &format!("Spark unavailable or invalid: {reason}"),
            input,
        )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn input(s: &str) -> RoutingInput {
        RoutingInput {
            request: s.into(),
            known_files: vec!["a.txt".into()],
            estimated_loc: Some(2),
        }
    }
    #[test]
    fn security_cannot_be_forced_to_spark() {
        assert!(RoutingPolicy::default()
            .pre_route(&input("change authentication"), ExecutorPreference::Spark)
            .is_err());
        assert_eq!(
            RoutingPolicy::default()
                .pre_route(&input("change authentication"), ExecutorPreference::Codex)
                .unwrap()
                .unwrap()
                .executor,
            ExecutorKind::Codex
        );
    }
    #[test]
    fn trivial_known_file_routes_local() {
        assert_eq!(
            RoutingPolicy::default()
                .pre_route(&input("fix typo"), ExecutorPreference::Auto)
                .unwrap()
                .unwrap()
                .executor,
            ExecutorKind::Spark
        );
    }
    #[test]
    fn uncertain_local_route_escalates() {
        let i = input("fix");
        let mut d = decision(ExecutorKind::Spark, "uncertain", &i);
        d.confidence = 0.3;
        assert_eq!(
            RoutingPolicy::default()
                .validate_model_route(d, &i)
                .unwrap()
                .executor,
            ExecutorKind::Codex
        );
    }
}

#[cfg(test)]
mod availability_tests {
    use super::*;
    #[tokio::test]
    async fn unavailable_spark_falls_back_only_in_auto() -> Result<()> {
        let local = provider_spark::SparkProvider::new("http://127.0.0.1:1")?;
        let input = RoutingInput {
            request: "fix typo".into(),
            known_files: vec!["greeting.txt".into()],
            estimated_loc: Some(1),
        };
        let policy = RoutingPolicy::default();
        let auto = policy
            .route(&input, ExecutorPreference::Auto, &local)
            .await?;
        assert_eq!(auto.decision.executor, ExecutorKind::Codex);
        assert!(auto.fallback);
        assert!(policy
            .route(&input, ExecutorPreference::Spark, &local)
            .await
            .is_err());
        Ok(())
    }
}
