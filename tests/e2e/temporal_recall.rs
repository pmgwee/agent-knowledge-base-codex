mod temporal_fixture;

use std::collections::HashSet;

use brain_service::{
    BrainCheckpointResponse, BrainCorrectionResponse, BrainEvidenceResponse, BrainItemsResponse,
};
use temporal_fixture::{TemporalFixture, call};

#[test]
fn last_week_returns_cited_cross_agent_work_and_temporal_roles() {
    let fixture = TemporalFixture::seeded();
    let mut server = fixture.server();
    let value = call(
        &mut server,
        1,
        "brain_timeline",
        serde_json::json!({
            "project": fixture.project_id.0,
            "window": "week",
            "now": rfc3339(fixture.now),
            "limit": 50
        }),
    );
    let result: BrainItemsResponse = serde_json::from_value(value).expect("timeline response");

    let harnesses = result
        .items
        .iter()
        .filter_map(|item| item.harness.as_ref().map(|harness| harness.as_str()))
        .collect::<HashSet<_>>();
    assert!(harnesses.contains("claude-code"), "{harnesses:?}");
    assert!(harnesses.contains("codex"), "{harnesses:?}");
    assert!(harnesses.contains("hermes"), "{harnesses:?}");
    assert!(result.items.iter().all(|item| !item.citations.is_empty()));
    assert!(
        result
            .items
            .iter()
            .any(|item| item.temporal_role == "historical_evidence")
    );
    assert!(result.items.iter().any(|item| item.late_observation));

    let event_reference = result
        .items
        .iter()
        .find(|item| item.source == "event")
        .expect("event result")
        .reference
        .clone();
    let evidence: BrainEvidenceResponse = serde_json::from_value(call(
        &mut server,
        2,
        "brain_evidence",
        serde_json::json!({"project": fixture.project_id.0, "reference": event_reference}),
    ))
    .expect("evidence response");
    assert!(!evidence.citations.is_empty());

    let checkpoint: BrainCheckpointResponse = serde_json::from_value(call(
        &mut server,
        3,
        "brain_checkpoint",
        serde_json::json!({
            "project": fixture.project_id.0,
            "prompt": "Continue OAuth callback work"
        }),
    ))
    .expect("checkpoint response");
    assert!(checkpoint.context.token_count <= 1_500);
    assert!(!checkpoint.context.citations.is_empty());
}

#[test]
fn a_correction_is_idempotent_audited_and_reverses_current_truth_without_erasing_history() {
    let fixture = TemporalFixture::seeded();
    let mut server = fixture.server();
    let correction_id = uuid::Uuid::now_v7();
    let arguments = serde_json::json!({
        "project": fixture.project_id.0,
        "correction_id": correction_id,
        "memory_id": fixture.old_memory_id,
        "kind": "decision",
        "title": "Auth storage decision",
        "content": "Use encrypted session auth storage",
        "valid_from": rfc3339(fixture.now - time::Duration::days(2))
    });
    let first: BrainCorrectionResponse =
        serde_json::from_value(call(&mut server, 1, "brain_correct", arguments.clone()))
            .expect("first correction");
    let replay: BrainCorrectionResponse =
        serde_json::from_value(call(&mut server, 2, "brain_correct", arguments))
            .expect("replayed correction");
    assert!(!first.replayed);
    assert!(replay.replayed);
    assert_eq!(first.memory.version_id, replay.memory.version_id);
    assert!(first.memory.evidence_ids.contains(&correction_id));
    assert_eq!(
        fixture.ledger().memory_version_count().expect("versions"),
        2
    );

    let current: BrainItemsResponse = serde_json::from_value(call(
        &mut server,
        3,
        "brain_search",
        serde_json::json!({
            "project": fixture.project_id.0,
            "text": "auth storage",
            "source": "memories"
        }),
    ))
    .expect("current search");
    assert_eq!(current.items.len(), 1);
    assert!(current.items[0].text.contains("encrypted session"));
    assert_eq!(current.items[0].temporal_role, "current_memory");

    let historical: BrainItemsResponse = serde_json::from_value(call(
        &mut server,
        4,
        "brain_search",
        serde_json::json!({
            "project": fixture.project_id.0,
            "text": "auth storage",
            "source": "memories",
            "as_of": rfc3339(fixture.now - time::Duration::days(3))
        }),
    ))
    .expect("historical search");
    assert_eq!(historical.items.len(), 1);
    assert!(historical.items[0].text.contains("cookie auth"));
    assert_eq!(historical.items[0].temporal_role, "as_of_memory");
    assert_eq!(
        historical.items[0].reference,
        format!("memory:{}", fixture.old_version_id)
    );
}

fn rfc3339(value: time::OffsetDateTime) -> String {
    value
        .format(&time::format_description::well_known::Rfc3339)
        .expect("fixture timestamp is RFC3339 compatible")
}
