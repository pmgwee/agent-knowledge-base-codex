use std::fs;

use brain_coordination::{CoordinationStore, TaskRecord, TaskStatus};
use brain_domain::{HOOK_PROTOCOL_VERSION, Harness, HookEnvelope, ProjectId, WorktreeId};
use brain_service::{HookProjectBinding, ProjectHookHandler};

#[test]
fn activity_renews_at_five_minute_intervals_and_session_end_releases() {
    let fixture = Fixture::new();
    let handler = fixture.handler();
    let start = fixture.envelope("SessionStart", Harness::ClaudeCode, "claude", fixture.now);
    handler.handle(&start).expect("start");
    handler
        .handle(&fixture.envelope(
            "UserPromptSubmit",
            Harness::ClaudeCode,
            "claude",
            fixture.now + time::Duration::minutes(4),
        ))
        .expect("early activity");
    handler
        .handle(&fixture.envelope(
            "PostToolUse",
            Harness::ClaudeCode,
            "claude",
            fixture.now + time::Duration::minutes(6),
        ))
        .expect("renew activity");
    let events = fixture.store().coordination_events().expect("events");
    assert_eq!(
        events
            .iter()
            .filter(|event| event.action == "renew")
            .count(),
        1
    );
    handler
        .handle(&fixture.envelope(
            "SessionEnd",
            Harness::ClaudeCode,
            "claude",
            fixture.now + time::Duration::minutes(7),
        ))
        .expect("end");
    assert!(
        fixture
            .store()
            .lease(fixture.task.id)
            .expect("lease")
            .is_none()
    );
}

#[test]
fn a_second_writer_receives_owner_and_separate_worktree_guidance() {
    let fixture = Fixture::new();
    let handler = fixture.handler();
    handler
        .handle(&fixture.envelope("SessionStart", Harness::ClaudeCode, "claude", fixture.now))
        .expect("first start");
    let second = handler
        .handle(&fixture.envelope(
            "SessionStart",
            Harness::Codex,
            "codex",
            fixture.now + time::Duration::seconds(1),
        ))
        .expect("second start")
        .reply
        .additional_context
        .expect("warning context");
    assert!(second.contains("writer lease is held"), "{second}");
    assert!(second.contains("claude-session"), "{second}");
    assert!(second.contains("separate task worktree"), "{second}");
}

struct Fixture {
    _temp: tempfile::TempDir,
    main: std::path::PathBuf,
    task_path: std::path::PathBuf,
    ledger: std::path::PathBuf,
    project: ProjectId,
    main_worktree: WorktreeId,
    task: TaskRecord,
    now: time::OffsetDateTime,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("temp");
        let main = temp.path().join("main");
        let task_path = temp.path().join("tasks").join("oauth");
        fs::create_dir_all(&main).expect("main");
        fs::create_dir_all(&task_path).expect("task path");
        let project = ProjectId(uuid::Uuid::now_v7());
        let now = time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(20_000);
        let task = TaskRecord {
            id: uuid::Uuid::now_v7(),
            project_id: project,
            worktree_id: WorktreeId(uuid::Uuid::now_v7()),
            title: "OAuth callback".to_owned(),
            worktree_path: Some(task_path.clone()),
            branch: Some("agent/oauth".to_owned()),
            status: TaskStatus::Active,
            created_at: now,
            closed_at: None,
        };
        let ledger = temp.path().join("ledger.sqlite");
        let mut store = CoordinationStore::open(&ledger, project).expect("store");
        store.create_task(&task).expect("task");
        Self {
            _temp: temp,
            main,
            task_path,
            ledger,
            project,
            main_worktree: WorktreeId(uuid::Uuid::now_v7()),
            task,
            now,
        }
    }

    fn handler(&self) -> ProjectHookHandler {
        ProjectHookHandler::new(HookProjectBinding {
            project_root: self.main.clone(),
            project_id: self.project,
            worktree_id: self.main_worktree,
            ledger_path: self.ledger.clone(),
            global_preferences_path: None,
        })
        .expect("handler")
    }

    fn store(&self) -> CoordinationStore {
        CoordinationStore::open(&self.ledger, self.project).expect("store")
    }

    fn envelope(
        &self,
        event_name: &str,
        harness: Harness,
        session: &str,
        received_at: time::OffsetDateTime,
    ) -> HookEnvelope {
        HookEnvelope {
            protocol: HOOK_PROTOCOL_VERSION,
            harness,
            event_name: event_name.to_owned(),
            received_at,
            nonce: uuid::Uuid::now_v7(),
            payload: serde_json::json!({
                "cwd": self.task_path,
                "session_id": format!("{session}-session"),
                "brain_task_id": self.task.id,
            }),
        }
    }
}
