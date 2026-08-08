use brain_domain::{
    Authority, EventBatch, EventType, HOOK_PROTOCOL_VERSION, Harness, HookEnvelope, MemoryKind,
    MemoryRecord, MemoryScope, MemoryStatus, NormalizedEvent, ProjectId, SourceCursor, WorktreeId,
};
use brain_service::{ClaudeHookHandler, HookProjectBinding};
use brain_store::{EventLedger, GlobalPreferenceStore};

#[test]
fn compiling_an_orientation_records_nothing_until_it_is_delivered() {
    // The defect this pins: `handle` used to record inside itself, one step before the reply was
    // written to the pipe, so an orientation that never reached its client counted exactly the
    // same as one that did. It was not hypothetical — a single day's service log held ten
    // `write hook reply` failures with nine requests still spooled, every one of them counted.
    //
    // A metric that overstates cannot be used to prove a saving, which is what made this block
    // the whole benchmark rather than merely annoy.
    let temp = tempfile::tempdir().expect("create delivery fixture");
    let project_root = temp.path().join("project");
    std::fs::create_dir_all(&project_root).expect("create project root");
    let project_id = ProjectId(uuid::Uuid::now_v7());
    let worktree_id = WorktreeId(uuid::Uuid::now_v7());
    let ledger_path = temp.path().join("events.db");
    let mut ledger = EventLedger::open(&ledger_path, project_id).expect("open delivery ledger");
    ledger
        .append_batch(&EventBatch {
            source_id: "delivery-fixture".to_owned(),
            events: vec![event(
                project_id,
                worktree_id,
                EventType::UserPrompted,
                1,
                "continue prior auth task",
            )],
            quarantined: Vec::new(),
            capture_gaps: Vec::new(),
            next_cursor: SourceCursor::byte_offset(1),
        })
        .expect("append delivery evidence");
    drop(ledger);

    let handler = ClaudeHookHandler::new(HookProjectBinding {
        project_root: project_root.clone(),
        project_id,
        worktree_id,
        ledger_path: ledger_path.clone(),
        global_preferences_path: None,
        brain_home: std::path::PathBuf::new(),
    })
    .expect("create delivery handler");

    let window_start = time::OffsetDateTime::now_utc() - time::Duration::hours(1);
    let outcome = handler
        .handle(&envelope("SessionStart", &project_root))
        .expect("compile startup reply");
    assert!(
        outcome.reply.additional_context.is_some(),
        "an orientation was compiled"
    );
    assert!(
        outcome.delivery.is_some(),
        "and a delivery is staged for the caller to record"
    );

    // This is the delivery failing: the outcome is dropped without ever being recorded, exactly
    // as it would be if `write_all` or `flush` returned an error.
    let staged = outcome.delivery.expect("staged delivery");
    drop(outcome.reply);

    let ledger = EventLedger::open(&ledger_path, project_id).expect("reopen delivery ledger");
    assert_eq!(
        ledger
            .context_delivery_summary(window_start)
            .expect("read delivery summary")
            .deliveries,
        0,
        "an orientation that was never handed over is not a delivery"
    );
    drop(ledger);

    // And now it succeeds, which is the pipe's post-flush call.
    staged.record().expect("record a delivered orientation");
    let ledger = EventLedger::open(&ledger_path, project_id).expect("reopen delivery ledger");
    assert_eq!(
        ledger
            .context_delivery_summary(window_start)
            .expect("read delivery summary")
            .deliveries,
        1,
        "one recorded delivery, once the client has it"
    );
}

#[test]
fn session_start_records_the_size_of_what_it_delivered() {
    // A hook reply is not an event and appears in no transcript, so unless it is recorded
    // as it happens the cost of the brain's own orientation is unrecoverable.
    let temp = tempfile::tempdir().expect("create delivery fixture");
    let project_root = temp.path().join("project");
    std::fs::create_dir_all(&project_root).expect("create project root");
    let project_id = ProjectId(uuid::Uuid::now_v7());
    let worktree_id = WorktreeId(uuid::Uuid::now_v7());
    let ledger_path = temp.path().join("events.db");
    let mut ledger = EventLedger::open(&ledger_path, project_id).expect("open delivery ledger");
    ledger
        .append_batch(&EventBatch {
            source_id: "delivery-fixture".to_owned(),
            events: vec![event(
                project_id,
                worktree_id,
                EventType::UserPrompted,
                1,
                "continue prior auth task",
            )],
            quarantined: Vec::new(),
            capture_gaps: Vec::new(),
            next_cursor: SourceCursor::byte_offset(1),
        })
        .expect("append delivery evidence");
    drop(ledger);

    let handler = ClaudeHookHandler::new(HookProjectBinding {
        project_root: project_root.clone(),
        project_id,
        worktree_id,
        ledger_path: ledger_path.clone(),
        global_preferences_path: None,
        brain_home: std::path::PathBuf::new(),
    })
    .expect("create delivery handler");

    let window_start = time::OffsetDateTime::now_utc() - time::Duration::hours(1);
    let outcome = handler
        .handle(&envelope("SessionStart", &project_root))
        .expect("compile startup reply");
    let delivered = outcome.reply.additional_context.expect("startup context");
    // Standing in for the pipe, which records only once the reply has been flushed to the client.
    outcome
        .delivery
        .expect("staged delivery")
        .record()
        .expect("record delivery");

    let ledger = EventLedger::open(&ledger_path, project_id).expect("reopen delivery ledger");
    let summary = ledger
        .context_delivery_summary(window_start)
        .expect("read delivery summary");

    assert_eq!(summary.deliveries, 1, "the hook must record what it sent");
    assert_eq!(
        summary.max_tokens,
        brain_context::token_count(&delivered) as u64,
        "recorded size must equal the orientation actually delivered"
    );
    assert!(summary.mean_tokens() > 0.0);
}

#[test]
fn session_start_returns_bounded_project_scoped_context() {
    let temp = tempfile::tempdir().expect("create hook handler fixture");
    let project_root = temp.path().join("project");
    let other_root = temp.path().join("other");
    std::fs::create_dir_all(&project_root).expect("create project root");
    std::fs::create_dir_all(&other_root).expect("create foreign root");
    let project_id = ProjectId(uuid::Uuid::now_v7());
    let worktree_id = WorktreeId(uuid::Uuid::now_v7());
    let ledger_path = temp.path().join("events.db");
    let mut ledger = EventLedger::open(&ledger_path, project_id).expect("open hook ledger");
    ledger
        .append_batch(&EventBatch {
            source_id: "hook-handler-fixture".to_owned(),
            events: vec![
                event(
                    project_id,
                    worktree_id,
                    EventType::UserPrompted,
                    1,
                    "continue prior auth task",
                ),
                event(
                    project_id,
                    worktree_id,
                    EventType::TestCompleted,
                    2,
                    "auth callback tests failed",
                ),
            ],
            quarantined: Vec::new(),
            capture_gaps: Vec::new(),
            next_cursor: SourceCursor::byte_offset(2),
        })
        .expect("append hook evidence");
    drop(ledger);
    let handler = ClaudeHookHandler::new(HookProjectBinding {
        project_root: project_root.clone(),
        project_id,
        worktree_id,
        ledger_path,
        global_preferences_path: None,
        brain_home: std::path::PathBuf::new(),
    })
    .expect("create hook handler");

    let reply = handler
        .handle(&envelope("SessionStart", &project_root))
        .expect("compile startup reply")
        .reply;
    let context = reply.additional_context.expect("startup context");
    assert!(context.contains("continue prior auth task"));
    assert!(context.contains("auth callback tests failed"));
    assert!(context.contains("Evidence:"));
    assert!(brain_context::token_count(&context) <= 1_500);

    let foreign = handler
        .handle(&envelope("SessionStart", &other_root))
        .expect("handle foreign project")
        .reply;
    assert_eq!(foreign.additional_context, None);
    let capture_only = handler
        .handle(&envelope("PostToolUse", &project_root))
        .expect("handle capture-only event")
        .reply;
    assert_eq!(capture_only.additional_context, None);
}

#[test]
fn session_start_includes_only_explicit_global_preferences() {
    let temp = tempfile::tempdir().expect("create global preference hook fixture");
    let project_root = temp.path().join("project");
    std::fs::create_dir_all(&project_root).expect("create project root");
    let project_id = ProjectId(uuid::Uuid::now_v7());
    let worktree_id = WorktreeId(uuid::Uuid::now_v7());
    let ledger_path = temp.path().join("events.db");
    EventLedger::open(&ledger_path, project_id).expect("open empty project ledger");
    let preferences_path = temp.path().join("preferences.sqlite");
    let mut preferences = GlobalPreferenceStore::open(&preferences_path).expect("open preferences");
    let preference = MemoryRecord {
        id: uuid::Uuid::now_v7(),
        version_id: uuid::Uuid::now_v7(),
        scope: MemoryScope::GlobalPreferences,
        worktree_id: None,
        task_id: None,
        kind: MemoryKind::Preference,
        title: "Response style".to_owned(),
        content: "Prefer concise implementation updates.".to_owned(),
        valid_from: time::OffsetDateTime::UNIX_EPOCH,
        valid_to: None,
        recorded_at: time::OffsetDateTime::UNIX_EPOCH,
        confidence: 1.0,
        authority: Authority::HumanCorrection,
        evidence_ids: Vec::new(),
        supersedes: Vec::new(),
        status: MemoryStatus::Current,
    };
    preferences
        .append_preference(&preference)
        .expect("append explicit preference");
    drop(preferences);
    let handler = ClaudeHookHandler::new(HookProjectBinding {
        project_root: project_root.clone(),
        project_id,
        worktree_id,
        ledger_path,
        global_preferences_path: Some(preferences_path),
        brain_home: std::path::PathBuf::new(),
    })
    .expect("create hook handler");

    let context = handler
        .handle(&envelope("SessionStart", &project_root))
        .expect("compile startup context")
        .reply
        .additional_context
        .expect("preference creates context");
    assert!(context.contains("Prefer concise implementation updates"));
    assert!(context.contains(&format!("memory:{}", preference.version_id)));
}

fn envelope(event_name: &str, cwd: &std::path::Path) -> HookEnvelope {
    HookEnvelope {
        protocol: HOOK_PROTOCOL_VERSION,
        harness: Harness::ClaudeCode,
        event_name: event_name.to_owned(),
        received_at: time::OffsetDateTime::now_utc(),
        nonce: uuid::Uuid::now_v7(),
        payload: serde_json::json!({
            "hook_event_name": event_name,
            "session_id": "new-session",
            "cwd": cwd,
        }),
    }
}

fn event(
    project_id: ProjectId,
    worktree_id: WorktreeId,
    event_type: EventType,
    sequence: i64,
    text: &str,
) -> NormalizedEvent {
    NormalizedEvent {
        event_id: uuid::Uuid::now_v7(),
        project_id,
        worktree_id,
        task_id: None,
        harness: Harness::ClaudeCode,
        native_session_id: "prior-session".to_owned(),
        native_turn_id: Some(format!("turn-{sequence}")),
        event_type,
        occurred_at: time::OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(sequence),
        observed_at: time::OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(sequence),
        source_locator: "fixture.jsonl".to_owned(),
        source_offset: sequence,
        source_schema: "fixture:v1".to_owned(),
        raw_hash: [u8::try_from(sequence).expect("small sequence"); 32],
        idempotency_key: [u8::try_from(sequence + 10).expect("small sequence"); 32],
        git_head: Some("abc123".to_owned()),
        git_branch: Some("feature/auth".to_owned()),
        payload: serde_json::json!({"text": text}),
        raw: serde_json::json!({"text": text}),
    }
}
