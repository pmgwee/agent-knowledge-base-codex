use brain_domain::{Harness, ProjectId};
use brain_store::{ContextDelivery, EventLedger};

fn delivery(project_id: ProjectId, total: u64, at: time::OffsetDateTime) -> ContextDelivery {
    ContextDelivery {
        project_id,
        harness: Harness::ClaudeCode,
        native_session_id: Some("session-1".to_owned()),
        event_name: "SessionStart".to_owned(),
        delivered_at: at,
        total_tokens: total,
        memory_tokens: total.saturating_sub(50),
        coordination_tokens: 50,
        citation_count: 3,
    }
}

#[test]
fn deliveries_summarise_only_within_the_requested_window() {
    let project_id = ProjectId(uuid::Uuid::now_v7());
    let ledger = EventLedger::open_in_memory(project_id).expect("open ledger");
    let now = time::OffsetDateTime::now_utc();

    ledger
        .record_context_delivery(&delivery(project_id, 900, now - time::Duration::days(10)))
        .expect("record old delivery");
    ledger
        .record_context_delivery(&delivery(project_id, 1_200, now - time::Duration::hours(2)))
        .expect("record recent delivery");
    ledger
        .record_context_delivery(&delivery(project_id, 800, now - time::Duration::hours(1)))
        .expect("record recent delivery");

    let week = ledger
        .context_delivery_summary(now - time::Duration::days(7))
        .expect("summarise week");

    assert_eq!(week.deliveries, 2, "the ten-day-old delivery is outside");
    assert_eq!(week.total_tokens, 2_000);
    assert_eq!(week.max_tokens, 1_200);
    assert!((week.mean_tokens() - 1_000.0).abs() < f64::EPSILON);
}

#[test]
fn a_delivery_for_another_project_is_refused() {
    let project_id = ProjectId(uuid::Uuid::now_v7());
    let foreign_id = ProjectId(uuid::Uuid::now_v7());
    let ledger = EventLedger::open_in_memory(project_id).expect("open ledger");

    let refused = ledger.record_context_delivery(&delivery(
        foreign_id,
        1_000,
        time::OffsetDateTime::now_utc(),
    ));

    assert!(
        refused.is_err(),
        "a foreign delivery must not be attributed to this project"
    );
    let summary = ledger
        .context_delivery_summary(time::OffsetDateTime::UNIX_EPOCH)
        .expect("summarise");
    assert_eq!(summary.deliveries, 0);
}

#[test]
fn an_empty_window_summarises_to_zero_rather_than_failing() {
    let project_id = ProjectId(uuid::Uuid::now_v7());
    let ledger = EventLedger::open_in_memory(project_id).expect("open ledger");

    let summary = ledger
        .context_delivery_summary(time::OffsetDateTime::now_utc())
        .expect("summarise empty window");

    assert_eq!(summary.deliveries, 0);
    assert_eq!(summary.total_tokens, 0);
    assert!((summary.mean_tokens() - 0.0).abs() < f64::EPSILON);
}
