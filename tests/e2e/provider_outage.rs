mod temporal_fixture;

use brain_service::{BrainItemsResponse, BrainStatusResponse};
use temporal_fixture::{TemporalFixture, call};

#[test]
fn canonical_fts_retrieval_survives_every_optional_provider_being_absent() {
    let fixture = TemporalFixture::seeded();
    assert!(!fixture.brain_home.join("basic-memory").exists());
    assert!(!fixture.brain_home.join("llm-wiki").exists());
    assert!(!fixture.brain_home.join("codegraph").exists());
    let mut server = fixture.server();

    let search: BrainItemsResponse = serde_json::from_value(call(
        &mut server,
        1,
        "brain_search",
        serde_json::json!({"project": fixture.project_id.0, "text": "OAuth callback"}),
    ))
    .expect("search result");
    assert!(!search.items.is_empty());
    assert!(search.items.iter().any(|item| item.text.contains("OAuth")));
    assert_eq!(search.optional_provider_state, "canonical_fts_only");
    assert!(search.items.iter().all(|item| !item.citations.is_empty()));

    let status: BrainStatusResponse = serde_json::from_value(call(
        &mut server,
        2,
        "brain_status",
        serde_json::json!({"project": fixture.project_id.0}),
    ))
    .expect("status result");
    assert_eq!(status.canonical_retrieval, "sqlite_fts5_available");
    assert_eq!(
        status.optional_providers,
        "not_required_for_canonical_retrieval"
    );
}
