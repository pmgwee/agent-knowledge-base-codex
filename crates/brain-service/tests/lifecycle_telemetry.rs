use brain_domain::{HOOK_PROTOCOL_VERSION, Harness, HookEnvelope, ProjectId, WorktreeId};
use brain_service::{ClaudeHookHandler, HookProjectBinding};
use brain_store::{
    EventLedger, LifecycleStage, RetrievalOutcome, RetrievalReasonCode, SessionAttribution,
    TelemetryQuery,
};

fn query(project_id: ProjectId, session_id: &str) -> TelemetryQuery {
    TelemetryQuery {
        project_id,
        session: Some(SessionAttribution::Attributed(session_id.to_owned())),
        start: time::OffsetDateTime::UNIX_EPOCH,
        end: time::OffsetDateTime::now_utc() + time::Duration::minutes(1),
        limit: 100,
    }
}

#[test]
fn hook_received_and_retrieval_decision_precede_reply_flush_receipt() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    std::fs::create_dir_all(&root).unwrap();
    let project_id = ProjectId(uuid::Uuid::now_v7());
    let worktree_id = WorktreeId(uuid::Uuid::now_v7());
    let ledger_path = temp.path().join("ledger.sqlite3");
    EventLedger::open(&ledger_path, project_id).unwrap();
    let handler = ClaudeHookHandler::new(HookProjectBinding {
        project_root: root.clone(),
        project_id,
        worktree_id,
        ledger_path: ledger_path.clone(),
        global_preferences_path: None,
        brain_home: temp.path().to_path_buf(),
    })
    .unwrap();

    let outcome = handler
        .handle(&envelope("SessionStart", &root, "session-a", None))
        .unwrap();
    let ledger = EventLedger::open(&ledger_path, project_id).unwrap();
    let before_flush = ledger
        .lifecycle_events(&query(project_id, "session-a"))
        .unwrap();
    assert!(
        before_flush
            .iter()
            .any(|event| event.stage == LifecycleStage::HookReceived)
    );
    assert!(
        !before_flush
            .iter()
            .any(|event| event.stage == LifecycleStage::ReplyFlushed)
    );
    let decisions = ledger
        .retrieval_decisions(&query(project_id, "session-a"))
        .unwrap();
    assert_eq!(decisions.len(), 1);
    assert_eq!(decisions[0].outcome, RetrievalOutcome::Delivered);
    drop(ledger);

    outcome
        .flush_receipt
        .expect("matched hook receipt")
        .record()
        .unwrap();
    let ledger = EventLedger::open(&ledger_path, project_id).unwrap();
    let after_flush = ledger
        .lifecycle_events(&query(project_id, "session-a"))
        .unwrap();
    assert!(
        after_flush
            .iter()
            .any(|event| event.stage == LifecycleStage::ReplyFlushed)
    );
}

#[test]
fn short_prompt_is_recorded_as_healthy_silence_not_a_missing_hook() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    std::fs::create_dir_all(&root).unwrap();
    let project_id = ProjectId(uuid::Uuid::now_v7());
    let worktree_id = WorktreeId(uuid::Uuid::now_v7());
    let ledger_path = temp.path().join("ledger.sqlite3");
    EventLedger::open(&ledger_path, project_id).unwrap();
    let handler = ClaudeHookHandler::new(HookProjectBinding {
        project_root: root.clone(),
        project_id,
        worktree_id,
        ledger_path: ledger_path.clone(),
        global_preferences_path: None,
        brain_home: temp.path().to_path_buf(),
    })
    .unwrap();

    let outcome = handler
        .handle(&envelope(
            "UserPromptSubmit",
            &root,
            "session-a",
            Some("ok"),
        ))
        .unwrap();
    assert!(outcome.reply.additional_context.is_none());
    let ledger = EventLedger::open(&ledger_path, project_id).unwrap();
    let decisions = ledger
        .retrieval_decisions(&query(project_id, "session-a"))
        .unwrap();
    assert_eq!(decisions.len(), 1);
    assert_eq!(decisions[0].outcome, RetrievalOutcome::HealthySilence);
    assert_eq!(decisions[0].reason_code, RetrievalReasonCode::ShortPrompt);
}

#[test]
fn telemetry_write_failure_is_fail_open_for_the_hook() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    let invalid_ledger_path = temp.path().join("ledger-is-a-directory");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&invalid_ledger_path).unwrap();
    let handler = ClaudeHookHandler::new(HookProjectBinding {
        project_root: root.clone(),
        project_id: ProjectId(uuid::Uuid::now_v7()),
        worktree_id: WorktreeId(uuid::Uuid::now_v7()),
        ledger_path: invalid_ledger_path,
        global_preferences_path: None,
        brain_home: temp.path().to_path_buf(),
    })
    .unwrap();

    let outcome = handler.handle(&envelope("PostToolUse", &root, "session-a", None));
    assert!(outcome.is_ok(), "telemetry must never block hook routing");

    let session_start = handler.handle(&envelope("SessionStart", &root, "session-a", None));
    assert!(
        session_start.is_ok(),
        "orientation retrieval failures must also fail open"
    );
    assert!(session_start.unwrap().reply.additional_context.is_none());
}

fn envelope(
    event_name: &str,
    cwd: &std::path::Path,
    session_id: &str,
    prompt: Option<&str>,
) -> HookEnvelope {
    HookEnvelope {
        protocol: HOOK_PROTOCOL_VERSION,
        harness: Harness::Codex,
        event_name: event_name.to_owned(),
        received_at: time::OffsetDateTime::now_utc(),
        nonce: uuid::Uuid::now_v7(),
        payload: serde_json::json!({
            "hook_event_name": event_name,
            "session_id": session_id,
            "cwd": cwd,
            "prompt": prompt,
        }),
    }
}
