use brain_adapters::{
    AdapterConformanceSubject, ClaudeAdapter, CodexAdapter, NormalizeContext, SourceDescriptor,
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
