use brain_context::{ContextCompiler, ContextQuery};
use brain_domain::{ProjectId, WorktreeId};

#[test]
fn weak_or_missing_evidence_produces_explicit_unknown() {
    let project = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let mut query = ContextQuery::for_worktree(project, worktree);
    query.prompt = Some("why was the cache removed?".to_owned());
    let context = ContextCompiler::from_events(Vec::new())
        .compile(query)
        .expect("compile empty context");
    assert!(context.text.contains("No reliable project evidence found"));
    assert!(context.evidence_ids.is_empty());
}
