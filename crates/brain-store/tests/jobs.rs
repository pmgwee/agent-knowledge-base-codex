use brain_domain::{
    EventBatch, EventType, Harness, NormalizedEvent, ProjectId, SourceCursor, WorktreeId,
};
use brain_store::{ConsolidationReason, EventLedger, JobStatus};

#[test]
fn jobs_are_idempotent_leased_and_dead_lettered_after_five_failures() {
    let project = ProjectId(uuid::Uuid::now_v7());
    let mut ledger = EventLedger::open_in_memory(project).expect("open ledger");
    let first = append_event(&mut ledger, project, 1);
    let last = append_event(&mut ledger, project, 2);
    let first_job = ledger
        .enqueue_consolidation_job(first, last, ConsolidationReason::EventThreshold)
        .expect("enqueue job");
    let replayed = ledger
        .enqueue_consolidation_job(first, last, ConsolidationReason::EventThreshold)
        .expect("enqueue same range");
    assert_eq!(first_job.id, replayed.id);
    assert_eq!(ledger.consolidation_job_count().expect("count jobs"), 1);

    let mut now = first_job.available_at;
    for attempt in 1..=5 {
        let leased = ledger
            .lease_consolidation_job("worker-a", now, time::Duration::seconds(10))
            .expect("lease job")
            .expect("job available");
        assert_eq!(leased.project_id, project);
        assert_eq!(leased.attempt, attempt);
        ledger
            .fail_consolidation_job(leased.id, "worker-a", "fixture provider failure", now)
            .expect("fail job");
        now += time::Duration::seconds(1_i64 << attempt);
    }
    let job = ledger
        .consolidation_job(first_job.id)
        .expect("read job")
        .expect("job exists");
    assert_eq!(job.status, JobStatus::DeadLetter);
    assert!(
        ledger
            .lease_consolidation_job("worker-b", now, time::Duration::seconds(10))
            .expect("try dead job")
            .is_none()
    );
}

#[test]
fn threshold_and_inactivity_triggers_cover_only_unqueued_ranges() {
    let project = ProjectId(uuid::Uuid::now_v7());
    let mut ledger = EventLedger::open_in_memory(project).expect("open ledger");
    append_event(&mut ledger, project, 1);
    append_event(&mut ledger, project, 2);
    let threshold = ledger
        .enqueue_event_threshold_job(2)
        .expect("evaluate threshold")
        .expect("threshold job");
    assert_eq!(threshold.reason, ConsolidationReason::EventThreshold);
    assert!(
        ledger
            .enqueue_event_threshold_job(2)
            .expect("repeat threshold")
            .is_none()
    );

    append_event(&mut ledger, project, 3);
    let inactivity = ledger
        .enqueue_inactivity_job(
            time::OffsetDateTime::now_utc() + time::Duration::hours(1),
            time::Duration::minutes(30),
        )
        .expect("evaluate inactivity")
        .expect("inactivity job");
    assert_eq!(inactivity.reason, ConsolidationReason::Inactivity);
    assert_eq!(inactivity.first_event_id, inactivity.last_event_id);
}

fn append_event(ledger: &mut EventLedger, project: ProjectId, offset: i64) -> uuid::Uuid {
    let id = uuid::Uuid::now_v7();
    ledger
        .append_batch(&EventBatch {
            source_id: "fixture".to_owned(),
            events: vec![NormalizedEvent {
                event_id: id,
                project_id: project,
                worktree_id: WorktreeId(uuid::Uuid::now_v7()),
                task_id: None,
                harness: Harness::ClaudeCode,
                native_session_id: "session".to_owned(),
                native_turn_id: None,
                event_type: EventType::AgentResponded,
                occurred_at: time::OffsetDateTime::UNIX_EPOCH,
                observed_at: time::OffsetDateTime::UNIX_EPOCH,
                source_locator: "fixture".to_owned(),
                source_offset: offset,
                source_schema: "fixture".to_owned(),
                raw_hash: [u8::try_from(offset).expect("small offset"); 32],
                idempotency_key: [u8::try_from(offset).expect("small offset"); 32],
                git_head: None,
                git_branch: None,
                payload: serde_json::json!({"content": format!("event {offset}")}),
                raw: serde_json::json!({"content": format!("event {offset}")}),
            }],
            quarantined: Vec::new(),
            capture_gaps: Vec::new(),
            next_cursor: SourceCursor::byte_offset(u64::try_from(offset).expect("positive")),
        })
        .expect("append event");
    id
}
