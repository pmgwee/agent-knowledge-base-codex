use std::io::Write;
use std::sync::Arc;

use brain_adapters::{ClaudeAdapter, NormalizeContext, SourceDescriptor};
use brain_domain::{ProjectId, WorktreeId};
use brain_service::{CaptureBinding, CaptureSupervisor};

#[tokio::test]
async fn restart_replays_from_committed_cursor_without_duplicates() {
    let fixture = ServiceFixture::new();
    fixture.write_records(10);

    let first = fixture.supervisor();
    first.capture_once().await.expect("capture initial records");
    assert_eq!(
        first.event_count(fixture.project_id).expect("count events"),
        10
    );
    drop(first);

    fixture.append_records(10, 5);
    let restarted = fixture.supervisor();
    restarted
        .capture_once()
        .await
        .expect("capture records after restart");

    assert_eq!(
        restarted
            .event_count(fixture.project_id)
            .expect("count replayed events"),
        15
    );
}

struct ServiceFixture {
    _temp: tempfile::TempDir,
    transcript: std::path::PathBuf,
    ledger: std::path::PathBuf,
    project_id: ProjectId,
    worktree_id: WorktreeId,
}

impl ServiceFixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("create service fixture");
        Self {
            transcript: temp.path().join("session.jsonl"),
            ledger: temp.path().join("brain").join("events.db"),
            project_id: ProjectId(uuid::Uuid::now_v7()),
            worktree_id: WorktreeId(uuid::Uuid::now_v7()),
            _temp: temp,
        }
    }

    fn write_records(&self, count: usize) {
        let mut file = std::fs::File::create(&self.transcript).expect("create transcript");
        write_records(&mut file, 0, count);
    }

    fn append_records(&self, start: usize, count: usize) {
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&self.transcript)
            .expect("open transcript for append");
        write_records(&mut file, start, count);
    }

    fn supervisor(&self) -> CaptureSupervisor {
        let adapter = Arc::new(ClaudeAdapter::new(
            self.transcript.parent().expect("transcript parent"),
        ));
        let binding = CaptureBinding::new(
            adapter,
            SourceDescriptor::file(&self.transcript),
            NormalizeContext {
                project_id: self.project_id,
                worktree_id: self.worktree_id,
                source_schema: "claude-jsonl:fixture".to_owned(),
            },
            &self.ledger,
        );
        CaptureSupervisor::new(vec![binding]).expect("create capture supervisor")
    }
}

fn write_records(file: &mut std::fs::File, start: usize, count: usize) {
    for sequence in start..start + count {
        writeln!(
            file,
            "{{\"type\":\"future-event\",\"sessionId\":\"session-1\",\"uuid\":\"turn-{sequence}\",\"timestamp\":\"2026-08-01T01:02:03Z\",\"sequence\":{sequence}}}"
        )
        .expect("write transcript record");
    }
    file.sync_all().expect("flush transcript records");
}
