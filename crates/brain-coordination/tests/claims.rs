use brain_coordination::{
    ClaimKind, CoordinationStore, Overlap, PathClaimInput, TaskRecord, TaskStatus,
};
use brain_domain::{ProjectId, WorktreeId};

#[test]
fn directory_and_child_file_claims_overlap_case_insensitively_on_windows() {
    let (mut store, first, second, now) = fixture();
    store
        .claim_paths(
            first.id,
            vec![input(ClaimKind::Directory, r"src\Auth")],
            now,
        )
        .expect("first claim");
    let result = store
        .claim_paths(
            second.id,
            vec![input(ClaimKind::File, r"SRC\auth\callback.rs")],
            now,
        )
        .expect("second claim");
    assert_eq!(result[0].warnings[0].overlap, Overlap::Definite);
}

#[test]
fn generated_dependency_and_cross_project_paths_do_not_warn() {
    let (mut store, first, second, now) = fixture();
    store
        .claim_paths(first.id, vec![input(ClaimKind::Directory, "target")], now)
        .expect("ignored claim");
    let result = store
        .claim_paths(
            second.id,
            vec![input(ClaimKind::File, "target/debug/app.exe")],
            now,
        )
        .expect("second claim");
    assert!(result[0].warnings.is_empty());
}

#[test]
fn escape_drive_and_alternate_stream_claims_are_rejected() {
    let (mut store, first, _, now) = fixture();
    for value in ["../secret", r"C:\repo\file", "src/file.rs:stream"] {
        assert!(
            store
                .claim_paths(first.id, vec![input(ClaimKind::File, value)], now)
                .is_err(),
            "{value}"
        );
    }
}

fn fixture() -> (
    CoordinationStore,
    TaskRecord,
    TaskRecord,
    time::OffsetDateTime,
) {
    let project = ProjectId(uuid::Uuid::now_v7());
    let now = time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(20_000);
    let first = task(project, "first", now);
    let second = task(project, "second", now);
    let mut store = CoordinationStore::open_in_memory(project).expect("store");
    store.create_task(&first).expect("first task");
    store.create_task(&second).expect("second task");
    (store, first, second, now)
}

fn task(project: ProjectId, title: &str, now: time::OffsetDateTime) -> TaskRecord {
    TaskRecord {
        id: uuid::Uuid::now_v7(),
        project_id: project,
        worktree_id: WorktreeId(uuid::Uuid::now_v7()),
        title: title.to_owned(),
        worktree_path: None,
        branch: None,
        status: TaskStatus::Active,
        created_at: now,
        closed_at: None,
    }
}

fn input(kind: ClaimKind, value: &str) -> PathClaimInput {
    PathClaimInput {
        kind,
        value: value.to_owned(),
        symbol: None,
    }
}
