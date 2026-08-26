use brain_context::{
    EvidencePacket, ProposedMemoryBatch, RedactedEvidence, parse_llm_response,
    validate_proposed_batch,
};
use brain_domain::{EventType, MemoryKind, MemoryScope, ProjectId};

#[test]
fn valid_provider_json_becomes_project_scoped_cited_memory() {
    let event_id =
        uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").expect("fixture event ID");
    let packet = packet(event_id);
    let batch = parse_llm_response(include_str!(
        "../../../fixtures/providers/llm-memory-response.json"
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
    let missing_valid_from = response_envelope(
        &serde_json::to_string(&serde_json::json!({
            "memories": [{
                "kind": "fact",
                "title": "a memory missing a required field",
                "content": "valid_from is absent",
                "confidence": 0.9,
                "evidence_ids": ["00000000-0000-0000-0000-000000000001"]
            }]
        }))
        .expect("fixture json"),
    );

    let error = parse_llm_response(&missing_valid_from)
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
    let batch = parse_llm_response(include_str!(
        "../../../fixtures/providers/llm-invalid-response.json"
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
    // Retrying the whole batch discarded the same sound proposal repeatedly, so the job exhausted
    // its attempts and dead-lettered work that was never in question.
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
    assert!(parse_llm_response("{not-json").is_err());
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
    let response = response_envelope("{\"memories\":[{\"kind\":\"project\"}]}");
    let error = parse_llm_response(&response).expect_err("kind is not a valid variant");
    let text = format!("{error:#}");
    assert!(
        text.contains("project"),
        "the error must quote the offending output, got {text}"
    );
}

/// A minimal Responses API envelope carrying one assistant text part.
fn response_envelope(text: &str) -> String {
    serde_json::json!({
        "status": "completed",
        "output": [{
            "type": "message",
            "role": "assistant",
            "content": [{"type": "output_text", "text": text}]
        }]
    })
    .to_string()
}

fn packet(event_id: uuid::Uuid) -> EvidencePacket {
    EvidencePacket {
        job_id: uuid::Uuid::now_v7(),
        project_id: ProjectId(uuid::Uuid::now_v7()),
        trigger: None,
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

// ---------------------------------------------------------------------------
// Provider configuration
// ---------------------------------------------------------------------------

#[test]
fn the_documented_path_is_appended_to_the_base_url_exactly_once() {
    // The documented HTTP endpoint ends in `/v1/responses` while the configured value is the
    // root. Both halves of that are easy to get wrong in the same direction, and the result is a
    // 404 that reads like a provider outage rather than a configuration mistake.
    assert_eq!(
        brain_context::responses_url("https://opencode.ai/zen/go/v1").expect("base URL"),
        "https://opencode.ai/zen/go/v1/responses"
    );
    assert_eq!(
        brain_context::responses_url("https://opencode.ai/zen/go/v1/").expect("trailing slash"),
        "https://opencode.ai/zen/go/v1/responses"
    );
    // Someone pasting the documented endpoint instead of the root must not get
    // `/responses/responses`.
    assert_eq!(
        brain_context::responses_url("https://opencode.ai/zen/go/v1/responses")
            .expect("complete endpoint"),
        "https://opencode.ai/zen/go/v1/responses"
    );
}

#[test]
fn a_chat_completions_base_url_is_refused_rather_than_mangled() {
    // What the previous provider's configuration held. Appending `/responses` to it would build
    // `/chat/completions/responses`, so it is named as the mistake it is.
    let error = brain_context::responses_url("https://example.invalid/v1/chat/completions")
        .expect_err("a Chat Completions endpoint is not a base URL");
    let text = format!("{error:#}");
    assert!(
        text.contains("chat/completions"),
        "the error must say what to remove, got: {text}"
    );
}

#[test]
fn base_url_metadata_cannot_change_the_responses_route() {
    for (label, value) in [
        ("query", "https://example.invalid/v1?route=legacy"),
        ("fragment", "https://example.invalid/v1#responses"),
        ("credentials", "https://user:secret@example.invalid/v1"),
    ] {
        let error = brain_context::responses_url(value)
            .expect_err("URL metadata must not be carried into an LLM endpoint");
        assert!(
            format!("{error:#}").to_lowercase().contains(label),
            "the error should identify the rejected {label}: {error:#}"
        );
    }
}

#[test]
fn provider_configuration_is_validated_before_any_request_is_made() {
    let sound = || brain_context::LlmConfig {
        base_url: "https://opencode.ai/zen/go/v1".to_owned(),
        model: "gpt-5.6-luna".to_owned(),
        api_key_env: "LLM_API_KEY".to_owned(),
        timeout: std::time::Duration::from_secs(30),
        max_retries: 2,
    };
    let client = brain_context::LlmClient::new(sound()).expect("a sound configuration builds");
    assert_eq!(
        client.responses_url(),
        "https://opencode.ai/zen/go/v1/responses"
    );
    assert_eq!(client.model(), "gpt-5.6-luna");

    for (label, broken) in [
        (
            "empty base URL",
            brain_context::LlmConfig {
                base_url: String::new(),
                ..sound()
            },
        ),
        (
            "non-HTTP scheme",
            brain_context::LlmConfig {
                base_url: "ftp://example.invalid/v1".to_owned(),
                ..sound()
            },
        ),
        (
            "empty model",
            brain_context::LlmConfig {
                model: "  ".to_owned(),
                ..sound()
            },
        ),
        (
            "empty key variable name",
            brain_context::LlmConfig {
                api_key_env: String::new(),
                ..sound()
            },
        ),
        (
            "zero timeout",
            brain_context::LlmConfig {
                timeout: std::time::Duration::ZERO,
                ..sound()
            },
        ),
    ] {
        let Err(error) = brain_context::LlmClient::new(broken) else {
            panic!("{label} must not build a client");
        };
        assert!(
            !error.to_string().is_empty(),
            "{label} must fail with a stated reason"
        );
    }
}

// ---------------------------------------------------------------------------
// The Responses envelope
// ---------------------------------------------------------------------------

#[test]
fn a_reasoning_item_before_the_message_does_not_hide_the_answer() {
    // `output` is a list and the message is not reliably its first element. Indexing it — which
    // is what porting `/choices/0/message/content` across literally would produce — works right
    // up until the model emits a reasoning item, and then returns nothing on a request that
    // succeeded. The memory fixture carries one for exactly this reason.
    let batch = parse_llm_response(include_str!(
        "../../../fixtures/providers/llm-memory-response.json"
    ))
    .expect("the message is found past the reasoning item");
    assert_eq!(batch.memories.len(), 1);
}

#[test]
fn a_chat_completions_envelope_is_no_longer_accepted() {
    // The old shape must not quietly keep working: if it did, a half-migrated deployment still
    // pointed at the old endpoint would look healthy.
    let old_shape = serde_json::json!({
        "choices": [{"message": {"content": "{\"memories\":[]}"}}]
    })
    .to_string();
    let error = parse_llm_response(&old_shape).expect_err("a Chat Completions envelope must fail");
    assert!(
        format!("{error:#}").contains("no assistant text"),
        "got: {error:#}"
    );
}

#[test]
fn an_incomplete_response_names_truncation_rather_than_the_schema() {
    // A response cut short carries a partial body. Reported as "does not match the schema" it
    // would be true and useless — and the retry that follows is pointless, because what a
    // truncation needs is a smaller packet.
    let truncated = serde_json::json!({
        "status": "incomplete",
        "incomplete_details": {"reason": "max_output_tokens"},
        "output": [{
            "type": "message",
            "role": "assistant",
            "content": [{"type": "output_text", "text": "{\"memories\":[{\"kind\":"}]
        }]
    })
    .to_string();
    let error = parse_llm_response(&truncated).expect_err("an incomplete response is an error");
    let text = format!("{error:#}");
    assert!(text.contains("incomplete"), "got: {text}");
    assert!(text.contains("max_output_tokens"), "got: {text}");
}

#[test]
fn a_provider_error_envelope_is_reported_as_the_provider_error_it_is() {
    let refused = serde_json::json!({
        "error": {"type": "invalid_request_error", "message": "unsupported parameter"}
    })
    .to_string();
    let error = parse_llm_response(&refused).expect_err("an error envelope is an error");
    assert!(
        format!("{error:#}").contains("unsupported parameter"),
        "got: {error:#}"
    );
}

#[test]
fn an_envelope_carrying_no_text_is_an_error_not_an_empty_batch() {
    // Silently returning zero memories would be indistinguishable from a session that produced
    // nothing worth keeping, which is a normal outcome — so the failure would never be noticed.
    let empty = serde_json::json!({
        "status": "completed",
        "output": [{"type": "reasoning", "summary": []}]
    })
    .to_string();
    assert!(parse_llm_response(&empty).is_err());
}
