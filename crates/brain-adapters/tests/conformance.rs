use brain_adapters::{
    AdapterConformanceSubject, ClaudeAdapter, NormalizeContext, SourceDescriptor,
    assert_adapter_conformance,
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

fn fixture_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
        .join("claude")
}
