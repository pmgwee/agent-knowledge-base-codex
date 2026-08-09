//! The bulk loader must return exactly what the per-memory path returned.
//!
//! `current_project_memories` was rewritten from roughly four queries *per memory version* to four
//! queries in total, because at 5,505 memories the old shape cost 14,282 ms inside a hook with a
//! 3 s budget — two of three registered projects timed out and delivered no orientation at all.
//!
//! A rewrite that is merely *faster* would be worthless here. This list is what
//! `resolve_candidates` closes over for supersession and contradiction, so a dropped evidence edge
//! or a resurrected version does not fail loudly — it produces a confident orientation that is
//! quietly wrong. So the test is an equivalence check against the old logic, rebuilt here from the
//! public API, over a fixture holding every case that can tell the two apart: a second version, a
//! retirement, an invalid status, and a withdrawal.

use brain_domain::{
    Authority, EventBatch, EventType, Harness, MemoryKind, MemoryRecord, MemoryScope, MemoryStatus,
    NormalizedEvent, ProjectId, SourceCursor, WorktreeId,
};
use brain_store::EventLedger;

fn event(project: ProjectId, worktree: WorktreeId, nonce: u8) -> NormalizedEvent {
    NormalizedEvent {
        // v7 ids minted inside one millisecond share their leading bytes, and `projection_path` is
        // derived from a prefix — so a fixture that does not vary them trips a UNIQUE constraint
        // that has nothing to do with what is being tested.
        event_id: uuid::Uuid::from_bytes({
            let mut bytes = *uuid::Uuid::now_v7().as_bytes();
            bytes[0] = nonce;
            bytes[1] = nonce.wrapping_mul(31);
            bytes
        }),
        project_id: project,
        worktree_id: worktree,
        task_id: None,
        harness: Harness::ClaudeCode,
        native_session_id: format!("session-{nonce}"),
        native_turn_id: None,
        event_type: EventType::AgentResponded,
        occurred_at: time::OffsetDateTime::UNIX_EPOCH,
        observed_at: time::OffsetDateTime::UNIX_EPOCH,
        source_locator: format!("fixture://{nonce}"),
        source_offset: i64::from(nonce),
        source_schema: "fixture".to_owned(),
        raw_hash: [nonce; 32],
        idempotency_key: [nonce; 32],
        git_head: None,
        git_branch: None,
        payload: serde_json::json!({ "content": format!("turn {nonce}") }),
        raw: serde_json::json!({ "content": format!("turn {nonce}") }),
    }
}

fn memory(
    project: ProjectId,
    worktree: WorktreeId,
    id: uuid::Uuid,
    title: &str,
    evidence: Vec<uuid::Uuid>,
    supersedes: Vec<uuid::Uuid>,
    status: MemoryStatus,
) -> MemoryRecord {
    MemoryRecord {
        id,
        version_id: uuid::Uuid::now_v7(),
        scope: MemoryScope::Project(project),
        worktree_id: Some(worktree),
        task_id: None,
        kind: MemoryKind::Decision,
        title: title.to_owned(),
        content: format!("body of {title}"),
        valid_from: time::OffsetDateTime::now_utc(),
        valid_to: None,
        recorded_at: time::OffsetDateTime::now_utc(),
        confidence: 0.9,
        authority: Authority::DerivedMemory,
        evidence_ids: evidence,
        supersedes,
        status,
    }
}

#[test]
fn the_bulk_loader_returns_exactly_what_the_per_memory_path_returned() {
    let project = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let mut ledger = EventLedger::open_in_memory(project).expect("open ledger");

    let events: Vec<_> = (1..=6).map(|n| event(project, worktree, n)).collect();
    ledger
        .append_batch(&EventBatch {
            source_id: "fixture".to_owned(),
            events: events.clone(),
            quarantined: Vec::new(),
            capture_gaps: Vec::new(),
            next_cursor: SourceCursor::byte_offset(64),
        })
        .expect("append events");
    let cite = |n: usize| events[n].event_id;

    // One version, one citation — the ordinary case.
    let plain = memory(
        project,
        worktree,
        uuid::Uuid::now_v7(),
        "plain claim",
        vec![cite(0)],
        Vec::new(),
        MemoryStatus::Current,
    );
    ledger.append_memory(&plain).expect("plain");

    // Two versions. The second must win, carrying *its* evidence and its supersession edge —
    // grouping edges by version is exactly where a bulk load can go wrong and still look right.
    let revised_id = uuid::Uuid::now_v7();
    let first = memory(
        project,
        worktree,
        revised_id,
        "revised claim",
        vec![cite(1)],
        Vec::new(),
        MemoryStatus::Current,
    );
    ledger.append_memory(&first).expect("revised v1");
    let second = memory(
        project,
        worktree,
        revised_id,
        "revised claim",
        vec![cite(1), cite(2)],
        vec![first.version_id],
        MemoryStatus::Current,
    );
    ledger.append_memory(&second).expect("revised v2");

    // Retired: the newest version is `Superseded`, so the memory must not appear at all. This is
    // the case a status-only filter gets wrong in the opposite direction.
    let retired_id = uuid::Uuid::now_v7();
    let retired_v1 = memory(
        project,
        worktree,
        retired_id,
        "retired claim",
        vec![cite(3)],
        Vec::new(),
        MemoryStatus::Current,
    );
    ledger.append_memory(&retired_v1).expect("retired v1");
    let retired_v2 = memory(
        project,
        worktree,
        retired_id,
        "retired claim",
        vec![cite(3)],
        vec![retired_v1.version_id],
        MemoryStatus::Superseded,
    );
    ledger.append_memory(&retired_v2).expect("retired v2");

    let invalid = memory(
        project,
        worktree,
        uuid::Uuid::now_v7(),
        "invalid claim",
        vec![cite(4)],
        Vec::new(),
        MemoryStatus::Invalid,
    );
    ledger.append_memory(&invalid).expect("invalid");

    let withdrawn = memory(
        project,
        worktree,
        uuid::Uuid::now_v7(),
        "withdrawn claim",
        vec![cite(5)],
        Vec::new(),
        MemoryStatus::Current,
    );
    ledger.append_memory(&withdrawn).expect("withdrawn");
    ledger
        .forget_memory(
            withdrawn.id,
            "test withdrawal",
            "operator",
            time::OffsetDateTime::now_utc(),
        )
        .expect("withdraw");

    // The oracle: the old logic, rebuilt from the public API — `current_memory` per id, the same
    // status filter, the same ordering. `current_memory` already returns `None` for a tombstoned
    // memory, which is how the old path excluded withdrawals.
    let ids = [plain.id, revised_id, retired_id, invalid.id, withdrawn.id];
    let mut expected: Vec<MemoryRecord> = ids
        .iter()
        .filter_map(|id| ledger.current_memory(*id).expect("current memory"))
        .filter(|memory| {
            !matches!(
                memory.status,
                MemoryStatus::Invalid | MemoryStatus::Superseded
            )
        })
        .collect();
    expected.sort_by_key(MemoryRecord::projection_path);

    let actual = ledger.current_project_memories().expect("bulk load");

    assert_eq!(
        expected.len(),
        actual.len(),
        "row count differs — expected {:?}, got {:?}",
        expected.iter().map(|m| &m.title).collect::<Vec<_>>(),
        actual.iter().map(|m| &m.title).collect::<Vec<_>>()
    );
    // `MemoryRecord` has no `PartialEq`, so compare the fields explicitly. That is not a
    // concession — it names what a bulk load can get wrong: the wrong version, or edges attached to
    // the wrong version. Everything else comes off the same row either way.
    for (left, right) in expected.iter().zip(actual.iter()) {
        assert_eq!(left.id, right.id, "memory id differs");
        assert_eq!(left.version_id, right.version_id, "version differs");
        assert_eq!(left.title, right.title, "title differs");
        assert_eq!(left.content, right.content, "content differs");
        assert_eq!(left.status, right.status, "status differs");
        assert_eq!(left.authority, right.authority, "authority differs");
        assert_eq!(left.worktree_id, right.worktree_id, "worktree differs");
        assert_eq!(left.evidence_ids, right.evidence_ids, "evidence differs");
        assert_eq!(left.supersedes, right.supersedes, "supersession differs");
    }

    let titles: Vec<_> = actual.iter().map(|m| m.title.as_str()).collect();
    assert!(titles.contains(&"plain claim"));
    assert!(titles.contains(&"revised claim"));
    assert!(
        !titles.contains(&"retired claim"),
        "a retired claim came back as current"
    );
    assert!(!titles.contains(&"invalid claim"));
    assert!(!titles.contains(&"withdrawn claim"));

    let revised = actual
        .iter()
        .find(|memory| memory.title == "revised claim")
        .expect("revised claim present");
    assert_eq!(
        revised.version_id, second.version_id,
        "the older version won"
    );
    assert_eq!(
        revised.evidence_ids.len(),
        2,
        "evidence was lost when edges were grouped"
    );
    assert_eq!(
        revised.supersedes,
        vec![first.version_id],
        "the supersession edge was lost when edges were grouped"
    );
}
