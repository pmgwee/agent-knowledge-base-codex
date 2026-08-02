use brain_context::{
    ContextProvider, ContextQuery, LlmWikiProvider, token_count, validate_llm_wiki_vault,
};
use brain_domain::{ProjectId, WorktreeId};

#[tokio::test]
async fn results_have_scope_dates_citations_trust_and_hard_budgets() {
    let temp = tempfile::tempdir().expect("temp");
    let brain = temp.path().join("brain");
    let vault = temp.path().join("llm wiki vault");
    std::fs::create_dir_all(&brain).expect("brain");
    std::fs::create_dir_all(&vault).expect("vault");
    for index in 0..5 {
        std::fs::write(
            vault.join(format!("oauth-{index}.md")),
            format!(
                "# OAuth reference {index}\nOAuth callback PKCE guidance {}",
                "detail ".repeat(500)
            ),
        )
        .expect("document");
    }
    let project = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let provider = LlmWikiProvider::new(
        project,
        worktree,
        &vault,
        &brain,
        std::time::Duration::from_millis(300),
        3,
        600,
    )
    .expect("provider");
    let mut query = ContextQuery::for_worktree(project, worktree);
    query.prompt = Some("implement OAuth callback PKCE".to_owned());
    let results = provider.retrieve(&query).await.expect("results");
    assert!(results.len() <= 3);
    assert!(results.iter().all(|item| item.project_id == project));
    assert!(results.iter().all(|item| item.source_date.is_some()));
    assert!(results.iter().all(|item| !item.source_uri.is_empty()));
    assert!(results.iter().all(|item| item.trust == "external_document"));
    let rendered = results
        .iter()
        .map(|item| format!("{} {} {}", item.title, item.content, item.source_uri))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(token_count(&rendered) <= 600);
}

#[test]
fn canonical_memory_paths_cannot_be_used_as_the_llm_wiki_vault() {
    let temp = tempfile::tempdir().expect("temp");
    let brain = temp.path().join("brain");
    let vault = brain.join("projects/wiki");
    std::fs::create_dir_all(&vault).expect("vault");
    assert!(validate_llm_wiki_vault(&brain, &vault).is_err());
}
