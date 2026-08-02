#![allow(dead_code)]
#![allow(clippy::too_many_arguments)]

use std::fs;

use brain_domain::{
    Authority, EventBatch, EventType, Harness, MemoryKind, MemoryRecord, MemoryScope, MemoryStatus,
    NormalizedEvent, ProjectId, ProjectRegistry, SourceCursor, WorktreeId,
};
use brain_mcp::{BrainTools, McpServer};
use brain_service::{BrainQueryService, ServiceLaunchConfig, ServiceProjectConfig};
use brain_store::EventLedger;
use sha2::{Digest, Sha256};

pub struct TemporalFixture {
    _temp: tempfile::TempDir,
    pub brain_home: std::path::PathBuf,
    pub project_id: ProjectId,
    pub now: time::OffsetDateTime,
    pub old_memory_id: uuid::Uuid,
    pub old_version_id: uuid::Uuid,
    config: ServiceLaunchConfig,
}

impl TemporalFixture {
    pub fn seeded() -> Self {
        let temp = tempfile::tempdir().expect("temp directory");
        let brain_home = temp.path().join("brain");
        let project_root = temp.path().join("project");
        fs::create_dir_all(&project_root).expect("project root");
        let identity = ProjectRegistry::open(&brain_home)
            .expect("registry")
            .register(&project_root)
            .expect("register project");
        let ledger_path = brain_home
            .join("projects")
            .join(identity.project_id.0.to_string())
            .join("ledger.sqlite");
        let now = time::OffsetDateTime::parse(
            "2026-08-02T12:00:00Z",
            &time::format_description::well_known::Rfc3339,
        )
        .expect("fixed clock");
        let mut ledger = EventLedger::open(&ledger_path, identity.project_id).expect("ledger");
        let events = vec![
            event(
                identity.project_id,
                identity.worktree_id,
                Harness::ClaudeCode,
                "claude-week",
                EventType::UserPrompted,
                now - time::Duration::days(6),
                now - time::Duration::days(6),
                "Implement OAuth callback using cookie auth storage",
            ),
            event(
                identity.project_id,
                identity.worktree_id,
                Harness::Codex,
                "codex-week",
                EventType::CheckpointAuthored,
                now - time::Duration::days(4),
                now - time::Duration::days(3),
                "OAuth callback works; PKCE remains",
            ),
            event(
                identity.project_id,
                identity.worktree_id,
                Harness::Hermes,
                "hermes-week",
                EventType::TestCompleted,
                now - time::Duration::days(2),
                now - time::Duration::days(2),
                "OAuth callback tests pass",
            ),
        ];
        let first_event_id = events[0].event_id;
        ledger
            .append_batch(&EventBatch {
                source_id: "temporal-fixture".to_owned(),
                events,
                quarantined: Vec::new(),
                capture_gaps: Vec::new(),
                next_cursor: SourceCursor::byte_offset(3),
            })
            .expect("append events");
        let old_memory_id = uuid::Uuid::now_v7();
        let old_version_id = uuid::Uuid::now_v7();
        ledger
            .append_memory(&MemoryRecord {
                id: old_memory_id,
                version_id: old_version_id,
                scope: MemoryScope::Project(identity.project_id),
                worktree_id: Some(identity.worktree_id),
                task_id: None,
                kind: MemoryKind::Decision,
                title: "Auth storage decision".to_owned(),
                content: "Use cookie auth storage".to_owned(),
                valid_from: now - time::Duration::days(6),
                valid_to: None,
                recorded_at: now - time::Duration::days(6),
                confidence: 1.0,
                authority: Authority::AgentCheckpoint,
                evidence_ids: vec![first_event_id],
                supersedes: Vec::new(),
                status: MemoryStatus::Current,
            })
            .expect("append old decision");
        let mut config = ServiceLaunchConfig::new(r"\\.\pipe\temporal-fixture");
        config.upsert_project(ServiceProjectConfig {
            project_root: identity.root,
            project_id: identity.project_id,
            worktree_id: identity.worktree_id,
            ledger_path,
            claude_sources: Vec::new(),
            codex_sources: Vec::new(),
            hermes_database: None,
        });
        Self {
            _temp: temp,
            brain_home,
            project_id: identity.project_id,
            now,
            old_memory_id,
            old_version_id,
            config,
        }
    }

    pub fn server(&self) -> McpServer {
        let service = BrainQueryService::from_config(&self.brain_home, self.config.clone())
            .expect("query service");
        let mut server = McpServer::new(BrainTools::new(service));
        let initialized = server
            .handle_line(
                &serde_json::json!({
                    "jsonrpc": "2.0", "id": 0, "method": "initialize",
                    "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "e2e", "version": "1"}}
                })
                .to_string(),
            )
            .expect("initialize response");
        let response: serde_json::Value =
            serde_json::from_str(&initialized).expect("initialize JSON");
        assert!(response.get("result").is_some());
        server
    }

    pub fn ledger(&self) -> EventLedger {
        EventLedger::open(&self.config.projects[0].ledger_path, self.project_id).expect("ledger")
    }
}

pub fn call(
    server: &mut McpServer,
    id: u64,
    name: &str,
    arguments: serde_json::Value,
) -> serde_json::Value {
    let response = server
        .handle_line(
            &serde_json::json!({
                "jsonrpc": "2.0", "id": id, "method": "tools/call",
                "params": {"name": name, "arguments": arguments}
            })
            .to_string(),
        )
        .expect("tool response");
    let response: serde_json::Value = serde_json::from_str(&response).expect("tool JSON");
    assert_eq!(response["result"]["isError"], false, "{response}");
    response["result"]["structuredContent"].clone()
}

fn event(
    project_id: ProjectId,
    worktree_id: WorktreeId,
    harness: Harness,
    session: &str,
    event_type: EventType,
    occurred_at: time::OffsetDateTime,
    observed_at: time::OffsetDateTime,
    text: &str,
) -> NormalizedEvent {
    let raw_hash: [u8; 32] = Sha256::digest(text.as_bytes()).into();
    let idempotency_key: [u8; 32] = Sha256::digest(
        [
            harness.as_str().as_bytes(),
            session.as_bytes(),
            event_type.as_str().as_bytes(),
        ]
        .concat(),
    )
    .into();
    NormalizedEvent {
        event_id: uuid::Uuid::now_v7(),
        project_id,
        worktree_id,
        task_id: None,
        harness,
        native_session_id: session.to_owned(),
        native_turn_id: None,
        event_type,
        occurred_at,
        observed_at,
        source_locator: format!("fixture://{session}"),
        source_offset: 1,
        source_schema: "temporal-fixture:v1".to_owned(),
        raw_hash,
        idempotency_key,
        git_head: Some("fixture-head".to_owned()),
        git_branch: Some("main".to_owned()),
        payload: serde_json::json!({"text": text}),
        raw: serde_json::json!({"text": text}),
    }
}
