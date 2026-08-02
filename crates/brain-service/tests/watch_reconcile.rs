use std::sync::Arc;
use std::time::Duration;

use brain_adapters::{ClaudeAdapter, NormalizeContext, SourceDescriptor};
use brain_domain::{ProjectId, WorktreeId};
use brain_service::{CaptureBinding, CaptureSupervisor};

#[tokio::test]
async fn running_service_captures_a_newly_flushed_record_and_stops_cleanly() {
    let temp = tempfile::tempdir().expect("create watcher fixture");
    let transcript = temp.path().join("session.jsonl");
    std::fs::write(&transcript, "").expect("create empty transcript");
    let project_id = ProjectId(uuid::Uuid::now_v7());
    let binding = CaptureBinding::new(
        Arc::new(ClaudeAdapter::new(temp.path())),
        SourceDescriptor::file(&transcript),
        NormalizeContext {
            project_id,
            worktree_id: WorktreeId(uuid::Uuid::now_v7()),
            source_schema: "claude-jsonl:fixture".to_owned(),
        },
        temp.path().join("events.db"),
    );
    let supervisor = Arc::new(CaptureSupervisor::new(vec![binding]).expect("create supervisor"));
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let task_supervisor = Arc::clone(&supervisor);
    let service = tokio::spawn(async move { task_supervisor.run(shutdown_rx).await });

    std::fs::write(
        &transcript,
        "{\"type\":\"future-event\",\"sessionId\":\"watcher\",\"marker\":\"FLUSHED\"}\n",
    )
    .expect("flush new transcript record");
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            if supervisor.event_count(project_id).expect("count events") == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("watcher or reconciliation should capture within deadline");

    shutdown_tx.send(true).expect("request service shutdown");
    service
        .await
        .expect("join service task")
        .expect("stop service cleanly");
}
