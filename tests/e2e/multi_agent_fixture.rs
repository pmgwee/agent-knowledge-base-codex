#![allow(dead_code)]

use std::path::{Path, PathBuf};

use brain_cli::{
    AgentSourceOptions, RegisterOptions, RegistrationResult, register_project_with_sources,
};
use brain_domain::{Harness, HookEnvelope};
use brain_service::{
    CaptureSupervisor, ProjectHookHandler, ServiceLaunchConfig, build_capture_bindings,
    build_hook_bindings,
};
use brain_store::EventLedger;
use rusqlite::{Connection, params};

pub struct MultiAgentFixture {
    _temp: tempfile::TempDir,
    pub brain_home: PathBuf,
    pub project_a: PathBuf,
    pub project_b: PathBuf,
    pub registered_a: RegistrationResult,
    pub registered_b: RegistrationResult,
    hermes_db: PathBuf,
}

impl MultiAgentFixture {
    pub fn new() -> Self {
        let temp = tempfile::tempdir().expect("create multi-agent fixture");
        let brain_home = temp.path().join("brain");
        let project_a = temp.path().join("project-a");
        let project_b = temp.path().join("project-b");
        std::fs::create_dir_all(&project_a).expect("create project A");
        std::fs::create_dir_all(&project_b).expect("create project B");
        let claude_a = temp.path().join("claude-a.jsonl");
        let claude_b = temp.path().join("claude-b.jsonl");
        let codex_a = temp.path().join("rollout-a.jsonl");
        let codex_b = temp.path().join("rollout-b.jsonl");
        write_claude(
            &claude_a,
            &project_a,
            "Implement OAuth callback",
            "callback test is failing",
        );
        write_claude(&claude_b, &project_b, "PROJECT_B_ONLY", "PROJECT_B_ONLY");
        write_codex(
            &codex_a,
            &project_a,
            "Continue OAuth callback",
            "PKCE implementation pending",
        );
        write_codex(&codex_b, &project_b, "PROJECT_B_ONLY", "PROJECT_B_ONLY");
        let hermes_db = temp.path().join("state.db");
        create_hermes(&hermes_db, &project_a, &project_b);

        let registered_a = register(&brain_home, &project_a, &claude_a, &codex_a, &hermes_db);
        let registered_b = register(&brain_home, &project_b, &claude_b, &codex_b, &hermes_db);
        Self {
            _temp: temp,
            brain_home,
            project_a,
            project_b,
            registered_a,
            registered_b,
            hermes_db,
        }
    }

    pub async fn capture(&self) -> CaptureSupervisor {
        let config = self.config();
        let supervisor = CaptureSupervisor::new(
            build_capture_bindings(&config).expect("build all adapter bindings"),
        )
        .expect("create multi-project supervisor");
        supervisor.capture_once().await.expect("capture all agents");
        supervisor
    }

    pub fn context(&self, harness: Harness, project: &Path) -> String {
        let config = self.config();
        let handler = ProjectHookHandler::for_projects(build_hook_bindings(&config))
            .expect("create multi-project hook handler");
        handler
            .handle(&HookEnvelope {
                protocol: brain_domain::HOOK_PROTOCOL_VERSION,
                harness,
                event_name: "SessionStart".to_owned(),
                received_at: time::OffsetDateTime::now_utc(),
                nonce: uuid::Uuid::now_v7(),
                payload: serde_json::json!({
                    "cwd": project,
                    "session_id": "fresh-cross-agent-session",
                    "source": "compact"
                }),
            })
            .expect("compile shared context")
            .reply
            .additional_context
            .unwrap_or_default()
    }

    pub fn ledger(&self, registration: &RegistrationResult) -> EventLedger {
        EventLedger::open(&registration.ledger_path, registration.project_id)
            .expect("open project ledger")
    }

    pub fn config(&self) -> ServiceLaunchConfig {
        ServiceLaunchConfig::load(ServiceLaunchConfig::default_path(&self.brain_home))
            .expect("load multi-agent service config")
    }

    pub fn hermes_path(&self) -> &Path {
        &self.hermes_db
    }
}

fn register(
    brain_home: &Path,
    project: &Path,
    claude: &Path,
    codex: &Path,
    hermes: &Path,
) -> RegistrationResult {
    register_project_with_sources(
        RegisterOptions {
            brain_home: brain_home.to_path_buf(),
            project_path: project.to_path_buf(),
            claude_projects_root: None,
            explicit_claude_sources: vec![claude.to_path_buf()],
            pipe_name: None,
        },
        AgentSourceOptions {
            configure_codex: true,
            codex_sessions_root: None,
            explicit_codex_sources: vec![codex.to_path_buf()],
            configure_hermes: true,
            hermes_database: Some(hermes.to_path_buf()),
        },
    )
    .expect("register all provider sources")
}

fn write_claude(path: &Path, cwd: &Path, task: &str, outcome: &str) {
    let lines = [
        serde_json::json!({
            "type": "user", "sessionId": "claude-session", "uuid": "claude-user",
            "timestamp": "2026-08-02T01:00:00Z", "cwd": cwd,
            "message": {"content": task}
        }),
        serde_json::json!({
            "type": "assistant", "sessionId": "claude-session", "uuid": "claude-assistant",
            "timestamp": "2026-08-02T01:01:00Z", "cwd": cwd,
            "message": {"content": [{"type": "text", "text": outcome}]}
        }),
    ];
    std::fs::write(
        path,
        lines
            .iter()
            .map(serde_json::Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n",
    )
    .expect("write Claude fixture");
}

fn write_codex(path: &Path, cwd: &Path, task: &str, outcome: &str) {
    let checkpoint = if task == "PROJECT_B_ONLY" {
        "PROJECT_B_ONLY"
    } else {
        "OAuth callback; next add PKCE"
    };
    let lines = [
        serde_json::json!({
            "timestamp": "2026-08-02T01:02:00Z", "type": "session_meta",
            "payload": {"id": "codex-session", "cwd": cwd, "git": {"branch": "main"}}
        }),
        serde_json::json!({
            "timestamp": "2026-08-02T01:03:00Z", "type": "event_msg",
            "payload": {"type": "user_message", "message": task}
        }),
        serde_json::json!({
            "timestamp": "2026-08-02T01:04:00Z", "type": "response_item",
            "payload": {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": outcome}]}
        }),
        serde_json::json!({
            "timestamp": "2026-08-02T01:05:00Z", "type": "compacted",
            "payload": {"summary": checkpoint}
        }),
    ];
    std::fs::write(
        path,
        lines
            .iter()
            .map(serde_json::Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n",
    )
    .expect("write Codex fixture");
}

fn create_hermes(path: &Path, project_a: &Path, project_b: &Path) {
    let connection = Connection::open(path).expect("create Hermes fixture");
    connection
        .execute_batch(include_str!("../../fixtures/hermes/schema.sql"))
        .expect("apply reviewed Hermes schema");
    insert_hermes_session(
        &connection,
        "hermes-a",
        project_a,
        "OAuth callback: fixed redirect URI",
        "OAuth callback tests now pass",
        1_786_000_000.0,
    );
    insert_hermes_session(
        &connection,
        "hermes-b",
        project_b,
        "PROJECT_B_ONLY",
        "PROJECT_B_ONLY",
        1_786_000_100.0,
    );
}

fn insert_hermes_session(
    connection: &Connection,
    session_id: &str,
    cwd: &Path,
    task: &str,
    outcome: &str,
    timestamp: f64,
) {
    connection
        .execute(
            "INSERT INTO sessions(id, source, started_at, cwd, git_branch) VALUES (?1, 'cli', ?2, ?3, 'main')",
            params![session_id, timestamp, cwd.to_string_lossy()],
        )
        .expect("insert Hermes session");
    connection
        .execute(
            "INSERT INTO messages(session_id, role, content, timestamp) VALUES (?1, 'user', ?2, ?3)",
            params![session_id, task, timestamp + 1.0],
        )
        .expect("insert Hermes user message");
    connection
        .execute(
            "INSERT INTO messages(session_id, role, content, timestamp) VALUES (?1, 'assistant', ?2, ?3)",
            params![session_id, outcome, timestamp + 2.0],
        )
        .expect("insert Hermes assistant message");
}
