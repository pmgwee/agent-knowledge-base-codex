mod fixtures;

use brain_context::token_count;
use fixtures::E2eFixture;

#[tokio::test]
async fn fresh_session_receives_prior_task_without_export() {
    let e2e = E2eFixture::new();
    e2e.write_prior_claude_session("implement auth callback", "tests failed");
    let registered = e2e.register_project();
    e2e.capture(&registered).await;

    let context = e2e.start_fresh_claude_session(&registered).await;

    assert!(context.contains("implement auth callback"));
    assert!(context.contains("tests failed"));
    assert!(context.contains("Evidence: event:"));
    assert!(token_count(&context) <= 1_500);
}
