//! Re-orienting a session when the subject moves.
//!
//! The brain used to push **once**, at session start. A session that ran for hours and pivoted to a
//! different subject was never re-oriented — what arrived at minute zero was all it ever got, and
//! nothing about that was visible because both halves worked: capture was complete and the opening
//! orientation was good.
//!
//! A push that fires on every prompt has the opposite failure mode. These tests are mostly about the
//! restraint rather than the capability: silence is the common correct answer, and the budget and
//! the no-repeat rule are what keep it from becoming noise.

use brain_domain::{
    Authority, EventBatch, EventType, HOOK_PROTOCOL_VERSION, Harness, HookEnvelope, MemoryKind,
    MemoryRecord, MemoryScope, MemoryStatus, NormalizedEvent, ProjectId, SourceCursor, WorktreeId,
};
use brain_service::{ClaudeHookHandler, HookProjectBinding};
use brain_store::{EventLedger, SearchQuery};

const PROMPT: &str = "how does the database migration rollback actually work here";

#[test]
fn a_prompt_that_matches_a_memory_is_answered_with_a_metered_push() {
    let (handler, ledger_path, project, root, _temp) = fixture(&[
        "Database migration rollback procedure",
        "Migration rollback uses a shadow table",
    ]);

    let reply = handler
        .handle(&envelope(&root, "session-a", PROMPT))
        .expect("handle")
        .reply;
    let text = reply
        .additional_context
        .expect("a matching prompt should get a push");

    assert!(text.contains("memory:"), "every line must cite: {text}");
    assert!(
        text.contains("[brain ·") && text.contains("tokens]"),
        "the meter is the last line, always: {text}"
    );

    // And the push is on the record, so the next turn knows not to repeat it.
    let ledger = EventLedger::open(&ledger_path, project).expect("reopen");
    assert!(ledger.session_push_count("session-a").expect("count") > 0);
}

#[test]
fn the_same_memory_is_never_pushed_into_one_session_twice() {
    // The failure this prevents: re-injecting what the model already has, most aggressively exactly
    // when the topic is *not* moving — which is when the budget is worth least.
    let (handler, ledger_path, project, root, _temp) =
        fixture(&["Database migration rollback procedure"]);

    let first = handler
        .handle(&envelope(&root, "session-a", PROMPT))
        .expect("handle")
        .reply
        .additional_context;
    assert!(first.is_some(), "the first ask should be answered");

    let second = handler
        .handle(&envelope(&root, "session-a", PROMPT))
        .expect("handle")
        .reply
        .additional_context;
    assert!(
        second.is_none(),
        "the same memory must not be pushed twice: {second:?}"
    );

    let ledger = EventLedger::open(&ledger_path, project).expect("reopen");
    assert_eq!(ledger.session_push_count("session-a").expect("count"), 1);
}

#[test]
fn a_new_session_starts_clean() {
    // A different session has none of it in context, so the no-repeat rule must not leak across.
    let (handler, _ledger_path, _project, root, _temp) =
        fixture(&["Database migration rollback procedure"]);

    assert!(
        handler
            .handle(&envelope(&root, "session-a", PROMPT))
            .expect("handle")
            .reply
            .additional_context
            .is_some()
    );
    assert!(
        handler
            .handle(&envelope(&root, "session-b", PROMPT))
            .expect("handle")
            .reply
            .additional_context
            .is_some(),
        "a second session must be oriented too"
    );
}

#[test]
fn a_short_prompt_is_answered_with_silence() {
    // "ok", "continue", "yes" share no vocabulary with anything specific, so retrieval returns
    // whatever is generally popular. The honest push for those is none.
    let (handler, _l, _p, root, _temp) = fixture(&["Database migration rollback procedure"]);
    for prompt in ["ok", "continue", "yes please", "go on"] {
        assert!(
            handler
                .handle(&envelope(&root, "session-a", prompt))
                .expect("handle")
                .reply
                .additional_context
                .is_none(),
            "a {} character prompt should push nothing",
            prompt.len()
        );
    }
}

#[test]
fn a_prompt_matching_nothing_is_answered_with_silence() {
    let (handler, _l, _p, root, _temp) = fixture(&["Database migration rollback procedure"]);
    assert!(
        handler
            .handle(&envelope(
                &root,
                "session-a",
                "what is the airspeed velocity of an unladen swallow"
            ))
            .expect("handle")
            .reply
            .additional_context
            .is_none()
    );
}

#[test]
fn a_prompt_without_a_session_id_pushes_nothing() {
    // With no session id there is no way to avoid repeating ourselves, and a push that repeats is
    // worse than no push at all.
    let (handler, _l, _p, root, _temp) = fixture(&["Database migration rollback procedure"]);
    let mut envelope = envelope(&root, "ignored", PROMPT);
    envelope
        .payload
        .as_object_mut()
        .expect("payload")
        .remove("session_id");
    assert!(
        handler
            .handle(&envelope)
            .expect("handle")
            .reply
            .additional_context
            .is_none()
    );
}

#[test]
fn a_prompt_from_outside_any_registered_project_pushes_nothing() {
    // Cross-project isolation is a locked release criterion, and this hook fires on every message
    // with whatever cwd the harness happened to be in.
    let (handler, _l, _p, _root, temp) = fixture(&["Database migration rollback procedure"]);
    let elsewhere = temp.path().join("unregistered");
    std::fs::create_dir_all(&elsewhere).expect("create");
    assert!(
        handler
            .handle(&envelope(&elsewhere, "session-a", PROMPT))
            .expect("handle")
            .reply
            .additional_context
            .is_none()
    );
}

#[test]
fn the_push_is_bounded_and_says_so_when_it_drops_something() {
    // Twelve matching memories against a four-memory ceiling. The meter has to name the loss —
    // a silent truncation reads as "that is all there was".
    let titles: Vec<String> = (0..12)
        .map(|i| format!("Database migration rollback step {i}"))
        .collect();
    let refs: Vec<&str> = titles.iter().map(String::as_str).collect();
    let (handler, _l, _p, root, _temp) = fixture(&refs);

    let text = handler
        .handle(&envelope(&root, "session-a", PROMPT))
        .expect("handle")
        .reply
        .additional_context
        .expect("push");

    let cited = text.matches("memory:").count();
    assert!(
        cited <= 4,
        "at most four memories per push, got {cited}: {text}"
    );
    assert!(
        text.contains("dropped"),
        "the meter must name what it dropped: {text}"
    );
}

// --- fixtures ---

type Fixture = (
    ClaudeHookHandler,
    std::path::PathBuf,
    ProjectId,
    std::path::PathBuf,
    tempfile::TempDir,
);

fn fixture(memory_titles: &[&str]) -> Fixture {
    let temp = tempfile::tempdir().expect("fixture");
    let root = temp.path().join("project");
    std::fs::create_dir_all(&root).expect("create root");
    let brain_home = temp.path().join("brain");
    std::fs::create_dir_all(&brain_home).expect("create brain home");
    let project = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let ledger_path = temp.path().join("events.db");

    let mut ledger = EventLedger::open(&ledger_path, project).expect("open");
    ledger
        .append_batch(&EventBatch {
            source_id: "push-fixture".to_owned(),
            events: vec![event(project, worktree)],
            quarantined: Vec::new(),
            capture_gaps: Vec::new(),
            next_cursor: SourceCursor::byte_offset(1),
        })
        .expect("append");
    let cited = ledger
        .search(
            &SearchQuery::text(project, "migration")
                .events_only()
                .with_limit(1),
        )
        .expect("search")
        .first()
        .map(|hit| hit.source_id)
        .expect("one event");
    for title in memory_titles {
        let record = MemoryRecord {
            id: uuid::Uuid::now_v7(),
            version_id: uuid::Uuid::now_v7(),
            scope: MemoryScope::Project(project),
            worktree_id: Some(worktree),
            task_id: None,
            kind: MemoryKind::Procedure,
            title: (*title).to_owned(),
            content: format!("{title}. The rollback runs against the shadow table first."),
            valid_from: time::OffsetDateTime::UNIX_EPOCH,
            valid_to: None,
            recorded_at: time::OffsetDateTime::UNIX_EPOCH,
            confidence: 1.0,
            authority: Authority::DerivedMemory,
            evidence_ids: vec![cited],
            supersedes: Vec::new(),
            status: MemoryStatus::Current,
        };
        ledger.append_memory(&record).expect("append memory");
    }
    drop(ledger);

    let handler = ClaudeHookHandler::new(HookProjectBinding {
        project_root: root.clone(),
        project_id: project,
        worktree_id: worktree,
        ledger_path: ledger_path.clone(),
        global_preferences_path: None,
        brain_home,
    })
    .expect("handler");

    let canonical = std::fs::canonicalize(&root).expect("canonicalize");
    (handler, ledger_path, project, canonical, temp)
}

fn envelope(cwd: &std::path::Path, session_id: &str, prompt: &str) -> HookEnvelope {
    HookEnvelope {
        protocol: HOOK_PROTOCOL_VERSION,
        harness: Harness::ClaudeCode,
        event_name: "UserPromptSubmit".to_owned(),
        received_at: time::OffsetDateTime::now_utc(),
        nonce: uuid::Uuid::now_v7(),
        payload: serde_json::json!({
            "hook_event_name": "UserPromptSubmit",
            "session_id": session_id,
            "prompt": prompt,
            "cwd": cwd,
        }),
    }
}

fn event(project: ProjectId, worktree: WorktreeId) -> NormalizedEvent {
    let key = [11_u8; 32];
    NormalizedEvent {
        event_id: uuid::Uuid::now_v7(),
        project_id: project,
        worktree_id: worktree,
        task_id: None,
        harness: Harness::ClaudeCode,
        native_session_id: "seed".to_owned(),
        native_turn_id: None,
        event_type: EventType::UserPrompted,
        occurred_at: time::OffsetDateTime::UNIX_EPOCH,
        observed_at: time::OffsetDateTime::UNIX_EPOCH,
        source_locator: "transcript.jsonl".to_owned(),
        source_offset: 0,
        source_schema: "push:v1".to_owned(),
        raw_hash: key,
        idempotency_key: key,
        git_head: None,
        git_branch: None,
        payload: serde_json::json!({ "content": "the database migration rollback ran" }),
        raw: serde_json::json!({ "content": "the database migration rollback ran" }),
    }
}
