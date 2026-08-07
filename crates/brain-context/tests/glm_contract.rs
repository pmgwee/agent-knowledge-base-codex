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
    let validated = validate_proposed_batch(&packet, batch).expect("validate response");
    assert_eq!(validated.accepted.len(), 1);
    assert!(validated.rejected.is_empty());
    assert_eq!(
        validated.accepted[0].scope,
        MemoryScope::Project(packet.project_id)
    );
    assert_eq!(validated.accepted[0].kind, MemoryKind::Checkpoint);
    assert_eq!(validated.accepted[0].evidence_ids, vec![event_id]);
}

#[test]
fn a_rejected_response_says_which_field_was_wrong_not_just_that_one_was() {
    // Three jobs dead-lettered on the live brain recording 400 characters of plausible-looking
    // JSON and no reason at all. The content came through; serde's own message — the half that
    // names the field — was dropped, so the record showed the evidence and withheld the verdict.
    //
    // Diagnosing it cost a reproduction run against a provider that was rate-limited at the
    // time. The failure is written to a job row nobody can re-ask, so whatever the row does not
    // say is not recoverable later.
    let missing_valid_from = serde_json::json!({
        "choices": [{"message": {"content": serde_json::to_string(&serde_json::json!({
            "memories": [{
                "kind": "fact",
                "title": "a memory missing a required field",
                "content": "valid_from is absent",
                "confidence": 0.9,
                "evidence_ids": ["00000000-0000-0000-0000-000000000001"]
            }]
        })).expect("fixture json")}}]
    })
    .to_string();

    let error = parse_glm_chat_response(&missing_valid_from)
        .expect_err("a response missing a required field must not parse");
    let text = format!("{error:#}");

    assert!(
        text.contains("valid_from"),
        "the error must name the field that was wrong, got: {text}"
    );
    assert!(
        text.contains("a memory missing a required field"),
        "and must still carry what the model actually wrote, got: {text}"
    );
}

#[test]
fn unknown_llm_evidence_ids_never_become_records() {
    // The integrity rule is absolute and unchanged: a memory citing evidence outside its packet
    // is never stored. Only the blast radius changed, from the batch to the memory.
    let packet = packet(uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap());
    let batch = parse_glm_chat_response(include_str!(
        "../../../fixtures/providers/glm-invalid-response.json"
    ))
    .expect("parse structurally valid response");
    let validated = validate_proposed_batch(&packet, batch).expect("validation is not fatal");
    assert!(
        validated.accepted.is_empty(),
        "invented evidence must not be stored"
    );
    assert_eq!(validated.rejected.len(), 1);
    assert!(
        validated.rejected[0].contains("unknown evidence"),
        "the reason must say why, got {:?}",
        validated.rejected[0]
    );
}

#[test]
fn one_bad_memory_does_not_discard_the_good_ones() {
    // The defect this guards, seen on the live brain: a provider returned several sound
    // memories alongside one citing an invented event id, and all of them were discarded.
    // Because the request is made at temperature 0, each retry reproduced the same output, so
    // the job exhausted its attempts and dead-lettered work that was never in question.
    let known = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").expect("fixture");
    let packet = packet(known);
    let batch: ProposedMemoryBatch = serde_json::from_value(serde_json::json!({
        "memories": [
            {
                "kind": "fact", "title": "sound memory", "content": "grounded in real evidence",
                "valid_from": "2026-08-02T01:00:00Z", "confidence": 0.9,
                "evidence_ids": [known], "supersedes": []
            },
            {
                "kind": "fact", "title": "invented citation", "content": "cites nothing real",
                "valid_from": "2026-08-02T01:00:00Z", "confidence": 0.9,
                "evidence_ids": ["00000000-0000-0000-0000-0000000000ff"], "supersedes": []
            }
        ]
    }))
    .expect("parse proposed batch");

    let validated = validate_proposed_batch(&packet, batch).expect("validation is not fatal");
    assert_eq!(validated.accepted.len(), 1, "the sound memory must survive");
    assert_eq!(validated.accepted[0].title, "sound memory");
    assert_eq!(validated.rejected.len(), 1, "the unsound one must not");
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
    let oversized = validate_proposed_batch(&packet, batch.clone()).expect("not fatal");
    assert!(oversized.accepted.is_empty());
    assert_eq!(oversized.rejected.len(), 1);

    batch.memories[0].title = "valid".to_owned();
    batch.memories[0].confidence = f32::NAN;
    let not_a_number = validate_proposed_batch(&packet, batch).expect("not fatal");
    assert!(not_a_number.accepted.is_empty());
    assert_eq!(not_a_number.rejected.len(), 1);
}

#[test]
fn a_failed_parse_quotes_the_output_that_failed() {
    // "does not match the schema" alone costs a reproduction run against the live provider to
    // learn which field was wrong — and the failure is recorded on a job, where the model can
    // no longer be asked what it said.
    let response = serde_json::json!({
        "choices": [{"message": {"content": "{\"memories\":[{\"kind\":\"project\"}]}"}}]
    })
    .to_string();
    let error = parse_glm_chat_response(&response).expect_err("kind is not a valid variant");
    let text = format!("{error:#}");
    assert!(
        text.contains("project"),
        "the error must quote the offending output, got {text}"
    );
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
