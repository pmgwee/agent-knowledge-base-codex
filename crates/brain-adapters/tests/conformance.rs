use brain_adapters::{
    AdapterConformanceSubject, ClaudeAdapter, CodexAdapter, HermesAdapter, NormalizeContext,
    SourceDescriptor, assert_adapter_conformance,
};
use brain_domain::{ProjectId, WorktreeId};

#[test]
fn claude_satisfies_the_source_adapter_contract() {
    let adapter = ClaudeAdapter::new(fixture_root());
    let project_id = ProjectId(uuid::Uuid::now_v7());
    let worktree_id = WorktreeId(uuid::Uuid::now_v7());
    let report = assert_adapter_conformance(AdapterConformanceSubject {
        adapter: &adapter,
        source: SourceDescriptor::file(fixture_root().join("session.jsonl")),
        context: NormalizeContext {
            project_id,
            worktree_id,
            source_schema: "claude-jsonl:conformance".to_owned(),
        },
    })
    .expect("Claude adapter conforms");

    assert!(report.raw_records > 0);
    assert!(report.normalized_events > 0);
    assert_eq!(report.project_id, project_id);
    assert_eq!(report.worktree_id, worktree_id);
}

#[test]
fn codex_satisfies_the_source_adapter_contract() {
    let root = codex_fixture_root();
    let adapter = CodexAdapter::new(vec![root.clone()]);
    let report = assert_adapter_conformance(AdapterConformanceSubject {
        adapter: &adapter,
        source: SourceDescriptor::file(root.join("rollout.jsonl")),
        context: NormalizeContext {
            project_id: ProjectId(uuid::Uuid::now_v7()),
            worktree_id: WorktreeId(uuid::Uuid::now_v7()),
            source_schema: "codex-rollout:conformance".to_owned(),
        },
    })
    .expect("Codex adapter conforms");

    assert!(report.raw_records > 0);
    assert!(report.normalized_events > 0);
}

#[test]
fn hermes_satisfies_the_source_adapter_contract_in_review_mode() {
    let temp = tempfile::tempdir().expect("create Hermes conformance fixture");
    let path = temp.path().join("state.db");
    let connection = rusqlite::Connection::open(&path).expect("open Hermes conformance database");
    connection
        .execute_batch(include_str!("../../../fixtures/hermes/schema.sql"))
        .expect("apply reviewed Hermes schema");
    connection
        .execute(
            "INSERT INTO sessions(id, source, started_at, cwd) VALUES ('session-1', 'cli', 1.0, 'C:\\fixture\\project')",
            [],
        )
        .expect("insert Hermes conformance session");
    connection
        .execute(
            "INSERT INTO messages(session_id, role, content, timestamp) VALUES ('session-1', 'user', 'fixture task', 1.0)",
            [],
        )
        .expect("insert Hermes conformance message");
    drop(connection);
    let adapter = HermesAdapter::reviewed(&path).expect("create Hermes conformance adapter");

    let report = assert_adapter_conformance(AdapterConformanceSubject {
        adapter: &adapter,
        source: SourceDescriptor::file(path),
        context: NormalizeContext {
            project_id: ProjectId(uuid::Uuid::now_v7()),
            worktree_id: WorktreeId(uuid::Uuid::now_v7()),
            source_schema: "hermes-state:conformance".to_owned(),
        },
    })
    .expect("Hermes adapter conforms");

    assert_eq!(report.raw_records, 1);
    assert_eq!(report.normalized_events, 2);
    assert!(report.committed_cursor.native_position.is_some());
}

fn fixture_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
        .join("claude")
}

fn codex_fixture_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
        .join("codex")
}
