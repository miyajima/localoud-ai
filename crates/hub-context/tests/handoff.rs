use hub_context::handoff::*;
use protocol_types::local::{
    ConversationHandoffDraft, ConversationMessage, ConversationRole, HandoffCandidate,
};

fn fixture() -> Handoff {
    let sections = [
        Section::Purpose,
        Section::Constraints,
        Section::Decisions,
        Section::CurrentState,
        Section::Unresolved,
        Section::Completion,
    ];
    let texts = [
        "今回だけ slug.py を修正。",
        "外部通信は禁止。ただしローカルテストは可。",
        "ASCII のみを採用。",
        "実装は未着手。",
        "空文字の仕様は未確定。",
        "テスト成功と差分レビューで終了。",
    ];
    Handoff {
        sources: texts
            .iter()
            .enumerate()
            .map(|(i, t)| Evidence {
                id: format!("s{i}"),
                role: Role::User,
                reference: format!("turn:{i}"),
                text: t.to_string(),
                call_id: None,
            })
            .collect(),
        groups: sections
            .iter()
            .enumerate()
            .map(|(i, s)| Group {
                id: format!("g{i}"),
                section: *s,
                source_ids: vec![format!("s{i}")],
                depends_on: vec![],
                corrects: vec![],
                verified_outcome: false,
            })
            .collect(),
        outcomes: vec![],
        max_bytes: 12000,
    }
}
#[test]
fn preserves_transient_qualifiers_code_and_three_turn_corrections() {
    let mut h = fixture();
    h.sources[2].text = "Use src/a.b.py and https://example.test/a?q=1.2\n```python\nif x != 1:\n    return 'no. never'\n```".into();
    for (n, old) in [(6, 2), (7, 6)] {
        h.sources.push(Evidence {
            id: format!("s{n}"),
            role: Role::User,
            reference: format!("turn:{n}"),
            text: format!("訂正 {n}。空文字は空文字を返す。ただし None は対象外。"),
            call_id: None,
        });
        h.groups.push(Group {
            id: format!("g{n}"),
            section: Section::Decisions,
            source_ids: vec![format!("s{n}")],
            depends_on: vec![],
            corrects: vec![format!("g{old}")],
            verified_outcome: false,
        });
    }
    let selected = h.select().unwrap();
    assert_eq!(selected.sources.len(), 8);
    for (a, b) in selected.sources.iter().zip(&h.sources) {
        assert_eq!(a.text.as_bytes(), b.text.as_bytes());
        assert_eq!(a.reference, b.reference);
    }
    assert!(selected.authority.contains("grant no permission"));
    assert_eq!(selected.groups[7].corrects, ["g6"]);
}
#[test]
fn mandatory_overflow_fails_optional_omission_is_visible() {
    let mut h = fixture();
    h.max_bytes = 1;
    assert!(h.select().unwrap_err().to_string().contains("mandatory"));
    h.max_bytes = 6000;
    h.sources.push(Evidence {
        id: "b".into(),
        role: Role::Assistant,
        reference: "turn:8".into(),
        text: "background".repeat(2000),
        call_id: None,
    });
    h.groups.push(Group {
        id: "bg".into(),
        section: Section::Background,
        source_ids: vec!["b".into()],
        depends_on: vec![],
        corrects: vec![],
        verified_outcome: false,
    });
    assert_eq!(h.select().unwrap().omitted_background_groups, ["bg"]);
    h.groups[0].depends_on.push("bg".into());
    assert!(h.select().is_err());
}
#[test]
fn rejects_missing_evidence_misattribution_and_reversed_correction() {
    let mut h = fixture();
    h.groups[0].source_ids = vec!["missing".into()];
    assert!(h.select().is_err());
    let mut h = fixture();
    h.sources[1].role = Role::Tool;
    assert!(h.select().is_err());
    let mut h = fixture();
    h.groups[0].corrects = vec!["g2".into()];
    assert!(h.select().is_err());
    let mut h = fixture();
    h.groups.pop();
    assert!(h.select().is_err());
    let mut h = fixture();
    h.sources.push(h.sources[0].clone());
    assert!(h.select().is_err());
}
#[test]
fn only_matching_completed_tool_evidence_supports_execution_result() {
    let mut h = fixture();
    h.groups[3].verified_outcome = true;
    h.sources[3].call_id = Some("test-1".into());
    h.outcomes.push(Outcome {
        call_id: "test-1".into(),
        completed: true,
        exit_code: Some(0),
    });
    assert!(h.select().is_err()); // Human/assistant assertion is not tool execution.
    h.sources[3].role = Role::Tool;
    assert!(h.select().is_ok());
    h.outcomes[0].call_id = "different-call".into();
    assert!(h.select().is_err());
    h.outcomes[0].call_id = "test-1".into();
    h.outcomes[0].exit_code = Some(1);
    assert!(h.select().is_err());
    h.outcomes[0].exit_code = Some(0);
    h.outcomes[0].completed = false;
    assert!(h.select().is_err());
}

#[test]
fn malformed_correction_target_returns_error_without_panicking() {
    let mut h = fixture();
    h.groups[0].corrects = vec!["g5".into()];
    h.groups[5].source_ids.clear();
    assert!(h.select().is_err());
    h.groups[5].source_ids = vec!["missing".into()];
    assert!(h.select().is_err());
}

fn conversation_fixture() -> (Vec<ConversationMessage>, ConversationHandoffDraft) {
    let sections = [
        "purpose",
        "constraints",
        "decisions",
        "current_state",
        "unresolved",
        "completion",
    ];
    let messages = sections
        .iter()
        .enumerate()
        .map(|(index, section)| ConversationMessage {
            id: format!("message-{index}"),
            role: ConversationRole::User,
            text: format!("Exact {section} quote {index}"),
            call_id: None,
            completed: None,
            exit_code: None,
        })
        .collect::<Vec<_>>();
    let draft = ConversationHandoffDraft {
        candidates: sections
            .iter()
            .enumerate()
            .map(|(index, section)| HandoffCandidate {
                id: format!("candidate-{index}"),
                source_message_id: format!("message-{index}"),
                quote: format!("Exact {section} quote {index}"),
                section: section.to_string(),
                depends_on: vec![],
                corrects: vec![],
                confidence: 0.9,
            })
            .collect(),
    };
    (messages, draft)
}

#[test]
fn local_draft_is_rechecked_against_exact_visible_conversation() {
    let (messages, draft) = conversation_fixture();
    let handoff = from_conversation_draft(&messages, draft, 12_000).unwrap();
    let selected = handoff.select().unwrap();
    assert_eq!(selected.sources.len(), 6);
    assert_eq!(selected.sources[0].reference, "message:message-0");
    assert_eq!(selected.sources[0].text, messages[0].text);
}

#[test]
fn local_draft_cannot_invent_quote_or_hide_low_confidence() {
    let (messages, mut draft) = conversation_fixture();
    draft.candidates[0].quote = "invented paraphrase".into();
    assert!(from_conversation_draft(&messages, draft, 12_000).is_err());

    let (messages, mut draft) = conversation_fixture();
    draft.candidates[0].confidence = 0.74;
    assert!(from_conversation_draft(&messages, draft, 12_000).is_err());
}

#[test]
fn local_draft_correction_must_point_to_earlier_source_message() {
    let (messages, mut draft) = conversation_fixture();
    draft.candidates[0].corrects = vec!["candidate-1".into()];
    assert!(from_conversation_draft(&messages, draft, 12_000).is_err());

    let (messages, mut draft) = conversation_fixture();
    draft.candidates[1].corrects = vec!["candidate-0".into()];
    assert!(from_conversation_draft(&messages, draft, 12_000).is_ok());
}
