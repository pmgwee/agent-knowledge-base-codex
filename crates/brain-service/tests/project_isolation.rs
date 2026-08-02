use std::sync::Arc;

use brain_adapters::{ClaudeAdapter, NormalizeContext, SourceDescriptor};
use brain_domain::{ProjectId, WorktreeId};
use brain_service::{CaptureBinding, CaptureSupervisor};

#[tokio::test]
async fn same_named_projects_never_share_evidence() {
    let temp = tempfile::tempdir().expect("create isolation fixture");
    let first_root = temp.path().join("one").join("api");
    let second_root = temp.path().join("two").join("api");
    std::fs::create_dir_all(&first_root).expect("create first project");
    std::fs::create_dir_all(&second_root).expect("create second project");
    let first_source = first_root.join("session.jsonl");
    let second_source = second_root.join("session.jsonl");
    std::fs::write(
        &first_source,
        "{\"type\":\"future-event\",\"sessionId\":\"a\",\"marker\":\"PROJECT_A_ONLY\"}\n",
    )
    .expect("write first transcript");
    std::fs::write(
        &second_source,
        "{\"type\":\"future-event\",\"sessionId\":\"b\",\"marker\":\"PROJECT_B_ONLY\"}\n",
    )
    .expect("write second transcript");
    let project_a = ProjectId(uuid::Uuid::now_v7());
    let project_b = ProjectId(uuid::Uuid::now_v7());
    let bindings = vec![
        binding(
            &first_root,
            &first_source,
            temp.path().join("brain-a.db"),
            project_a,
        ),
        binding(
            &second_root,
            &second_source,
            temp.path().join("brain-b.db"),
            project_b,
        ),
    ];
    let supervisor = CaptureSupervisor::new(bindings).expect("create capture supervisor");

    supervisor
        .capture_once()
        .await
        .expect("capture both projects");

    assert_eq!(supervisor.event_count(project_a).expect("count A"), 1);
    assert_eq!(supervisor.event_count(project_b).expect("count B"), 1);
    assert!(
        supervisor
            .raw_contains(project_a, "PROJECT_A_ONLY")
            .expect("query A")
    );
    assert!(
        !supervisor
            .raw_contains(project_a, "PROJECT_B_ONLY")
            .expect("query foreign marker from A")
    );
    assert!(
        !supervisor
            .raw_contains(project_b, "PROJECT_A_ONLY")
            .expect("query foreign marker from B")
    );
}

fn binding(
    root: &std::path::Path,
    source: &std::path::Path,
    ledger: std::path::PathBuf,
    project_id: ProjectId,
) -> CaptureBinding {
    CaptureBinding::new(
        Arc::new(ClaudeAdapter::new(root)),
        SourceDescriptor::file(source),
        NormalizeContext {
            project_id,
            worktree_id: WorktreeId(uuid::Uuid::now_v7()),
            source_schema: "claude-jsonl:fixture".to_owned(),
        },
        ledger,
    )
}
