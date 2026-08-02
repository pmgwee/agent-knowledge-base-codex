mod multi_agent_fixture;

use brain_domain::Harness;
use multi_agent_fixture::MultiAgentFixture;

#[tokio::test]
async fn codex_continues_work_recorded_by_claude_codex_and_hermes() {
    let fixture = MultiAgentFixture::new();
    fixture.capture().await;

    let ledger = fixture.ledger(&fixture.registered_a);
    assert!(
        ledger
            .event_count_by_harness(Harness::ClaudeCode)
            .expect("count Claude")
            > 0
    );
    assert!(
        ledger
            .event_count_by_harness(Harness::Codex)
            .expect("count Codex")
            > 0
    );
    assert!(
        ledger
            .event_count_by_harness(Harness::Hermes)
            .expect("count Hermes")
            > 0
    );

    let context = fixture.context(Harness::Codex, &fixture.project_a);
    assert!(context.contains("OAuth callback"), "{context}");
    assert!(context.contains("redirect URI"), "{context}");
    assert!(context.contains("pass"), "{context}");
    assert!(context.contains("PKCE"), "{context}");
    assert!(!context.contains("PROJECT_B_ONLY"), "{context}");
    assert!(context.split_whitespace().count() < 1_500);
}

#[tokio::test]
async fn post_compaction_reorients_to_the_same_task_and_checkpoint() {
    let fixture = MultiAgentFixture::new();
    fixture.capture().await;

    let first = fixture.context(Harness::Codex, &fixture.project_a);
    let resumed = fixture.context(Harness::Codex, &fixture.project_a);
    assert_eq!(first, resumed);
    assert!(resumed.contains("OAuth callback"));
    assert!(resumed.contains("next add PKCE"));
}
