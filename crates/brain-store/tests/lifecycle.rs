use brain_domain::{Harness, ProjectId};
use brain_store::{
    EventLedger, LifecycleChannel, LifecycleEvent, LifecycleStage, RetrievalDecision,
    RetrievalOutcome, RetrievalReasonCode, SessionAttribution, TelemetryQuery, fold_lifecycle,
    fold_retrieval_decisions,
};

fn project() -> ProjectId {
    ProjectId(uuid::Uuid::now_v7())
}

fn at(seconds: i64) -> time::OffsetDateTime {
    time::OffsetDateTime::from_unix_timestamp(1_700_000_000 + seconds).unwrap()
}

fn lifecycle_event(project_id: ProjectId, stage: LifecycleStage) -> LifecycleEvent {
    LifecycleEvent {
        event_id: uuid::Uuid::now_v7(),
        project_id,
        harness: Harness::Codex,
        session: SessionAttribution::Attributed("session-a".to_owned()),
        correlation_id: Some("correlation-a".to_owned()),
        channel: LifecycleChannel::SessionStart,
        stage,
        occurred_at: at(100),
        detail: serde_json::json!({"receipt": "fixture"}),
    }
}

fn decision(project_id: ProjectId) -> RetrievalDecision {
    RetrievalDecision {
        decision_id: uuid::Uuid::now_v7(),
        project_id,
        harness: Harness::Codex,
        session: SessionAttribution::Attributed("session-a".to_owned()),
        correlation_id: Some("correlation-a".to_owned()),
        channel: LifecycleChannel::UserPromptSubmit,
        outcome: RetrievalOutcome::HealthySilence,
        reason_code: RetrievalReasonCode::NoRelevantCandidate,
        candidate_count: 4,
        selected_count: 0,
        dropped_count: 4,
        token_count: 0,
        latency_ms: 9,
        query_sha256: "a".repeat(64),
        selected_evidence_ids: Vec::new(),
        occurred_at: at(101),
    }
}

#[test]
fn lifecycle_and_retrieval_receipts_are_idempotent_but_never_mutable() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("ledger.sqlite3");
    let project_id = project();
    let ledger = EventLedger::open(&path, project_id).unwrap();
    let event = lifecycle_event(project_id, LifecycleStage::HookReceived);
    assert!(ledger.record_lifecycle_event(&event).unwrap());
    assert!(!ledger.record_lifecycle_event(&event).unwrap());
    let mut conflict = event.clone();
    conflict.detail = serde_json::json!({"receipt": "conflicting"});
    assert!(
        ledger
            .record_lifecycle_event(&conflict)
            .unwrap_err()
            .to_string()
            .contains("conflict")
    );

    let retrieval = decision(project_id);
    assert!(ledger.record_retrieval_decision(&retrieval).unwrap());
    assert!(!ledger.record_retrieval_decision(&retrieval).unwrap());
    let mut conflict = retrieval.clone();
    conflict.reason_code = RetrievalReasonCode::BudgetDrop;
    assert!(
        ledger
            .record_retrieval_decision(&conflict)
            .unwrap_err()
            .to_string()
            .contains("conflict")
    );
    drop(ledger);

    let connection = rusqlite::Connection::open(path).unwrap();
    assert!(
        connection
            .execute("UPDATE lifecycle_events SET stage = 'reply_flushed'", [])
            .is_err()
    );
    assert!(
        connection
            .execute("DELETE FROM retrieval_decisions", [])
            .is_err()
    );
}

#[test]
fn reads_are_project_session_and_time_bounded_and_folds_preserve_healthy_silence() {
    let project_id = project();
    let other_project = project();
    let ledger = EventLedger::open_in_memory(project_id).unwrap();
    let received = lifecycle_event(project_id, LifecycleStage::HookReceived);
    let mut flushed = lifecycle_event(project_id, LifecycleStage::ReplyFlushed);
    flushed.occurred_at = at(102);
    ledger.record_lifecycle_event(&received).unwrap();
    ledger.record_lifecycle_event(&flushed).unwrap();
    ledger
        .record_retrieval_decision(&decision(project_id))
        .unwrap();

    let query = TelemetryQuery {
        project_id,
        session: Some(SessionAttribution::Attributed("session-a".to_owned())),
        start: at(0),
        end: at(200),
        limit: 100,
    };
    let events = ledger.lifecycle_events(&query).unwrap();
    assert_eq!(events.len(), 2);
    let lifecycle = fold_lifecycle(&events);
    assert_eq!(lifecycle.stage_counts[&LifecycleStage::HookReceived], 1);
    assert_eq!(lifecycle.stage_counts[&LifecycleStage::ReplyFlushed], 1);

    let decisions = ledger.retrieval_decisions(&query).unwrap();
    let retrieval = fold_retrieval_decisions(&decisions);
    assert_eq!(retrieval.total, 1);
    assert_eq!(retrieval.healthy_silence, 1);
    assert_eq!(retrieval.delivered, 0);

    let mut other = received.clone();
    other.event_id = uuid::Uuid::now_v7();
    other.project_id = other_project;
    assert!(ledger.record_lifecycle_event(&other).is_err());
    let mut wrong_query = query.clone();
    wrong_query.project_id = other_project;
    assert!(ledger.lifecycle_events(&wrong_query).is_err());
}

#[test]
fn required_reason_codes_and_unattributed_sessions_round_trip() {
    let required = [
        RetrievalReasonCode::ShortPrompt,
        RetrievalReasonCode::MissingSessionId,
        RetrievalReasonCode::SessionMemoryCap,
        RetrievalReasonCode::NoRelevantCandidate,
        RetrievalReasonCode::BudgetDrop,
        RetrievalReasonCode::RetrievalError,
        RetrievalReasonCode::Timeout,
        RetrievalReasonCode::NotRequested,
    ];
    assert_eq!(
        required.map(|reason| reason.as_str()),
        [
            "short_prompt",
            "missing_session_id",
            "session_memory_cap",
            "no_relevant_candidate",
            "budget_drop",
            "retrieval_error",
            "timeout",
            "not_requested",
        ]
    );

    let project_id = project();
    let ledger = EventLedger::open_in_memory(project_id).unwrap();
    let mut event = lifecycle_event(project_id, LifecycleStage::McpRequest);
    event.session = SessionAttribution::Unattributed;
    event.channel = LifecycleChannel::BrainMcp;
    ledger.record_lifecycle_event(&event).unwrap();
    let query = TelemetryQuery {
        project_id,
        session: Some(SessionAttribution::Unattributed),
        start: at(0),
        end: at(1_000),
        limit: 10,
    };
    assert_eq!(
        ledger.lifecycle_events(&query).unwrap()[0].session,
        SessionAttribution::Unattributed
    );
}

#[test]
fn opening_a_version_nine_ledger_adds_lifecycle_tables_without_rewriting_history() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("old.sqlite3");
    {
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection.execute_batch(
            "CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL);\
             INSERT INTO schema_migrations(version, applied_at) VALUES (9, 'fixture');",
        ).unwrap();
    }
    let project_id = project();
    let ledger = EventLedger::open(&path, project_id).unwrap();
    let event = lifecycle_event(project_id, LifecycleStage::HookReceived);
    ledger.record_lifecycle_event(&event).unwrap();
    drop(ledger);
    let connection = rusqlite::Connection::open(path).unwrap();
    let version: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM schema_migrations WHERE version = 10",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(version, 1);
}
