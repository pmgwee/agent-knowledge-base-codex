use brain_coordination::{CoordinationStore, LeaseError, SessionIdentity, TaskRecord, TaskStatus};
use brain_domain::{Harness, ProjectId, WorktreeId};

#[test]
fn only_one_session_can_hold_the_writer_lease_for_a_worktree() {
    let (mut store, task, now) = fixture();
    store
        .acquire_lease(task.id, session("claude"), now, None)
        .expect("first lease");
    let second = store.acquire_lease(task.id, session("codex"), now, None);
    assert!(matches!(second, Err(LeaseError::AlreadyHeld { .. })));
}

#[test]
fn takeover_after_expiry_is_audited_and_increments_generation() {
    let (mut store, task, now) = fixture();
    let first = store
        .acquire_lease(task.id, session("claude"), now, None)
        .expect("first lease");
    let second = store
        .acquire_lease(
            task.id,
            session("codex"),
            now + time::Duration::minutes(30),
            None,
        )
        .expect("takeover at expiry boundary");
    assert_eq!(second.generation, first.generation + 1);
    let events = store.coordination_events().expect("events");
    assert_eq!(
        events
            .iter()
            .filter(|event| event.action == "takeover")
            .count(),
        1
    );
    assert_eq!(
        events.last().expect("takeover event").prior_owner,
        Some(session("claude"))
    );
}

#[test]
fn renew_release_and_handoff_require_owner_and_generation() {
    let (mut store, task, now) = fixture();
    let lease = store
        .acquire_lease(task.id, session("claude"), now, None)
        .expect("lease");
    assert!(matches!(
        store.renew_lease(task.id, &session("codex"), lease.generation, now, None),
        Err(LeaseError::WrongOwner)
    ));
    assert!(matches!(
        store.release_lease(task.id, &session("claude"), 99, now),
        Err(LeaseError::GenerationChanged)
    ));
    let handed = store
        .handoff_lease(
            task.id,
            &session("claude"),
            lease.generation,
            session("codex"),
            now + time::Duration::minutes(1),
        )
        .expect("handoff");
    assert_eq!(handed.owner, session("codex"));
    assert_eq!(handed.generation, lease.generation + 1);
    store
        .release_lease(
            task.id,
            &session("codex"),
            handed.generation,
            now + time::Duration::minutes(2),
        )
        .expect("release");
    assert!(store.lease(task.id).expect("lease lookup").is_none());
}

#[test]
fn a_clock_skewed_renewal_never_moves_expiry_backward() {
    let (mut store, task, now) = fixture();
    let first = store
        .acquire_lease(task.id, session("claude"), now, None)
        .expect("lease");
    let renewed = store
        .renew_lease(
            task.id,
            &session("claude"),
            first.generation,
            now - time::Duration::minutes(5),
            None,
        )
        .expect("skewed renew");
    assert!(renewed.expires_at >= first.expires_at);
}

fn fixture() -> (CoordinationStore, TaskRecord, time::OffsetDateTime) {
    let project = ProjectId(uuid::Uuid::now_v7());
    let now = time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(20_000);
    let task = TaskRecord {
        id: uuid::Uuid::now_v7(),
        project_id: project,
        worktree_id: WorktreeId(uuid::Uuid::now_v7()),
        title: "OAuth callback".to_owned(),
        worktree_path: None,
        branch: Some("agent/oauth".to_owned()),
        status: TaskStatus::Active,
        created_at: now,
        closed_at: None,
    };
    let mut store = CoordinationStore::open_in_memory(project).expect("store");
    store.create_task(&task).expect("task");
    (store, task, now)
}

fn session(name: &str) -> SessionIdentity {
    SessionIdentity {
        harness: match name {
            "claude" => Harness::ClaudeCode,
            "codex" => Harness::Codex,
            other => Harness::Other(other.to_owned()),
        },
        native_session_id: format!("{name}-session"),
    }
}
