use brain_context::{
    EvidencePacket, ProposedMemoryBatch, RedactedEvidence, parse_glm_chat_response,
    validate_proposed_batch,
};
use brain_domain::{EventType, MemoryKind, MemoryScope, ProjectId};

#[test]
fn valid_glm_json_becomes_project_scoped_cited_memory() {
    let event_id =
        uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").expect("fixture event ID");
    let packet = packet(event_id);
    let batch = parse_glm_chat_response(include_str!(
        "../../../fixtures/providers/glm-memory-response.json"
    ))
    .expect("parse valid response");
    let memories = validate_proposed_batch(&packet, batch).expect("validate response");
    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0].scope, MemoryScope::Project(packet.project_id));
    assert_eq!(memories[0].kind, MemoryKind::Checkpoint);
    assert_eq!(memories[0].evidence_ids, vec![event_id]);
}

#[test]
fn unknown_llm_evidence_ids_are_rejected() {
    let packet = packet(uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap());
    let batch = parse_glm_chat_response(include_str!(
        "../../../fixtures/providers/glm-invalid-response.json"
    ))
    .expect("parse structurally valid response");
    let error = validate_proposed_batch(&packet, batch).expect_err("reject invented evidence");
    assert!(error.to_string().contains("unknown evidence"));
}

#[test]
fn malformed_or_oversized_provider_output_is_rejected() {
    assert!(parse_glm_chat_response("{not-json").is_err());
    let packet = packet(uuid::Uuid::now_v7());
    let mut batch: ProposedMemoryBatch = serde_json::from_value(serde_json::json!({
        "memories": [{
            "kind": "fact",
            "title": "x".repeat(301),
            "content": "content",
            "valid_from": "1970-01-01T00:00:00Z",
            "confidence": 1.0,
            "evidence_ids": [packet.events[0].event_id],
            "supersedes": []
        }]
    }))
    .expect("parse proposed batch");
    assert!(validate_proposed_batch(&packet, batch.clone()).is_err());
    batch.memories[0].title = "valid".to_owned();
    batch.memories[0].confidence = f32::NAN;
    assert!(validate_proposed_batch(&packet, batch).is_err());
}

fn packet(event_id: uuid::Uuid) -> EvidencePacket {
    EvidencePacket {
        job_id: uuid::Uuid::now_v7(),
        project_id: ProjectId(uuid::Uuid::now_v7()),
        events: vec![RedactedEvidence {
            event_id,
            event_type: EventType::AgentResponded,
            occurred_at: time::OffsetDateTime::parse(
                "2026-08-02T01:00:00Z",
                &time::format_description::well_known::Rfc3339,
            )
            .expect("fixture time"),
            payload: serde_json::json!({"content": "OAuth callback tests pass"}),
            raw: serde_json::json!({}),
        }],
        redactions: Vec::new(),
        allowed_supersession_ids: Vec::new(),
    }
}
