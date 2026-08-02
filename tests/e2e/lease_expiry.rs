mod coordination_fixture;

use brain_coordination::LeaseError;
use brain_domain::Harness;
use coordination_fixture::{CoordinationE2e, session};

#[test]
fn crashed_session_recovers_by_expiry_takeover_and_audited_handoff() {
    let fixture = CoordinationE2e::new();
    let task = fixture.start_task("recover crashed writer");
    let claude = session(Harness::ClaudeCode, "claude-crashed");
    let codex = session(Harness::Codex, "codex-recovery");
    let hermes = session(Harness::Hermes, "hermes-handoff");
    let mut store = fixture.store();

    let first = store
        .acquire_lease(task.id, claude.clone(), fixture.now, None)
        .expect("initial lease");
    let early = store.acquire_lease(
        task.id,
        codex.clone(),
        first.expires_at - time::Duration::nanoseconds(1),
        None,
    );
    assert!(matches!(early, Err(LeaseError::AlreadyHeld { .. })));

    let takeover = store
        .acquire_lease(task.id, codex.clone(), first.expires_at, None)
        .expect("expiry takeover");
    assert_eq!(takeover.generation, first.generation + 1);
    let handed = store
        .handoff_lease(
            task.id,
            &codex,
            takeover.generation,
            hermes.clone(),
            takeover.acquired_at + time::Duration::minutes(1),
        )
        .expect("explicit handoff");
    assert_eq!(handed.owner, hermes);
    assert_eq!(handed.generation, takeover.generation + 1);

    let events = store.coordination_events().expect("coordination audit");
    let takeover_event = events
        .iter()
        .find(|event| event.action == "takeover")
        .expect("takeover event");
    assert_eq!(takeover_event.prior_owner, Some(claude));
    assert!(events.iter().any(|event| event.action == "handoff"));
}
