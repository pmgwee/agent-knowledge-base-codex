use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use brain_context::{
    CodeGraphCapabilities, CodeGraphClient, CodeGraphHit, CodeGraphIndex, CodeGraphProvider,
    ContextProvider, ContextQuery,
};
use brain_domain::{ProjectId, WorktreeId};

#[tokio::test]
async fn index_for_another_head_or_worktree_is_rejected() {
    let temp = tempfile::tempdir().expect("temp");
    let root = temp.path().join("worktree a");
    std::fs::create_dir_all(&root).expect("root");
    std::fs::write(root.join("auth.rs"), "fn callback() {}\n").expect("file");
    let project = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let provider = CodeGraphProvider::new(
        Arc::new(FixtureClient::new(&root, "old-head")),
        project,
        worktree,
        &root,
        "current-head",
        true,
    )
    .expect("provider");
    assert!(provider.validate_status().is_err());
    assert!(provider.retrieve(&prompt(project, worktree)).await.is_err());
}

#[tokio::test]
async fn code_hits_include_current_file_symbol_and_revision_provenance() {
    let temp = tempfile::tempdir().expect("temp");
    let root = temp.path().join("worktree");
    std::fs::create_dir_all(&root).expect("root");
    std::fs::write(root.join("auth.rs"), "fn callback() {}\n").expect("file");
    let project = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let provider = CodeGraphProvider::new(
        Arc::new(FixtureClient::new(&root, "head-a")),
        project,
        worktree,
        &root,
        "head-a",
        true,
    )
    .expect("provider");
    let hits = provider
        .retrieve(&prompt(project, worktree))
        .await
        .expect("hits");
    assert_eq!(hits.len(), 1);
    assert!(hits[0].source_uri.contains("auth.rs#L1-L1"));
    assert_eq!(hits[0].title, "callback");
    assert_eq!(hits[0].git_head.as_deref(), Some("head-a"));
}

struct FixtureClient {
    root: PathBuf,
    head: String,
}

impl FixtureClient {
    fn new(root: &Path, head: &str) -> Self {
        Self {
            root: root.to_path_buf(),
            head: head.to_owned(),
        }
    }

    fn index(&self) -> CodeGraphIndex {
        CodeGraphIndex {
            index_id: "fixture-index".to_owned(),
            worktree_path: self.root.clone(),
            git_head: self.head.clone(),
            provider_version: "fixture-1".to_owned(),
            indexed_at: time::OffsetDateTime::UNIX_EPOCH,
        }
    }
}

impl CodeGraphClient for FixtureClient {
    fn capabilities(&self) -> Result<CodeGraphCapabilities> {
        Ok(CodeGraphCapabilities {
            schema_version: 1,
            provider_version: "fixture-1".to_owned(),
            structured_search: true,
            index_identity: true,
        })
    }

    fn index(&self, _worktree: &Path) -> Result<CodeGraphIndex> {
        Ok(self.index())
    }

    fn status(&self, _worktree: &Path) -> Result<CodeGraphIndex> {
        Ok(self.index())
    }

    fn search(&self, _worktree: &Path, _query: &str, _limit: usize) -> Result<Vec<CodeGraphHit>> {
        Ok(vec![CodeGraphHit {
            file: PathBuf::from("auth.rs"),
            symbol: Some("callback".to_owned()),
            line_start: Some(1),
            line_end: Some(1),
            relationship: Some("called_by login".to_owned()),
            excerpt: "fn callback()".to_owned(),
            score: 0.95,
        }])
    }
}

fn prompt(project: ProjectId, worktree: WorktreeId) -> ContextQuery {
    let mut query = ContextQuery::for_worktree(project, worktree);
    query.prompt = Some("OAuth callback".to_owned());
    query
}
