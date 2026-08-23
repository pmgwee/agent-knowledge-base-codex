use brain_cli::{
    ChannelState, SessionActivity, SessionFilter, SessionLifecycleState, SessionStatusOptions,
    fold_session_status, read_session_status,
};
use brain_domain::{
    EventBatch, EventType, Harness, NormalizedEvent, ProjectId, SourceCursor, WorktreeId,
};
use brain_store::{
    EventLedger, LifecycleChannel, LifecycleEvent, LifecycleStage, RetrievalDecision,
    RetrievalOutcome, RetrievalReasonCode, SessionAttribution,
};

fn at(minutes: i64) -> time::OffsetDateTime {
    time::OffsetDateTime::from_unix_timestamp(1_700_000_000 + minutes * 60).unwrap()
}

fn event(
    project_id: ProjectId,
    session: &str,
    channel: LifecycleChannel,
    stage: LifecycleStage,
    minute: i64,
) -> LifecycleEvent {
    LifecycleEvent {
        event_id: uuid::Uuid::now_v7(),
        project_id,
        harness: Harness::Codex,
        session: SessionAttribution::Attributed(session.to_owned()),
        correlation_id: Some(format!("{session}-{minute}")),
        channel,
        stage,
        occurred_at: at(minute),
        detail: serde_json::json!({}),
    }
}

fn decision(
    project_id: ProjectId,
    session: &str,
    outcome: RetrievalOutcome,
    minute: i64,
) -> RetrievalDecision {
    RetrievalDecision {
        decision_id: uuid::Uuid::now_v7(),
        project_id,
        harness: Harness::Codex,
        session: SessionAttribution::Attributed(session.to_owned()),
        correlation_id: Some(format!("{session}-{minute}")),
        channel: LifecycleChannel::UserPromptSubmit,
        outcome,
        reason_code: RetrievalReasonCode::NoRelevantCandidate,
        candidate_count: 0,
        selected_count: 0,
        dropped_count: 0,
        token_count: 0,
        latency_ms: 2,
        query_sha256: "a".repeat(64),
        selected_evidence_ids: Vec::new(),
        occurred_at: at(minute),
    }
}

#[test]
fn folds_delivery_healthy_silence_mcp_not_requested_and_closed_capture_independently() {
    let project_id = ProjectId(uuid::Uuid::now_v7());
    let lifecycle = vec![
        event(
            project_id,
            "session-a",
            LifecycleChannel::SessionStart,
            LifecycleStage::HookReceived,
            1,
        ),
        event(
            project_id,
            "session-a",
            LifecycleChannel::SessionStart,
            LifecycleStage::ReplyFlushed,
            1,
        ),
        event(
            project_id,
            "session-a",
            LifecycleChannel::UserPromptSubmit,
            LifecycleStage::HookReceived,
            2,
        ),
        event(
            project_id,
            "session-a",
            LifecycleChannel::UserPromptSubmit,
            LifecycleStage::ReplyFlushed,
            2,
        ),
        event(
            project_id,
            "session-a",
            LifecycleChannel::SessionEnd,
            LifecycleStage::SessionEndPersisted,
            3,
        ),
        event(
            project_id,
            "session-a",
            LifecycleChannel::Capture,
            LifecycleStage::CaptureCaughtUp,
            4,
        ),
    ];
    let page = fold_session_status(
        project_id,
        &lifecycle,
        &[decision(
            project_id,
            "session-a",
            RetrievalOutcome::HealthySilence,
            2,
        )],
        &[SessionActivity {
            harness: Harness::Codex,
            native_session_id: "session-a".to_owned(),
            first_observed_at: at(0),
            last_observed_at: at(4),
        }],
        SessionStatusOptions {
            filter: SessionFilter::All,
            limit: 50,
            cursor: None,
            now: at(5),
            stale_after: time::Duration::minutes(30),
        },
    )
    .unwrap();
    let session = &page.sessions[0];
    assert_eq!(session.state, SessionLifecycleState::Closed);
    assert_eq!(session.startup.state, ChannelState::Delivered);
    assert_eq!(session.prompt_push.state, ChannelState::HealthySilence);
    assert_eq!(session.mcp_pull.state, ChannelState::NotRequested);
    assert_eq!(session.session_end.state, ChannelState::BoundaryStored);
    assert_eq!(session.capture.state, ChannelState::CaptureCaughtUp);
}

#[test]
fn stale_open_and_historical_uninstrumented_are_not_confused_with_hook_failure() {
    let project_id = ProjectId(uuid::Uuid::now_v7());
    let activities = vec![
        SessionActivity {
            harness: Harness::Codex,
            native_session_id: "old-instrumented".to_owned(),
            first_observed_at: at(0),
            last_observed_at: at(1),
        },
        SessionActivity {
            harness: Harness::Codex,
            native_session_id: "historical".to_owned(),
            first_observed_at: at(-100),
            last_observed_at: at(-90),
        },
    ];
    let lifecycle = vec![event(
        project_id,
        "old-instrumented",
        LifecycleChannel::SessionStart,
        LifecycleStage::HookReceived,
        0,
    )];
    let page = fold_session_status(
        project_id,
        &lifecycle,
        &[],
        &activities,
        SessionStatusOptions {
            filter: SessionFilter::All,
            limit: 50,
            cursor: None,
            now: at(100),
            stale_after: time::Duration::minutes(30),
        },
    )
    .unwrap();
    let stale = page
        .sessions
        .iter()
        .find(|session| session.native_session_id == "old-instrumented")
        .unwrap();
    assert_eq!(stale.state, SessionLifecycleState::StaleOpen);
    let historical = page
        .sessions
        .iter()
        .find(|session| session.native_session_id == "historical")
        .unwrap();
    assert_eq!(
        historical.state,
        SessionLifecycleState::HistoricalUninstrumented
    );
    assert_eq!(
        historical.startup.state,
        ChannelState::HistoricalUninstrumented
    );
}

#[test]
fn pagination_is_bounded_and_cross_project_rows_are_refused() {
    let project_id = ProjectId(uuid::Uuid::now_v7());
    let foreign = ProjectId(uuid::Uuid::now_v7());
    let bad = event(
        foreign,
        "foreign",
        LifecycleChannel::SessionStart,
        LifecycleStage::HookReceived,
        1,
    );
    assert!(
        fold_session_status(
            project_id,
            &[bad],
            &[],
            &[],
            SessionStatusOptions {
                filter: SessionFilter::All,
                limit: 1,
                cursor: None,
                now: at(10),
                stale_after: time::Duration::minutes(30),
            },
        )
        .is_err()
    );
}

#[test]
fn codex_rollout_alias_and_hook_uuid_fold_into_one_session() {
    let project_id = ProjectId(uuid::Uuid::now_v7());
    let session_id = "019ff705-aad7-7373-94b3-932a9b15a323";
    let page = fold_session_status(
        project_id,
        &[
            event(
                project_id,
                session_id,
                LifecycleChannel::SessionStart,
                LifecycleStage::HookReceived,
                1,
            ),
            event(
                project_id,
                session_id,
                LifecycleChannel::SessionStart,
                LifecycleStage::ReplyFlushed,
                2,
            ),
        ],
        &[],
        &[SessionActivity {
            harness: Harness::Codex,
            native_session_id: format!("rollout-2026-08-13T01-29-31-{session_id}"),
            first_observed_at: at(0),
            last_observed_at: at(3),
        }],
        SessionStatusOptions {
            filter: SessionFilter::All,
            limit: 50,
            cursor: None,
            now: at(4),
            stale_after: time::Duration::minutes(30),
        },
    )
    .unwrap();

    assert_eq!(page.sessions.len(), 1);
    assert_eq!(page.sessions[0].native_session_id, session_id);
    assert_eq!(page.sessions[0].startup.state, ChannelState::Delivered);
    assert_eq!(page.sessions[0].first_observed_at, at(0));
    assert_eq!(page.sessions[0].last_observed_at, at(3));
}

#[test]
fn a_session_that_predates_telemetry_does_not_raise_missing_hook_channels() {
    let project_id = ProjectId(uuid::Uuid::now_v7());
    let session_id = "019ff705-aad7-7373-94b3-932a9b15a323";
    let page = fold_session_status(
        project_id,
        &[event(
            project_id,
            session_id,
            LifecycleChannel::Capture,
            LifecycleStage::CaptureCaughtUp,
            3,
        )],
        &[],
        &[SessionActivity {
            harness: Harness::Codex,
            native_session_id: session_id.to_owned(),
            first_observed_at: at(0),
            last_observed_at: at(3),
        }],
        SessionStatusOptions {
            filter: SessionFilter::All,
            limit: 50,
            cursor: None,
            now: at(4),
            stale_after: time::Duration::minutes(30),
        },
    )
    .unwrap();

    assert_eq!(page.sessions.len(), 1);
    assert_eq!(page.sessions[0].state, SessionLifecycleState::Active);
    assert_eq!(
        page.sessions[0].startup.state,
        ChannelState::HistoricalUninstrumented
    );
    assert_eq!(
        page.sessions[0].prompt_push.state,
        ChannelState::HistoricalUninstrumented
    );
    assert_eq!(
        page.sessions[0].capture.state,
        ChannelState::CaptureCaughtUp
    );
}

#[test]
fn session_history_is_not_lost_behind_the_recent_event_cap() {
    let project_id = ProjectId(uuid::Uuid::now_v7());
    let worktree_id = WorktreeId(uuid::Uuid::now_v7());
    let session_id = "019ff705-aad7-7373-94b3-932a9b15a323";
    let mut events = Vec::new();
    events.push(normalized_event(
        project_id,
        worktree_id,
        Harness::Codex,
        session_id,
        0,
    ));
    for sequence in 1_u64..=501 {
        events.push(normalized_event(
            project_id,
            worktree_id,
            Harness::ClaudeCode,
            "newer-session",
            i64::try_from(sequence + 100).unwrap(),
        ));
    }
    let mut ledger = EventLedger::open_in_memory(project_id).unwrap();
    ledger
        .append_batch(&EventBatch {
            source_id: "session-status-cap-fixture".to_owned(),
            events,
            quarantined: Vec::new(),
            capture_gaps: Vec::new(),
            next_cursor: SourceCursor::byte_offset(1),
        })
        .unwrap();
    ledger
        .record_lifecycle_event(&event(
            project_id,
            session_id,
            LifecycleChannel::Capture,
            LifecycleStage::CaptureCaughtUp,
            700,
        ))
        .unwrap();

    let page = read_session_status(
        &ledger,
        project_id,
        SessionStatusOptions {
            filter: SessionFilter::All,
            limit: 50,
            cursor: None,
            now: at(800),
            stale_after: time::Duration::minutes(30),
        },
    )
    .unwrap();
    let session = page
        .sessions
        .iter()
        .find(|session| session.native_session_id == session_id)
        .unwrap();

    assert_eq!(
        session.startup.state,
        ChannelState::HistoricalUninstrumented
    );
    assert_eq!(
        session.prompt_push.state,
        ChannelState::HistoricalUninstrumented
    );
}

fn normalized_event(
    project_id: ProjectId,
    worktree_id: WorktreeId,
    harness: Harness,
    session_id: &str,
    minute: i64,
) -> NormalizedEvent {
    let mut idempotency_key = [0_u8; 32];
    idempotency_key[..8].copy_from_slice(&minute.to_le_bytes());
    idempotency_key[8] = match harness {
        Harness::Codex => 1,
        _ => 2,
    };
    NormalizedEvent {
        event_id: uuid::Uuid::now_v7(),
        project_id,
        worktree_id,
        task_id: None,
        harness,
        native_session_id: session_id.to_owned(),
        native_turn_id: None,
        event_type: EventType::AgentResponded,
        occurred_at: at(minute),
        observed_at: at(minute),
        source_locator: "fixture.jsonl".to_owned(),
        source_offset: minute,
        source_schema: "fixture:v1".to_owned(),
        raw_hash: [3; 32],
        idempotency_key,
        git_head: None,
        git_branch: None,
        payload: serde_json::json!({}),
        raw: serde_json::json!({}),
    }
}
