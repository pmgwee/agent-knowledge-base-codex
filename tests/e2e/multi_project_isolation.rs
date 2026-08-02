mod multi_agent_fixture;

use brain_domain::Harness;
use multi_agent_fixture::MultiAgentFixture;

#[tokio::test]
async fn one_global_hermes_database_is_hard_scoped_into_separate_project_ledgers() {
    let fixture = MultiAgentFixture::new();
    let config = fixture.config();
    assert_eq!(config.projects.len(), 2);
    assert!(
        config
            .projects
            .iter()
            .all(|project| { project.hermes_database.as_deref() == Some(fixture.hermes_path()) })
    );

    let supervisor = fixture.capture().await;
    assert!(
        supervisor
            .event_count(fixture.registered_a.project_id)
            .expect("count A")
            > 0
    );
    assert!(
        supervisor
            .event_count(fixture.registered_b.project_id)
            .expect("count B")
            > 0
    );
    assert!(
        !supervisor
            .raw_contains(fixture.registered_a.project_id, "PROJECT_B_ONLY")
            .expect("scan A")
    );
    assert!(
        !supervisor
            .raw_contains(fixture.registered_b.project_id, "OAuth callback")
            .expect("scan B")
    );

    let a_context = fixture.context(Harness::ClaudeCode, &fixture.project_a);
    let b_context = fixture.context(Harness::Codex, &fixture.project_b);
    assert!(!a_context.contains("PROJECT_B_ONLY"), "{a_context}");
    assert!(b_context.contains("PROJECT_B_ONLY"), "{b_context}");
    assert!(!b_context.contains("OAuth callback"), "{b_context}");
}
