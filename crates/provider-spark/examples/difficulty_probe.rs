use anyhow::{Context, Result};
use protocol_types::local::{DifficultyAssessment, LocalModelProvider, RoutingInput};
use provider_spark::{LocalProtocol, SparkProvider};
use serde::Serialize;
use std::env;

#[derive(Clone, Copy)]
struct Case {
    id: &'static str,
    expected_difficulty: u8,
    expected_risk: &'static str,
    request: &'static str,
    known_files: &'static [&'static str],
    estimated_loc: Option<u32>,
}

#[derive(Serialize)]
struct CaseReport {
    id: String,
    expected_difficulty: u8,
    expected_risk: String,
    request: String,
    known_files: Vec<String>,
    estimated_loc: Option<u32>,
    valid_contract: bool,
    difficulty: Option<u8>,
    risk: Option<String>,
    confidence: Option<f64>,
    exact_difficulty: bool,
    absolute_error: Option<u8>,
    risk_match: Option<bool>,
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
    latency_ms: Option<u64>,
    error: Option<String>,
}

#[derive(Serialize)]
struct ProbeReport {
    endpoint: String,
    model_id: String,
    display_name: String,
    protocol: LocalProtocol,
    quantization_bits: u8,
    cases: Vec<CaseReport>,
    valid_count: usize,
    exact_count: usize,
    within_one_count: usize,
    risk_match_count: usize,
}

const CASES: &[Case] = &[
    Case {
        id: "level-1-typo",
        expected_difficulty: 1,
        expected_risk: "low",
        request: "Fix the typo 'teh' to 'the' in README.md.",
        known_files: &["README.md"],
        estimated_loc: Some(1),
    },
    Case {
        id: "level-1-exact-replacement",
        expected_difficulty: 1,
        expected_risk: "low",
        request: "Replace the exact label 'Start' with 'Begin' in config.toml and change nothing else.",
        known_files: &["config.toml"],
        estimated_loc: Some(1),
    },
    Case {
        id: "level-2-validation",
        expected_difficulty: 2,
        expected_risk: "low",
        request: "Add bounds validation to the existing parse_port function in src/config.rs without changing its public API.",
        known_files: &["src/config.rs"],
        estimated_loc: Some(20),
    },
    Case {
        id: "level-2-unit-test",
        expected_difficulty: 2,
        expected_risk: "low",
        request: "Add one unit test for the existing empty-input behavior in src/parser.rs.",
        known_files: &["src/parser.rs"],
        estimated_loc: Some(15),
    },
    Case {
        id: "level-3-feature",
        expected_difficulty: 3,
        expected_risk: "low",
        request: "Add CSV export using the existing serializer, update its unit tests, and keep the current command-line interface.",
        known_files: &["src/export.rs", "tests/export.rs"],
        estimated_loc: Some(80),
    },
    Case {
        id: "level-3-cross-file-fix",
        expected_difficulty: 3,
        expected_risk: "medium",
        request: "Fix null handling across the API handler and its integration tests; reproduce the failing case before editing.",
        known_files: &["src/api.rs", "tests/api.rs"],
        estimated_loc: Some(60),
    },
    Case {
        id: "level-4-unknown-crash",
        expected_difficulty: 4,
        expected_risk: "medium",
        request: "Investigate an intermittent crash with no stack trace, identify the root cause, and propose a safe fix.",
        known_files: &[],
        estimated_loc: None,
    },
    Case {
        id: "level-4-auth-audit",
        expected_difficulty: 4,
        expected_risk: "high",
        request: "Audit a possible authentication bypass across the service, tests, and deployment configuration, then fix it without weakening access controls.",
        known_files: &["src/auth.rs", "tests/auth.rs", "deploy/policy.yaml"],
        estimated_loc: None,
    },
    Case {
        id: "level-5-production-migration",
        expected_difficulty: 5,
        expected_risk: "high",
        request: "Design and implement a backward-compatible production database migration with rollback, data backfill, and a staged deployment plan.",
        known_files: &[],
        estimated_loc: None,
    },
    Case {
        id: "level-5-architecture",
        expected_difficulty: 5,
        expected_risk: "medium",
        request: "Replace the monolith's synchronous workflow with an event-driven architecture while preserving public behavior and operational recovery.",
        known_files: &[],
        estimated_loc: None,
    },
];

fn report_case(
    case: Case,
    result: Result<protocol_types::local::Generation<DifficultyAssessment>>,
) -> CaseReport {
    match result {
        Ok(generation) => {
            let assessment = generation.output;
            let valid_contract = assessment.validate().is_ok();
            let difficulty = Some(assessment.difficulty);
            let absolute_error = Some(assessment.difficulty.abs_diff(case.expected_difficulty));
            let risk = Some(format!("{:?}", assessment.risk).to_ascii_lowercase());
            let risk_match = Some(risk.as_deref() == Some(case.expected_risk));
            CaseReport {
                id: case.id.into(),
                expected_difficulty: case.expected_difficulty,
                expected_risk: case.expected_risk.into(),
                request: case.request.into(),
                known_files: case.known_files.iter().map(|file| (*file).into()).collect(),
                estimated_loc: case.estimated_loc,
                valid_contract,
                difficulty,
                risk,
                confidence: Some(assessment.confidence),
                exact_difficulty: valid_contract
                    && assessment.difficulty == case.expected_difficulty,
                absolute_error,
                risk_match,
                prompt_tokens: Some(generation.usage.prompt_tokens),
                completion_tokens: Some(generation.usage.completion_tokens),
                latency_ms: Some(generation.usage.latency_ms),
                error: (!valid_contract).then(|| "DifficultyAssessment::validate failed".into()),
            }
        }
        Err(error) => CaseReport {
            id: case.id.into(),
            expected_difficulty: case.expected_difficulty,
            expected_risk: case.expected_risk.into(),
            request: case.request.into(),
            known_files: case.known_files.iter().map(|file| (*file).into()).collect(),
            estimated_loc: case.estimated_loc,
            valid_contract: false,
            difficulty: None,
            risk: None,
            confidence: None,
            exact_difficulty: false,
            absolute_error: None,
            risk_match: None,
            prompt_tokens: None,
            completion_tokens: None,
            latency_ms: None,
            error: Some(format!("{error:#}")),
        },
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = env::args().skip(1);
    let endpoint = args
        .next()
        .context("usage: difficulty_probe ENDPOINT MODEL_ID DISPLAY_NAME")?;
    let model_id = args
        .next()
        .context("usage: difficulty_probe ENDPOINT MODEL_ID DISPLAY_NAME")?;
    let display_name = args.next().unwrap_or_else(|| model_id.clone());
    let config = SparkProvider::discover_config(endpoint, model_id, display_name).await?;
    let provider = SparkProvider::configured(config.clone())?;
    provider.health().await?;
    let mut cases = Vec::with_capacity(CASES.len());
    for case in CASES {
        let input = RoutingInput {
            request: case.request.into(),
            known_files: case.known_files.iter().map(|file| (*file).into()).collect(),
            estimated_loc: case.estimated_loc,
        };
        cases.push(report_case(*case, provider.assess_difficulty(&input).await));
    }
    let valid_count = cases.iter().filter(|case| case.valid_contract).count();
    let exact_count = cases.iter().filter(|case| case.exact_difficulty).count();
    let within_one_count = cases
        .iter()
        .filter(|case| case.absolute_error.is_some_and(|error| error <= 1))
        .count();
    let risk_match_count = cases
        .iter()
        .filter(|case| case.risk_match == Some(true))
        .count();
    let report = ProbeReport {
        endpoint: config.endpoint,
        model_id: config.model_id,
        display_name: config.display_name,
        protocol: config.protocol,
        quantization_bits: config.quantization_bits,
        cases,
        valid_count,
        exact_count,
        within_one_count,
        risk_match_count,
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
