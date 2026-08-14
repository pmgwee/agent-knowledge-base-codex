use brain_cli::{
    ChannelState, SessionActivity, SessionFilter, SessionLifecycleState, SessionStatusOptions,
    fold_session_status,
};
use brain_domain::{Harness, ProjectId};
use brain_store::{
    LifecycleChannel, LifecycleEvent, LifecycleStage, RetrievalDecision, RetrievalOutcome,
    RetrievalReasonCode, SessionAttribution,
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
