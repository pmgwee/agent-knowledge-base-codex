mod coordination_fixture;

use brain_coordination::{ClaimKind, LeaseError, Overlap, PathClaimInput};
use brain_domain::Harness;
use coordination_fixture::{CoordinationE2e, session};

#[test]
fn three_agents_get_isolated_worktrees_and_early_overlap_warning() {
    let fixture = CoordinationE2e::new();
    let claude_task = fixture.start_task("oauth callback");
    let codex_task = fixture.start_task("oauth tests");
    let hermes_task = fixture.start_task("oauth documentation");

    let paths = [
        claude_task.worktree_path.as_ref().expect("Claude path"),
        codex_task.worktree_path.as_ref().expect("Codex path"),
        hermes_task.worktree_path.as_ref().expect("Hermes path"),
    ];
    assert_ne!(paths[0], paths[1]);
    assert_ne!(paths[1], paths[2]);
    assert_ne!(paths[0], paths[2]);
    assert!(
        paths
            .iter()
            .all(|path| path.starts_with(&fixture.worktree_parent))
    );

    let mut store = fixture.store();
    store
        .acquire_lease(
            claude_task.id,
            session(Harness::ClaudeCode, "claude-1"),
            fixture.now,
            None,
        )
        .expect("Claude lease");
    store
        .acquire_lease(
            codex_task.id,
            session(Harness::Codex, "codex-1"),
            fixture.now,
            None,
        )
        .expect("Codex lease");
    store
        .acquire_lease(
            hermes_task.id,
            session(Harness::Hermes, "hermes-1"),
            fixture.now,
            None,
        )
        .expect("Hermes lease");

    let second_writer = store.acquire_lease(
        claude_task.id,
        session(Harness::Codex, "codex-accidental"),
        fixture.now,
        None,
    );
    assert!(matches!(second_writer, Err(LeaseError::AlreadyHeld { .. })));

    store
        .claim_paths(
            claude_task.id,
            vec![claim(ClaimKind::Directory, "src/auth")],
            fixture.now,
        )
        .expect("Claude claim");
    let warnings = store
        .claim_paths(
            codex_task.id,
            vec![claim(ClaimKind::File, "SRC\\auth\\callback.rs")],
            fixture.now,
        )
        .expect("Codex claim");
    assert_eq!(warnings[0].warnings[0].overlap, Overlap::Definite);
    assert_eq!(store.active_leases(fixture.now).expect("leases").len(), 3);
}

fn claim(kind: ClaimKind, value: &str) -> PathClaimInput {
    PathClaimInput {
        kind,
        value: value.to_owned(),
        symbol: None,
    }
}
