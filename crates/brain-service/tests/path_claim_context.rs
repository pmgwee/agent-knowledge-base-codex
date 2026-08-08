use std::fs;

use brain_context::token_count;
use brain_coordination::{
    ClaimKind, CoordinationStore, PathClaimInput, SessionIdentity, TaskRecord, TaskStatus,
};
use brain_domain::{HOOK_PROTOCOL_VERSION, Harness, HookEnvelope, ProjectId, WorktreeId};
use brain_service::{HookProjectBinding, ProjectHookHandler};

#[test]
fn session_start_shows_active_owners_and_overlap_before_memory() {
    let temp = tempfile::tempdir().expect("temp");
    let main = temp.path().join("main");
    let task_a_path = temp.path().join("worktrees").join("task-a");
    let task_b_path = temp.path().join("worktrees").join("task-b");
    fs::create_dir_all(&main).expect("main");
    fs::create_dir_all(&task_a_path).expect("task A");
    fs::create_dir_all(&task_b_path).expect("task B");
    let project = ProjectId(uuid::Uuid::now_v7());
    let main_worktree = WorktreeId(uuid::Uuid::now_v7());
    let task_a = task(project, task_a_path.clone(), "OAuth", 1);
    let task_b = task(project, task_b_path, "OAuth tests", 2);
    let ledger_path = temp.path().join("ledger.sqlite");
    let now = time::OffsetDateTime::now_utc();
    let mut store = CoordinationStore::open(&ledger_path, project).expect("store");
    store.create_task(&task_a).expect("task A");
    store.create_task(&task_b).expect("task B");
    store
        .acquire_lease(
            task_a.id,
            SessionIdentity {
                harness: Harness::ClaudeCode,
                native_session_id: "claude-owner".to_owned(),
            },
            now,
            None,
        )
        .expect("lease");
    store
        .claim_paths(
            task_a.id,
            vec![PathClaimInput {
                kind: ClaimKind::Directory,
                value: "src/auth".to_owned(),
                symbol: None,
            }],
            now,
        )
        .expect("claim A");
    store
        .claim_paths(
            task_b.id,
            vec![PathClaimInput {
                kind: ClaimKind::File,
                value: "SRC/auth/callback.rs".to_owned(),
                symbol: None,
            }],
            now,
        )
        .expect("claim B");

    let handler = ProjectHookHandler::new(HookProjectBinding {
        project_root: main,
        project_id: project,
        worktree_id: main_worktree,
        ledger_path,
        global_preferences_path: None,
        brain_home: std::path::PathBuf::new(),
    })
    .expect("handler");
    let reply = handler
        .handle(&HookEnvelope {
            protocol: HOOK_PROTOCOL_VERSION,
            harness: Harness::Codex,
            event_name: "SessionStart".to_owned(),
            received_at: now,
            nonce: uuid::Uuid::now_v7(),
            payload: serde_json::json!({
                "cwd": task_a_path,
                "session_id": "new-codex-session"
            }),
        })
        .expect("handle")
        .reply;
    let context = reply.additional_context.expect("coordination context");
    assert!(context.starts_with("Coordination state"), "{context}");
    assert!(context.contains("[current worktree]"), "{context}");
    assert!(context.contains("claude-owner"), "{context}");
    assert!(context.contains("OVERLAP Definite"), "{context}");
    assert!(token_count(&context) <= 1_500);
}

fn task(project: ProjectId, path: std::path::PathBuf, title: &str, offset: i64) -> TaskRecord {
    TaskRecord {
        id: uuid::Uuid::now_v7(),
        project_id: project,
        worktree_id: WorktreeId(uuid::Uuid::now_v7()),
        title: title.to_owned(),
        worktree_path: Some(path),
        branch: Some(format!("agent/task-{offset}")),
        status: TaskStatus::Active,
        created_at: time::OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(offset),
        closed_at: None,
    }
}
