use std::cell::RefCell;

use brain_adapters::{
    HermesActivation, HermesAdapter, NormalizeContext, ReadOutcome, SourceAdapter, SourceDescriptor,
};
use brain_domain::{EventType, Harness, ProjectId, SourceCursor, WorktreeId};

#[test]
fn hermes_reads_only_messages_after_the_composite_cursor() {
    let fixture = HermesFixture::reviewed();
    fixture.insert_session("session-1", r"C:\fixture\project");
    fixture.insert_message("session-1", "user", "implement auth callback", 1.0);
    fixture.insert_message("session-1", "assistant", "tests failed", 2.0);
    let adapter = HermesAdapter::reviewed(&fixture.path).expect("create reviewed adapter");
    let source = SourceDescriptor::file(&fixture.path);
    let first = batch(
        adapter
            .read_increment(&source, &SourceCursor::start())
            .expect("read first Hermes batch"),
    );
    assert_eq!(first.records.len(), 2);
    assert_eq!(first.next_cursor.byte_offset, 2);
    assert_eq!(
        first.next_cursor.native_position,
        Some(serde_json::json!({"session_id": "session-1", "message_id": 2}))
    );

    fixture.insert_message("session-1", "tool", "cargo test failed", 3.0);
    fixture.insert_message("session-1", "assistant", "fixed redirect URI", 4.0);
    let second = batch(
        adapter
            .read_increment(&source, &first.next_cursor)
            .expect("read incremental Hermes batch"),
    );

    assert_eq!(second.records.len(), 2);
    assert_eq!(
        second.records[0].value.as_ref().expect("first value")["message"]["id"],
        3
    );
    assert_eq!(second.next_cursor.byte_offset, 4);
    assert_eq!(
        second.next_cursor.native_position,
        Some(serde_json::json!({"session_id": "session-1", "message_id": 4}))
    );
}

#[test]
fn reviewed_hermes_rows_normalize_without_exposing_reasoning() {
    let fixture = HermesFixture::reviewed();
    fixture.insert_session("session-1", r"C:\fixture\project");
    fixture.insert_message_with_reasoning(
        "session-1",
        "assistant",
        "tests now pass",
        "private reasoning fixture",
        1.0,
    );
    let adapter = HermesAdapter::reviewed(&fixture.path).expect("create reviewed adapter");
    let source = SourceDescriptor::file(&fixture.path);
    let records = batch(
        adapter
            .read_increment(&source, &SourceCursor::start())
            .expect("read Hermes evidence"),
    )
    .records;
    let events = adapter
        .normalize(&records[0], &normalize_context())
        .expect("normalize Hermes evidence");

    assert_eq!(events[0].event_type, EventType::SessionStarted);
    assert_eq!(events[1].event_type, EventType::AgentResponded);
    assert_eq!(events[2].event_type, EventType::OpaqueEvidence);
    assert!(events.iter().all(|event| event.harness == Harness::Hermes));
    assert!(events[2].payload.get("reasoning").is_none());
    assert_eq!(events[2].payload["retention"], "raw-only");
    assert_eq!(
        events[2].raw["message"]["reasoning"],
        "private reasoning fixture"
    );
}

#[test]
fn unreviewed_hermes_schema_blocks_activation_and_cursor_advance() {
    let fixture = HermesFixture::drifted();
    let adapter = HermesAdapter::reviewed(&fixture.path).expect("create guarded adapter");
    let source = SourceDescriptor::file(&fixture.path);
    assert!(matches!(
        adapter.activation().expect("inspect activation"),
        HermesActivation::FixtureOnly { .. }
    ));

    let outcome = adapter
        .read_increment(&source, &SourceCursor::byte_offset(42))
        .expect("report schema drift");

    match outcome {
        ReadOutcome::SchemaDrift(drift) => {
            assert_ne!(drift.expected, drift.observed);
            assert!(drift.sample_hash.iter().any(|byte| *byte != 0));
        }
        other => panic!("expected schema drift, got {other:?}"),
    }
}

#[test]
fn read_only_adapter_sees_committed_wal_rows_without_checkpointing() {
    let fixture = HermesFixture::reviewed();
    fixture.enable_wal();
    fixture.insert_session("session-wal", r"C:\fixture\project");
    fixture.insert_message("session-wal", "user", "WAL-visible task", 1.0);
    let wal_path = fixture.path.with_extension("db-wal");
    let wal_size_before = std::fs::metadata(&wal_path)
        .expect("Hermes WAL exists")
        .len();
    let adapter = HermesAdapter::reviewed(&fixture.path).expect("create reviewed adapter");

    let rows = batch(
        adapter
            .read_increment(
                &SourceDescriptor::file(&fixture.path),
                &SourceCursor::start(),
            )
            .expect("read WAL-visible row"),
    );

    assert_eq!(rows.records.len(), 1);
    assert_eq!(
        std::fs::metadata(&wal_path)
            .expect("Hermes WAL remains")
            .len(),
        wal_size_before
    );
}

#[test]
fn production_binding_activates_only_for_reviewed_schema_and_filters_exact_project_scope() {
    let fixture = HermesFixture::reviewed();
    let project_a = fixture._temp.path().join("project-a");
    let project_b = fixture._temp.path().join("project-b");
    std::fs::create_dir_all(&project_a).expect("create project A");
    std::fs::create_dir_all(&project_b).expect("create project B");
    fixture.insert_session("session-a", project_a.to_string_lossy().as_ref());
    fixture.insert_session("session-b", project_b.to_string_lossy().as_ref());
    fixture.insert_child_session("session-a-child", "session-a");
    fixture.insert_message("session-a", "user", "PROJECT_A_TASK", 1.0);
    fixture.insert_message("session-a-child", "assistant", "PROJECT_A_CHILD", 2.0);
    fixture.insert_message("session-b", "user", "PROJECT_B_ONLY", 3.0);
    let adapter = HermesAdapter::reviewed_for_project(&fixture.path, &project_a)
        .expect("create production-bound Hermes adapter");

    assert!(matches!(
        adapter.activation().expect("inspect activation"),
        HermesActivation::Active { .. }
    ));
    let rows = batch(
        adapter
            .read_increment(
                &SourceDescriptor::file(&fixture.path),
                &SourceCursor::start(),
            )
            .expect("read project-bound Hermes rows"),
    );
    let raw = rows
        .records
        .iter()
        .map(|record| record.raw_text.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(raw.contains("PROJECT_A_TASK"));
    assert!(raw.contains("PROJECT_A_CHILD"));
    assert!(!raw.contains("PROJECT_B_ONLY"));
}

#[test]
#[ignore = "local installed-schema gate; reads LOCALAPPDATA/hermes/state.db"]
fn installed_hermes_v22_schema_matches_the_reviewed_profile() {
    let database = std::path::PathBuf::from(
        std::env::var_os("LOCALAPPDATA").expect("LOCALAPPDATA is available"),
    )
    .join("hermes")
    .join("state.db");
    let project = std::env::current_dir().expect("resolve current project");
    let adapter = HermesAdapter::reviewed_for_project(database, project)
        .expect("open installed Hermes database read-only");

    assert!(matches!(
        adapter.activation().expect("inspect installed schema"),
        HermesActivation::Active { .. }
    ));
}

fn batch(outcome: ReadOutcome) -> brain_adapters::RawRecordBatch {
    match outcome {
        ReadOutcome::Batch(batch) => batch,
        other => panic!("expected batch, got {other:?}"),
    }
}

fn normalize_context() -> NormalizeContext {
    NormalizeContext {
        project_id: ProjectId(uuid::Uuid::now_v7()),
        worktree_id: WorktreeId(uuid::Uuid::now_v7()),
        source_schema: "hermes-state:v22".to_owned(),
    }
}

struct HermesFixture {
    _temp: tempfile::TempDir,
    path: std::path::PathBuf,
    writer: RefCell<Option<rusqlite::Connection>>,
}

impl HermesFixture {
    fn reviewed() -> Self {
        Self::from_schema(include_str!("../../../fixtures/hermes/schema.sql"))
    }

    fn drifted() -> Self {
        Self::from_schema(include_str!("../../../fixtures/hermes/schema-drift.sql"))
    }

    fn from_schema(schema: &str) -> Self {
        let temp = tempfile::tempdir().expect("create Hermes fixture");
        let path = temp.path().join("state.db");
        rusqlite::Connection::open(&path)
            .expect("open Hermes fixture")
            .execute_batch(schema)
            .expect("apply Hermes schema");
        Self {
            _temp: temp,
            path,
            writer: RefCell::new(None),
        }
    }

    fn enable_wal(&self) {
        let connection = rusqlite::Connection::open(&self.path).expect("open WAL fixture");
        connection
            .execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;")
            .expect("enable WAL");
        *self.writer.borrow_mut() = Some(connection);
    }

    fn insert_session(&self, id: &str, cwd: &str) {
        self.with_connection(|connection| {
            connection.execute(
                "INSERT INTO sessions(id, source, started_at, cwd, git_branch) VALUES (?1, 'cli', 1.0, ?2, 'main')",
                rusqlite::params![id, cwd],
            )
        })
        .expect("insert Hermes session");
    }

    fn insert_message(&self, session_id: &str, role: &str, content: &str, timestamp: f64) {
        self.with_connection(|connection| {
            connection.execute(
                "INSERT INTO messages(session_id, role, content, timestamp) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![session_id, role, content, timestamp],
            )
        })
        .expect("insert Hermes message");
    }

    fn insert_child_session(&self, id: &str, parent_session_id: &str) {
        self.with_connection(|connection| {
            connection.execute(
                "INSERT INTO sessions(id, source, started_at, parent_session_id) VALUES (?1, 'subagent', 1.0, ?2)",
                rusqlite::params![id, parent_session_id],
            )
        })
        .expect("insert Hermes child session");
    }

    fn insert_message_with_reasoning(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
        reasoning: &str,
        timestamp: f64,
    ) {
        self.with_connection(|connection| {
            connection.execute(
                "INSERT INTO messages(session_id, role, content, reasoning, timestamp) VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![session_id, role, content, reasoning, timestamp],
            )
        })
        .expect("insert Hermes reasoning message");
    }

    fn with_connection<T>(&self, operation: impl FnOnce(&rusqlite::Connection) -> T) -> T {
        let writer = self.writer.borrow();
        if let Some(connection) = writer.as_ref() {
            return operation(connection);
        }
        drop(writer);
        let connection = rusqlite::Connection::open(&self.path).expect("open Hermes fixture");
        operation(&connection)
    }
}
