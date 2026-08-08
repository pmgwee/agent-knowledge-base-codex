//! The session boundary, which nothing else could observe.
//!
//! Transcripts are append-only JSONL: a session ending writes no line, the file simply stops
//! growing. So `EventType::SessionEnded` had never been emitted once — zero across 139,192 captured
//! events in three live projects — and `ConsolidationReason::SessionStopped` was unreachable code
//! that read as fully wired. Sessions consolidated only on crossing the 200-event threshold, which
//! means a short session's work waited for the *next* session to push it over.

use brain_domain::{
    EventBatch, EventType, HOOK_PROTOCOL_VERSION, Harness, HookEnvelope, NormalizedEvent,
    ProjectId, SourceCursor, WorktreeId,
};
use brain_service::{ClaudeHookHandler, HookProjectBinding};
use brain_store::EventLedger;

#[test]
fn a_session_end_hook_records_the_boundary_and_queues_consolidation() {
    let (handler, ledger_path, project_id, root, _temp) = fixture();

    let before = {
        let ledger = EventLedger::open(&ledger_path, project_id).expect("open");
        ledger.consolidation_queue().expect("queue").pending
    };

    handler
        .handle(&envelope("SessionEnd", &root, "session-alpha", "clear"))
        .expect("handle session end");

    let ledger = EventLedger::open(&ledger_path, project_id).expect("reopen");
    assert_eq!(
        session_ended_count(&ledger, project_id),
        1,
        "the boundary must be on the ledger — it is the only place it can be recorded"
    );
    assert!(
        ledger.consolidation_queue().expect("queue").pending > before,
        "a session ending must queue the work it produced, rather than waiting for the next \
         session to cross a threshold"
    );
}

#[test]
fn firing_twice_records_once() {
    // `SessionEnd` has several triggers — `clear`, `logout`, `prompt_input_exit`, `other` — and
    // nothing promises exactly one per session. Two boundaries would consolidate the same span
    // twice and bill the provider for it.
    let (handler, ledger_path, project_id, root, _temp) = fixture();

    for _ in 0..3 {
        handler
            .handle(&envelope("SessionEnd", &root, "session-alpha", "clear"))
            .expect("handle");
    }

    let ledger = EventLedger::open(&ledger_path, project_id).expect("reopen");
    assert_eq!(session_ended_count(&ledger, project_id), 1);
}

#[test]
fn two_different_sessions_each_get_their_own_boundary() {
    let (handler, ledger_path, project_id, root, _temp) = fixture();
    handler
        .handle(&envelope("SessionEnd", &root, "session-alpha", "clear"))
        .expect("handle");
    handler
        .handle(&envelope("SessionEnd", &root, "session-beta", "logout"))
        .expect("handle");

    let ledger = EventLedger::open(&ledger_path, project_id).expect("reopen");
    assert_eq!(session_ended_count(&ledger, project_id), 2);
}

#[test]
fn a_session_end_without_a_session_id_records_nothing() {
    // Inventing an id would create an episode that never existed, and it would be indistinguishable
    // from a real one afterwards.
    let (handler, ledger_path, project_id, root, _temp) = fixture();
    let mut envelope = envelope("SessionEnd", &root, "ignored", "other");
    envelope
        .payload
        .as_object_mut()
        .expect("payload")
        .remove("session_id");

    handler.handle(&envelope).expect("handle");

    let ledger = EventLedger::open(&ledger_path, project_id).expect("reopen");
    assert_eq!(session_ended_count(&ledger, project_id), 0);
}

#[test]
fn a_session_end_outside_any_registered_project_is_ignored() {
    // Cross-project isolation is a locked release criterion, and a hook fires with whatever cwd the
    // harness happened to be in.
    let (handler, ledger_path, project_id, _root, temp) = fixture();
    let elsewhere = temp.path().join("unregistered");
    std::fs::create_dir_all(&elsewhere).expect("create");

    handler
        .handle(&envelope(
            "SessionEnd",
            &elsewhere,
            "session-alpha",
            "clear",
        ))
        .expect("handle");

    let ledger = EventLedger::open(&ledger_path, project_id).expect("reopen");
    assert_eq!(session_ended_count(&ledger, project_id), 0);
}

// --- fixtures ---

type Fixture = (
    ClaudeHookHandler,
    std::path::PathBuf,
    ProjectId,
    std::path::PathBuf,
    tempfile::TempDir,
);

fn fixture() -> Fixture {
    let temp = tempfile::tempdir().expect("fixture");
    let root = temp.path().join("project");
    std::fs::create_dir_all(&root).expect("create root");
    let project_id = ProjectId(uuid::Uuid::now_v7());
    let worktree_id = WorktreeId(uuid::Uuid::now_v7());
    let ledger_path = temp.path().join("events.db");

    let mut ledger = EventLedger::open(&ledger_path, project_id).expect("open");
    ledger
        .append_batch(&EventBatch {
            source_id: "session-end-fixture".to_owned(),
            events: (0..3)
                .map(|index| event(project_id, worktree_id, index))
                .collect(),
            quarantined: Vec::new(),
            capture_gaps: Vec::new(),
            next_cursor: SourceCursor::byte_offset(3),
        })
        .expect("append");
    drop(ledger);

    let handler = ClaudeHookHandler::new(HookProjectBinding {
        project_root: root.clone(),
        project_id,
        worktree_id,
        ledger_path: ledger_path.clone(),
        global_preferences_path: None,
    })
    .expect("handler");

    let canonical_root = std::fs::canonicalize(&root).expect("canonicalize");
    (handler, ledger_path, project_id, canonical_root, temp)
}

fn session_ended_count(ledger: &EventLedger, project_id: ProjectId) -> usize {
    ledger
        .recent_events(project_id, 500)
        .expect("recent events")
        .into_iter()
        .filter(|stored| stored.event_type == EventType::SessionEnded)
        .count()
}

fn envelope(
    event_name: &str,
    cwd: &std::path::Path,
    session_id: &str,
    reason: &str,
) -> HookEnvelope {
    HookEnvelope {
        protocol: HOOK_PROTOCOL_VERSION,
        harness: Harness::ClaudeCode,
        event_name: event_name.to_owned(),
        received_at: time::OffsetDateTime::now_utc(),
        nonce: uuid::Uuid::now_v7(),
        payload: serde_json::json!({
            "hook_event_name": event_name,
            "session_id": session_id,
            "reason": reason,
            "cwd": cwd,
        }),
    }
}

fn event(project_id: ProjectId, worktree_id: WorktreeId, sequence: i64) -> NormalizedEvent {
    let mut key = [0_u8; 32];
    key[0] = sequence as u8;
    NormalizedEvent {
        event_id: uuid::Uuid::now_v7(),
        project_id,
        worktree_id,
        task_id: None,
        harness: Harness::ClaudeCode,
        native_session_id: "session-alpha".to_owned(),
        native_turn_id: None,
        event_type: EventType::UserPrompted,
        occurred_at: time::OffsetDateTime::now_utc(),
        observed_at: time::OffsetDateTime::now_utc(),
        source_locator: "transcript.jsonl".to_owned(),
        source_offset: sequence,
        source_schema: "session-end:v1".to_owned(),
        raw_hash: key,
        idempotency_key: key,
        git_head: None,
        git_branch: None,
        payload: serde_json::json!({ "content": "work happened here" }),
        raw: serde_json::json!({ "content": "work happened here" }),
    }
}
