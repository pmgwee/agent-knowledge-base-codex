//! Filing a conclusion back, and the rule that keeps it checkable.

use brain_cli::{RememberRequest, remember};
use brain_domain::{
    Authority, EventBatch, EventType, Harness, MemoryKind, MemoryRecord, MemoryScope, MemoryStatus,
    NormalizedEvent, ProjectId, SourceCursor, WorktreeId,
};
use brain_store::EventLedger;

#[test]
fn a_filed_conclusion_is_as_checkable_as_a_distilled_one() {
    let (mut ledger, project, worktree) = fixture();
    let evidence = append_event(&mut ledger, project, worktree, 1);

    let filed = remember(
        &mut ledger,
        RememberRequest {
            project_id: project,
            worktree_id: worktree,
            kind: MemoryKind::Decision,
            title: "  Use jose, not jsonwebtoken  ",
            content: "  Edge has no node crypto.  ",
            evidence_ids: vec![evidence],
            supersedes: Vec::new(),
            now: now(),
        },
    )
    .expect("file a conclusion");

    assert_eq!(filed.title, "Use jose, not jsonwebtoken", "trimmed");
    let stored = ledger
        .current_memory(filed.memory_id)
        .expect("lookup")
        .expect("the filed memory is current");
    assert_eq!(stored.evidence_ids, vec![evidence]);
    assert_eq!(
        stored.authority,
        Authority::HumanCorrection,
        "deliberately filed outranks incidentally distilled"
    );
}

#[test]
fn an_uncited_claim_is_refused() {
    // Without this, `remember` would be the one way to get an unsourced claim into a vault that
    // has none, which would quietly cost the property everything else here is built to protect.
    let (mut ledger, project, worktree) = fixture();
    let error = remember(
        &mut ledger,
        RememberRequest {
            project_id: project,
            worktree_id: worktree,
            kind: MemoryKind::Fact,
            title: "Something I believe",
            content: "But cannot point at.",
            evidence_ids: Vec::new(),
            supersedes: Vec::new(),
            now: now(),
        },
    )
    .expect_err("an uncited claim must be refused");
    assert!(format!("{error}").contains("uncited"), "{error}");
}

#[test]
fn evidence_the_ledger_does_not_hold_is_refused_by_the_ledger_itself() {
    // The same guard that protects consolidation protects this: a memory citing an event the
    // ledger does not have is never stored, whoever filed it.
    let (mut ledger, project, worktree) = fixture();
    let error = remember(
        &mut ledger,
        RememberRequest {
            project_id: project,
            worktree_id: worktree,
            kind: MemoryKind::Fact,
            title: "Invented evidence",
            content: "Citing a turn that never happened.",
            evidence_ids: vec![uuid::Uuid::now_v7()],
            supersedes: Vec::new(),
            now: now(),
        },
    )
    .expect_err("invented evidence must be refused");
    assert!(
        format!("{error:#}").contains("does not belong"),
        "{error:#}"
    );
}

#[test]
fn filing_a_correction_supersedes_without_deleting() {
    // Filing an answer is also how the brain gets corrected: the new claim wins on the subject and
    // the old one becomes history rather than disappearing.
    let (mut ledger, project, worktree) = fixture();
    let evidence = append_event(&mut ledger, project, worktree, 1);
    let original = memory(project, worktree, "Auth library", vec![evidence]);
    ledger.append_memory(&original).expect("append original");

    let filed = remember(
        &mut ledger,
        RememberRequest {
            project_id: project,
            worktree_id: worktree,
            kind: MemoryKind::Decision,
            title: "Auth library, corrected",
            content: "We moved to jose.",
            evidence_ids: vec![evidence],
            supersedes: vec![original.id],
            now: now(),
        },
    )
    .expect("file a correction");

    assert_eq!(filed.supersedes, vec![original.version_id]);
    // The original still exists; its version is simply no longer the current answer.
    assert!(
        ledger
            .memory_versions(original.id)
            .expect("versions")
            .iter()
            .any(|version| version.version_id == original.version_id),
        "superseding must not delete the claim it replaced"
    );
}

#[test]
fn superseding_something_absent_fails_before_anything_is_written() {
    let (mut ledger, project, worktree) = fixture();
    let evidence = append_event(&mut ledger, project, worktree, 1);

    let error = remember(
        &mut ledger,
        RememberRequest {
            project_id: project,
            worktree_id: worktree,
            kind: MemoryKind::Fact,
            title: "Replaces nothing that exists",
            content: "The id is wrong.",
            evidence_ids: vec![evidence],
            supersedes: vec![uuid::Uuid::now_v7()],
            now: now(),
        },
    )
    .expect_err("superseding an absent memory must fail");
    assert!(format!("{error}").contains("to supersede"), "{error}");
    assert!(
        ledger.current_project_memories().expect("list").is_empty(),
        "a failed correction must leave no memory beside the one it meant to replace"
    );
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
            source_id: "remember-fixture".to_owned(),
            events: vec![NormalizedEvent {
                event_id,
                project_id: project,
                worktree_id: worktree,
                task_id: None,
                harness: Harness::ClaudeCode,
                native_session_id: "remember-session".to_owned(),
                native_turn_id: None,
                event_type: EventType::UserPrompted,
                occurred_at: now(),
                observed_at: now(),
                source_locator: "transcript.jsonl".to_owned(),
                source_offset: offset,
                source_schema: "remember:v1".to_owned(),
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
