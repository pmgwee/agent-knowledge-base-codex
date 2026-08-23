use brain_cli::{
    AcceptableEvidence, RetrievalChannel, RetrievalGoldCase, RetrievalSplit,
    evaluate_retrieval_cases, evaluate_retrieval_fixture_cases, read_gold_cases,
};
use brain_domain::{
    Authority, EventBatch, EventType, Harness, MemoryKind, MemoryRecord, MemoryScope, MemoryStatus,
    NormalizedEvent, ProjectId, SourceCursor, WorktreeId,
};
use brain_store::EventLedger;

fn at(seconds: i64) -> time::OffsetDateTime {
    time::OffsetDateTime::from_unix_timestamp(1_700_000_000 + seconds).unwrap()
}

fn append_event(
    ledger: &mut EventLedger,
    project_id: ProjectId,
    event_id: uuid::Uuid,
    content: &str,
    offset: i64,
    occurred_at: time::OffsetDateTime,
) {
    let mut hash = [0_u8; 32];
    hash[..8].copy_from_slice(&offset.to_le_bytes());
    ledger
        .append_batch(&EventBatch {
            source_id: "retrieval-gold".to_owned(),
            events: vec![NormalizedEvent {
                event_id,
                project_id,
                worktree_id: WorktreeId(uuid::Uuid::nil()),
                task_id: None,
                harness: Harness::Codex,
                native_session_id: "gold-session".to_owned(),
                native_turn_id: None,
                event_type: EventType::UserPrompted,
                occurred_at,
                observed_at: occurred_at,
                source_locator: "history/gold.jsonl".to_owned(),
                source_offset: offset,
                source_schema: "benchmark:v1".to_owned(),
                raw_hash: hash,
                idempotency_key: hash,
                git_head: None,
                git_branch: None,
                payload: serde_json::json!({"content": content}),
                raw: serde_json::json!({"content": content}),
            }],
            quarantined: Vec::new(),
            capture_gaps: Vec::new(),
            next_cursor: SourceCursor::start(),
        })
        .unwrap();
}

fn case(
    id: &str,
    channel: RetrievalChannel,
    query: &str,
    expected_fact: &str,
    event_id: uuid::Uuid,
    offset: i64,
) -> RetrievalGoldCase {
    RetrievalGoldCase {
        id: id.to_owned(),
        schema_version: 1,
        split: RetrievalSplit::Calibration,
        channel,
        project_alias: "fixture-project-a".to_owned(),
        query: query.to_owned(),
        as_of: at(100),
        expected_facts: vec![expected_fact.to_owned()],
        acceptable_evidence: vec![AcceptableEvidence {
            event_id,
            source_locator: "history/gold.jsonl".to_owned(),
            source_offset: offset,
        }],
        prohibited_facts: vec!["foreign project secret".to_owned()],
        expect_abstention: false,
    }
}

#[test]
fn historical_pull_scores_exact_resolved_evidence_and_percentages() {
    let project_id = ProjectId(uuid::Uuid::now_v7());
    let mut ledger = EventLedger::open_in_memory(project_id).unwrap();
    let event_id = uuid::Uuid::now_v7();
    append_event(
        &mut ledger,
        project_id,
        event_id,
        "the release codename is heliotrope",
        41,
        at(10),
    );

    let report = evaluate_retrieval_cases(
        &ledger,
        project_id,
        &[case(
            "historical-1",
            RetrievalChannel::HistoricalPull,
            "release codename heliotrope",
            "release codename is heliotrope",
            event_id,
            41,
        )],
        "exe-hash",
        "config-hash",
    )
    .unwrap();

    assert_eq!(report.valid_cases, 1);
    assert_eq!(report.invalid_cases, 0);
    assert_eq!(report.metrics.precision.percent, Some(100.0));
    assert_eq!(report.metrics.recall.percent, Some(100.0));
    assert_eq!(report.metrics.mrr.percent, Some(100.0));
    assert_eq!(report.metrics.fact_accuracy.percent, Some(100.0));
    assert_eq!(
        report.cases[0].returned_citations[0],
        format!("event:{event_id}")
    );
}

#[test]
fn a_gold_offset_mismatch_is_invalid_instead_of_receiving_credit() {
    let project_id = ProjectId(uuid::Uuid::now_v7());
    let mut ledger = EventLedger::open_in_memory(project_id).unwrap();
    let event_id = uuid::Uuid::now_v7();
    append_event(&mut ledger, project_id, event_id, "known fact", 7, at(10));
    let report = evaluate_retrieval_cases(
        &ledger,
        project_id,
        &[case(
            "bad-offset",
            RetrievalChannel::HistoricalPull,
            "known fact",
            "known fact",
            event_id,
            8,
        )],
        "exe-hash",
        "config-hash",
    )
    .unwrap();

    assert_eq!(report.valid_cases, 0);
    assert_eq!(report.invalid_cases, 1);
    assert!(
        report.cases[0]
            .invalid_reason
            .as_deref()
            .unwrap()
            .contains("offset")
    );
    assert_eq!(report.metrics.recall.percent, None);
}

#[test]
fn prompt_push_negative_credits_healthy_silence_and_never_calls_it_zero_recall() {
    let project_id = ProjectId(uuid::Uuid::now_v7());
    let ledger = EventLedger::open_in_memory(project_id).unwrap();
    let negative = RetrievalGoldCase {
        id: "negative-push".to_owned(),
        schema_version: 1,
        split: RetrievalSplit::LockedTest,
        channel: RetrievalChannel::PromptPush,
        project_alias: "fixture-project-a".to_owned(),
        query: "ok".to_owned(),
        as_of: at(100),
        expected_facts: Vec::new(),
        acceptable_evidence: Vec::new(),
        prohibited_facts: vec!["anything unsolicited".to_owned()],
        expect_abstention: true,
    };
    let report =
        evaluate_retrieval_cases(&ledger, project_id, &[negative], "exe-hash", "config-hash")
            .unwrap();

    assert_eq!(report.metrics.healthy_silence.percent, Some(100.0));
    assert_eq!(report.metrics.harmful_push.percent, Some(0.0));
    assert_eq!(report.metrics.recall.percent, None);
    assert!(report.cases[0].abstained);
}

#[test]
fn evidence_newer_than_the_cutoff_is_refused_even_when_the_query_matches() {
    let project_id = ProjectId(uuid::Uuid::now_v7());
    let mut ledger = EventLedger::open_in_memory(project_id).unwrap();
    let event_id = uuid::Uuid::now_v7();
    append_event(
        &mut ledger,
        project_id,
        event_id,
        "future answer",
        12,
        at(101),
    );
    let report = evaluate_retrieval_cases(
        &ledger,
        project_id,
        &[case(
            "future",
            RetrievalChannel::SessionStart,
            "future answer",
            "future answer",
            event_id,
            12,
        )],
        "exe-hash",
        "config-hash",
    )
    .unwrap();

    assert_eq!(report.invalid_cases, 1);
    assert!(
        report.cases[0]
            .invalid_reason
            .as_deref()
            .unwrap()
            .contains("cutoff")
    );
}

#[test]
fn deterministic_evaluation_does_not_mutate_memory_access_state() {
    let project_id = ProjectId(uuid::Uuid::now_v7());
    let mut ledger = EventLedger::open_in_memory(project_id).unwrap();
    let event_id = uuid::Uuid::now_v7();
    append_event(
        &mut ledger,
        project_id,
        event_id,
        "the release codename is heliotrope",
        55,
        at(10),
    );
    let memory = MemoryRecord {
        id: uuid::Uuid::now_v7(),
        version_id: uuid::Uuid::now_v7(),
        scope: MemoryScope::Project(project_id),
        worktree_id: None,
        task_id: None,
        kind: MemoryKind::Fact,
        title: "Release codename heliotrope".to_owned(),
        content: "The release codename is heliotrope.".to_owned(),
        valid_from: at(10),
        valid_to: None,
        recorded_at: at(10),
        confidence: 1.0,
        authority: Authority::DerivedMemory,
        evidence_ids: vec![event_id],
        supersedes: Vec::new(),
        status: MemoryStatus::Current,
    };
    ledger.append_memory(&memory).unwrap();

    let report = evaluate_retrieval_cases(
        &ledger,
        project_id,
        &[case(
            "read-only-push",
            RetrievalChannel::PromptPush,
            "please recall the release codename heliotrope",
            "release codename is heliotrope",
            event_id,
            55,
        )],
        "exe-hash",
        "config-hash",
    )
    .unwrap();

    assert_eq!(report.metrics.recall.percent, Some(100.0));
    assert!(ledger.memory_access(memory.id).unwrap().is_none());
}

#[test]
fn preregistered_calibration_fixture_is_fully_materialized_and_executable() {
    let gold = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../benchmarks/second-brain/v2/retrieval-gold/calibration.jsonl");
    let cases = read_gold_cases(&gold, RetrievalSplit::Calibration).expect("calibration gold");
    let report = evaluate_retrieval_fixture_cases(
        ProjectId(uuid::Uuid::now_v7()),
        &cases,
        "fixture-executable",
        "fixture-config",
    )
    .expect("fixture evaluation");

    assert!(report.fixture_generated);
    assert_eq!(report.case_count, 120);
    assert_eq!(report.valid_cases, 120);
    assert_eq!(report.invalid_cases, 0);
    assert_eq!(report.metrics.freshness.percent, Some(100.0));
    assert_eq!(report.metrics.harmful_push.percent, Some(0.0));
    assert_eq!(report.metrics.healthy_silence.percent, Some(100.0));
}
