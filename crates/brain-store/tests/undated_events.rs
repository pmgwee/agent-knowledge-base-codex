//! An undated record must not become a 1970 record.
//!
//! Measured across the three live ledgers before this shipped: **10,187 events** carried
//! `occurred_at = 0` beside a perfectly good `observed_at`, because every adapter defaults an
//! unparseable timestamp to `UNIX_EPOCH`.
//!
//! The epoch is not a missing value, it is a wrong one, and wrong in the worst available direction:
//! it sorts to the *front* of every chronological view, so the records we know least about lead the
//! timeline. It reaches past the timeline too — `resolve_candidates` breaks contradictions on
//! recency, so an epoch-dated claim reads as the oldest side of every disagreement it joins, and
//! `brain lint`'s four "misdated" memories were simply inheriting the dates of their evidence.
//!
//! The substitution lives at the ledger boundary, not in the adapters, because
//! `assert_adapter_contract` requires normalisation to be a pure function of the record. A
//! `now_utc()` fallback inside an adapter breaks that — which is how the first attempt at this fix
//! failed, caught by the conformance suite rather than by a reviewer.

use brain_domain::{
    EventBatch, EventType, Harness, NormalizedEvent, ProjectId, SourceCursor, WorktreeId,
};
use brain_store::EventLedger;

#[test]
fn an_undated_event_is_stored_at_its_observation_time() {
    let project = ProjectId(uuid::Uuid::now_v7());
    let mut ledger = EventLedger::open_in_memory(project).expect("open");
    let observed = time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(20_000);

    append(
        &mut ledger,
        project,
        time::OffsetDateTime::UNIX_EPOCH,
        observed,
        1,
    );

    let stored = ledger.recent_events(project, 10).expect("read");
    assert_eq!(stored.len(), 1);
    assert_eq!(
        stored[0].occurred_at, observed,
        "an undated record carries the bound capture already held"
    );
}

#[test]
fn a_dated_event_keeps_its_own_date() {
    // The case that made the defect invisible: 92% of records parse fine, so nothing looked wrong.
    let project = ProjectId(uuid::Uuid::now_v7());
    let mut ledger = EventLedger::open_in_memory(project).expect("open");
    let occurred = time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(19_000);
    let observed = time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(20_000);

    append(&mut ledger, project, occurred, observed, 2);

    let stored = ledger.recent_events(project, 10).expect("read");
    assert_eq!(
        stored[0].occurred_at, occurred,
        "the substitution must never shadow a real date"
    );
}

#[test]
fn an_undated_event_with_an_undated_observation_is_left_alone() {
    // Both unset means nothing in the row knows better, and inventing a date would be worse than
    // an obvious 1970. This is the branch that keeps the rule honest rather than merely tidy.
    let project = ProjectId(uuid::Uuid::now_v7());
    let mut ledger = EventLedger::open_in_memory(project).expect("open");
    let epoch = time::OffsetDateTime::UNIX_EPOCH;

    append(&mut ledger, project, epoch, epoch, 3);

    let stored = ledger.recent_events(project, 10).expect("read");
    assert_eq!(stored[0].occurred_at, epoch);
}

#[test]
fn the_repair_moves_only_undated_rows() {
    // The repair for what is already captured. It updates events in place — the one place in this
    // ledger that does — so the test pins that it touches nothing else: same row count, and a
    // dated event unmoved.
    let project = ProjectId(uuid::Uuid::now_v7());
    let mut ledger = EventLedger::open_in_memory(project).expect("open");
    let occurred = time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(19_000);
    let observed = time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(20_000);
    append(&mut ledger, project, occurred, observed, 4);

    // Nothing to repair — the boundary already substituted on the way in.
    assert_eq!(ledger.repair_epoch_event_dates().expect("repair"), 0);
    assert_eq!(ledger.event_count().expect("count"), 1);
    let stored = ledger.recent_events(project, 10).expect("read");
    assert_eq!(stored[0].occurred_at, occurred);
}

fn append(
    ledger: &mut EventLedger,
    project: ProjectId,
    occurred_at: time::OffsetDateTime,
    observed_at: time::OffsetDateTime,
    nonce: u8,
) {
    let mut key = [0_u8; 32];
    key[0] = nonce;
    key[1] = 43;
    ledger
        .append_batch(&EventBatch {
            source_id: "undated-fixture".to_owned(),
            events: vec![NormalizedEvent {
                event_id: uuid::Uuid::now_v7(),
                project_id: project,
                worktree_id: WorktreeId(uuid::Uuid::now_v7()),
                task_id: None,
                harness: Harness::ClaudeCode,
                native_session_id: "s1".to_owned(),
                native_turn_id: None,
                event_type: EventType::UserPrompted,
                occurred_at,
                observed_at,
                source_locator: "transcript.jsonl".to_owned(),
                source_offset: 0,
                source_schema: "undated:v1".to_owned(),
                raw_hash: key,
                idempotency_key: key,
                git_head: None,
                git_branch: None,
                payload: serde_json::json!({ "content": "a turn" }),
                raw: serde_json::json!({ "content": "a turn" }),
            }],
            quarantined: Vec::new(),
            capture_gaps: Vec::new(),
            next_cursor: SourceCursor::byte_offset(1),
        })
        .expect("append");
}
