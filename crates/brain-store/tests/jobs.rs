use brain_domain::{
    EventBatch, EventType, Harness, NormalizedEvent, ProjectId, SourceCursor, WorktreeId,
};
use brain_store::{
    ConsolidationReason, EventLedger, JobStatus, MAX_JOB_EVENTS, MAX_JOB_PAYLOAD_BYTES,
};

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

#[test]
fn a_large_backlog_is_split_into_bounded_jobs_rather_than_one_unusable_one() {
    // The defect this guards, measured on the live brain: registration ingests a project's whole
    // transcript history in one burst, and the first threshold job then spanned 65,645 events —
    // ~2.9 GB of payload in a single evidence packet. A job is sent to a model whole, so an
    // unbounded range is not a large job, it is an impossible one. Draining the 223 jobs queued
    // that way was priced at roughly 1.09 billion input tokens.
    let project = ProjectId(uuid::Uuid::now_v7());
    let mut ledger = EventLedger::open_in_memory(project).expect("open ledger");
    let backlog = MAX_JOB_EVENTS * 2 + 50;
    for offset in 1..=backlog {
        append_event(&mut ledger, project, i64::try_from(offset).expect("fits"));
    }

    let first = ledger
        .enqueue_event_threshold_job(1)
        .expect("evaluate threshold")
        .expect("threshold job");
    let spanned = ledger
        .events_between(first.first_event_id, first.last_event_id)
        .expect("load job events");
    assert!(
        spanned.len() <= MAX_JOB_EVENTS,
        "a job must not span more than {MAX_JOB_EVENTS} events, got {}",
        spanned.len()
    );

    // The remainder is not dropped — it becomes the next job, so a backlog drains in bounded
    // pieces instead of being skipped.
    let second = ledger
        .enqueue_event_threshold_job(1)
        .expect("evaluate threshold again")
        .expect("second job");
    assert_ne!(first.id, second.id, "the backlog must continue, not repeat");
    let second_span = ledger
        .events_between(second.first_event_id, second.last_event_id)
        .expect("load second job events");
    assert!(second_span.len() <= MAX_JOB_EVENTS);
    assert_ne!(
        spanned.last().expect("first job non-empty").event_id,
        second_span.first().expect("second job non-empty").event_id,
        "the second job must start after the first ends, not overlap it"
    );
}

#[test]
fn a_window_of_oversized_events_is_bounded_by_bytes_not_only_count() {
    // Event sizes span three orders of magnitude — a session.compacted event was measured at
    // 1.3 MB against ~2 KB for a tool call. Bounding on count alone would still admit packets
    // far past any context window.
    let project = ProjectId(uuid::Uuid::now_v7());
    let mut ledger = EventLedger::open_in_memory(project).expect("open ledger");
    for offset in 1..=12 {
        append_large_event(&mut ledger, project, offset, 80_000);
    }

    let job = ledger
        .enqueue_event_threshold_job(1)
        .expect("evaluate threshold")
        .expect("threshold job");
    let spanned = ledger
        .events_between(job.first_event_id, job.last_event_id)
        .expect("load job events");
    let bytes: usize = spanned
        .iter()
        .map(|event| event.payload.to_string().len())
        .sum();
    assert!(
        spanned.len() < 12,
        "byte pressure must end the window early, got all {} events",
        spanned.len()
    );
    assert!(
        bytes <= MAX_JOB_PAYLOAD_BYTES * 2,
        "packet payload {bytes} is far past the {MAX_JOB_PAYLOAD_BYTES} byte bound"
    );
}

#[test]
fn enqueueing_against_a_large_backlog_does_not_rescan_the_whole_ledger() {
    // The defect this guards cost an evening. The window that bounds a job was computed with a
    // running SUM(LENGTH(...)) OVER (ORDER BY rowid) across *every* uncovered event, so each
    // call re-read the entire backlog — on the live brain, tens of thousands of rows and
    // hundreds of megabytes of payload text, once every two seconds per project.
    //
    // Nothing failed loudly. The service pegged a core, the async runtime was starved, and the
    // provider request on it timed out at exactly 60 s against a connection that had opened in
    // two. It looked like a network or model problem for hours; it was a query plan.
    //
    // So the guard is cost, not correctness: work must stay bounded by the window, not by the
    // size of the backlog behind it. The margin is deliberately loose — this catches a return
    // to full scans, not a modest slowdown.
    let project = ProjectId(uuid::Uuid::now_v7());
    let mut ledger = EventLedger::open_in_memory(project).expect("open ledger");
    for offset in 1..=2_000 {
        append_large_event(&mut ledger, project, offset, 2_000);
    }

    let started = std::time::Instant::now();
    let mut enqueued = 0;
    while ledger
        .enqueue_event_threshold_job(1)
        .expect("evaluate threshold")
        .is_some()
    {
        enqueued += 1;
        assert!(enqueued <= 2_000, "enqueue must terminate, not loop");
    }
    let elapsed = started.elapsed();

    assert!(enqueued > 1, "a 2,000 event backlog must need several jobs");
    assert!(
        elapsed < std::time::Duration::from_secs(20),
        "draining a 2,000 event backlog took {elapsed:?}; the window is rescanning the backlog"
    );
}

fn append_large_event(
    ledger: &mut EventLedger,
    project: ProjectId,
    offset: i64,
    payload_bytes: usize,
) -> uuid::Uuid {
    let id = uuid::Uuid::now_v7();
    let filler = "x".repeat(payload_bytes);
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
                raw_hash: [u8::try_from(offset % 251).expect("small"); 32],
                idempotency_key: [u8::try_from(offset % 251).expect("small"); 32],
                git_head: None,
                git_branch: None,
                payload: serde_json::json!({ "content": filler }),
                raw: serde_json::json!({ "content": "raw" }),
            }],
            quarantined: Vec::new(),
            capture_gaps: Vec::new(),
            next_cursor: SourceCursor::byte_offset(u64::try_from(offset).expect("positive")),
        })
        .expect("append large event");
    id
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
                raw_hash: [u8::try_from(offset % 251).expect("small offset"); 32],
                idempotency_key: [u8::try_from(offset % 251).expect("small offset"); 32],
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
