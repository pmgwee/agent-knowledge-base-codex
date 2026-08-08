//! Health-checking the vault.
//!
//! Every rule here is derived, which is the constraint that shapes all of them: a lint that asked a
//! model "does this look wrong" would produce findings with no evidence behind them, in a vault
//! whose whole property is that nothing in it is unsourced.

use brain_cli::lint_project;
use brain_domain::{
    Authority, EventBatch, EventType, Harness, MemoryKind, MemoryRecord, MemoryScope, MemoryStatus,
    NormalizedEvent, ProjectId, SourceCursor, WorktreeId,
};
use brain_store::EventLedger;

#[test]
fn a_healthy_project_reports_nothing_actionable() {
    // The rule that keeps the rest honest. A lint that always finds something is one nobody reads.
    let (mut ledger, project, worktree) = fixture();
    let shared = append_event(&mut ledger, project, worktree, 1);
    for title in ["First claim", "Second claim"] {
        let mut record = memory(project, worktree, title, vec![shared]);
        record.valid_from = now();
        ledger.append_memory(&record).expect("append");
    }

    let report = lint_project(&ledger, project, now()).expect("lint");
    assert!(
        !report.actionable,
        "a clean project must not demand a decision, got {:?}",
        report.findings.iter().map(|f| f.rule).collect::<Vec<_>>()
    );
    assert_eq!(report.memories_checked, 2);
}

#[test]
fn a_memory_dated_at_the_epoch_is_misdated_not_merely_old() {
    // Found by running this against the live brain: four memories carried valid_from of
    // 1970-01-01. That is a default, not a date — they are undated, and they sort to the front of
    // every chronological view, which is the opposite of what an unset field should do.
    let (mut ledger, project, worktree) = fixture();
    let event = append_event(&mut ledger, project, worktree, 1);
    let mut undated = memory(project, worktree, "No date supplied", vec![event]);
    undated.valid_from = time::OffsetDateTime::UNIX_EPOCH;
    ledger.append_memory(&undated).expect("append");

    let report = lint_project(&ledger, project, now()).expect("lint");
    let misdated = report
        .findings
        .iter()
        .find(|finding| finding.rule == "misdated")
        .expect("an epoch date must be reported as misdated");
    assert_eq!(misdated.count, 1);
    assert!(misdated.actionable, "an undated claim needs a decision");
    assert!(
        !report.findings.iter().any(|f| f.rule == "unrefreshed"),
        "an undated memory must not also be counted as merely old"
    );
}

#[test]
fn contradicting_memories_are_reported_and_need_a_decision() {
    // Two current memories about one subject, at equal authority, saying different things. The
    // resolver already detected these; nothing surfaced them outside an orientation, where they
    // competed for the same scarce space as everything else.
    let (mut ledger, project, worktree) = fixture();
    let event = append_event(&mut ledger, project, worktree, 1);

    for (index, content) in ["We keep the bin directory", "We exclude the bin directory"]
        .into_iter()
        .enumerate()
    {
        let mut record = memory(project, worktree, "Backup contents", vec![event]);
        // Explicit ids, far apart. `projection_file_name` disambiguates with the first eight hex
        // characters of the id, and in a v7 UUID those are timestamp bits — so two memories with
        // the same title minted in the same millisecond produce the same filename and collide on
        // the `projection_path` unique index. Narrow in production, since it needs an identical
        // title too, and it fails loudly rather than overwriting. Worth knowing all the same.
        record.id = uuid::Uuid::from_bytes([index as u8 + 1; 16]);
        record.content = content.to_owned();
        record.valid_from = now();
        ledger.append_memory(&record).expect("append");
    }

    let report = lint_project(&ledger, project, now()).expect("lint");
    let contradiction = report
        .findings
        .iter()
        .find(|finding| finding.rule == "contradiction")
        .expect("disagreeing memories must be reported");
    assert!(contradiction.actionable);
    assert!(report.actionable);
}

#[test]
fn an_island_is_counted_but_does_not_demand_a_decision() {
    // An orphan is honest here: a claim really can share evidence with nothing else. The useful
    // signal is the proportion over time, not the individual note.
    let (mut ledger, project, worktree) = fixture();
    let lonely = append_event(&mut ledger, project, worktree, 1);
    let mut record = memory(project, worktree, "An isolated claim", vec![lonely]);
    record.valid_from = now();
    ledger.append_memory(&record).expect("append");

    let report = lint_project(&ledger, project, now()).expect("lint");
    let unlinked = report
        .findings
        .iter()
        .find(|finding| finding.rule == "unlinked")
        .expect("an island must be counted");
    assert_eq!(unlinked.count, 1);
    assert!(
        !unlinked.actionable,
        "an island is an observation, not a defect"
    );
    assert!(!report.actionable);
}

// --- fixtures ---

fn now() -> time::OffsetDateTime {
    time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(20_000)
}

fn fixture() -> (EventLedger, ProjectId, WorktreeId) {
    let project = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let ledger = EventLedger::open_in_memory(project).expect("open ledger");
    (ledger, project, worktree)
}

fn append_event(
    ledger: &mut EventLedger,
    project: ProjectId,
    worktree: WorktreeId,
    offset: i64,
) -> uuid::Uuid {
    let event_id = uuid::Uuid::now_v7();
    let mut key = [0_u8; 32];
    key[0] = offset as u8;
    ledger
        .append_batch(&EventBatch {
            source_id: "lint-fixture".to_owned(),
            events: vec![NormalizedEvent {
                event_id,
                project_id: project,
                worktree_id: worktree,
                task_id: None,
                harness: Harness::ClaudeCode,
                native_session_id: "lint-session".to_owned(),
                native_turn_id: None,
                event_type: EventType::UserPrompted,
                occurred_at: now(),
                observed_at: now(),
                source_locator: "transcript.jsonl".to_owned(),
                source_offset: offset,
                source_schema: "lint:v1".to_owned(),
                raw_hash: key,
                idempotency_key: key,
                git_head: None,
                git_branch: None,
                payload: serde_json::json!({ "content": "a turn" }),
                raw: serde_json::json!({ "content": "a turn" }),
            }],
            quarantined: Vec::new(),
            capture_gaps: Vec::new(),
            next_cursor: SourceCursor::byte_offset(offset as u64),
        })
        .expect("append event");
    event_id
}

fn memory(
    project: ProjectId,
    worktree: WorktreeId,
    title: &str,
    evidence_ids: Vec<uuid::Uuid>,
) -> MemoryRecord {
    MemoryRecord {
        id: uuid::Uuid::now_v7(),
        version_id: uuid::Uuid::now_v7(),
        scope: MemoryScope::Project(project),
        worktree_id: Some(worktree),
        task_id: None,
        kind: MemoryKind::Decision,
        title: title.to_owned(),
        content: format!("{title}."),
        valid_from: now(),
        valid_to: None,
        recorded_at: now(),
        confidence: 1.0,
        authority: Authority::DerivedMemory,
        evidence_ids,
        supersedes: Vec::new(),
        status: MemoryStatus::Current,
    }
}
