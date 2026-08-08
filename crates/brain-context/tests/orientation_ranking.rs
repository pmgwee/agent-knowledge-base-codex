//! Which memories reach an agent when there is only room for a few.
//!
//! The orientation has space for two or three memories against thousands in the ledger, so the
//! ordering *is* the selection. It used to be alphabetical by subject — a real ordering, just not
//! one connected to anything the session was about.

use brain_context::{ContextCompiler, ContextEvidence, ContextQuery};
use brain_domain::{
    Authority, EventType, Harness, MemoryKind, MemoryRecord, MemoryScope, MemoryStatus, ProjectId,
    WorktreeId,
};

#[test]
fn a_ranking_decides_which_memories_reach_the_agent() {
    // "Aardvark" wins on the alphabet and loses on relevance. Whichever appears tells you which
    // rule is in force.
    let project = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let alphabetically_first = memory(
        project,
        worktree,
        "Aardvark trivia",
        "unrelated to anything",
    );
    let actually_relevant = memory(
        project,
        worktree,
        "Zebra deployment",
        "the work in progress",
    );

    let compiler =
        ContextCompiler::from_events(vec![event(project, worktree)]).with_memories(vec![
            alphabetically_first.clone(),
            actually_relevant.clone(),
        ]);

    // Without a ranking, the alphabet decides — the behaviour this replaces.
    let unranked = compiler
        .compile(query(project, worktree))
        .expect("compile unranked");
    let unranked_first = first_memory_title(&unranked.text);
    assert_eq!(
        unranked_first.as_deref(),
        Some("Aardvark trivia"),
        "with no ranking the alphabetical order stands, got {unranked_first:?}"
    );

    // With one, relevance decides.
    let ranked = ContextCompiler::from_events(vec![event(project, worktree)])
        .with_memories(vec![alphabetically_first, actually_relevant.clone()])
        .with_memory_ranking(vec![actually_relevant.id])
        .compile(query(project, worktree))
        .expect("compile ranked");
    assert_eq!(
        first_memory_title(&ranked.text).as_deref(),
        Some("Zebra deployment"),
        "a ranked memory must outrank one that only sorts earlier"
    );
}

#[test]
fn an_unranked_memory_is_demoted_rather_than_dropped() {
    // A ranking is a preference, not a filter. A memory the ranker never saw is not thereby
    // irrelevant, and dropping it would make retrieval failure look like an empty brain.
    let project = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let ranked = memory(project, worktree, "Ranked subject", "in the ranking");
    let unranked = memory(project, worktree, "Absent subject", "not in the ranking");

    let compiled = ContextCompiler::from_events(vec![event(project, worktree)])
        .with_memories(vec![ranked.clone(), unranked.clone()])
        .with_memory_ranking(vec![ranked.id])
        .compile(query(project, worktree))
        .expect("compile");

    assert!(compiled.text.contains("Ranked subject"));
    assert!(
        compiled.text.contains("Absent subject"),
        "an unranked memory still appears, just later"
    );
}

// --- fixtures ---

fn first_memory_title(text: &str) -> Option<String> {
    for candidate in ["Aardvark trivia", "Zebra deployment"] {
        if let Some(index) = text.find(candidate) {
            let other = ["Aardvark trivia", "Zebra deployment"]
                .into_iter()
                .find(|entry| *entry != candidate)
                .and_then(|entry| text.find(entry));
            if other.is_none_or(|other| index < other) {
                return Some(candidate.to_owned());
            }
        }
    }
    None
}

fn query(project: ProjectId, worktree: WorktreeId) -> ContextQuery {
    ContextQuery::for_worktree(project, worktree)
}

fn event(project: ProjectId, worktree: WorktreeId) -> ContextEvidence {
    ContextEvidence {
        event_id: uuid::Uuid::now_v7(),
        project_id: project,
        worktree_id: worktree,
        task_id: None,
        harness: Harness::ClaudeCode,
        native_session_id: "ranking-session".to_owned(),
        event_type: EventType::UserPrompted,
        occurred_at: time::OffsetDateTime::UNIX_EPOCH,
        observed_at: time::OffsetDateTime::UNIX_EPOCH,
        source_offset: 1,
        git_head: None,
        git_branch: Some("main".to_owned()),
        payload: serde_json::json!({ "content": "working on the deployment" }),
        raw: serde_json::json!({ "content": "working on the deployment" }),
    }
}

fn memory(project: ProjectId, worktree: WorktreeId, title: &str, content: &str) -> MemoryRecord {
    MemoryRecord {
        id: uuid::Uuid::now_v7(),
        version_id: uuid::Uuid::now_v7(),
        scope: MemoryScope::Project(project),
        worktree_id: Some(worktree),
        task_id: None,
        kind: MemoryKind::Decision,
        title: title.to_owned(),
        content: content.to_owned(),
        valid_from: time::OffsetDateTime::UNIX_EPOCH,
        valid_to: None,
        recorded_at: time::OffsetDateTime::UNIX_EPOCH,
        confidence: 1.0,
        authority: Authority::DerivedMemory,
        evidence_ids: vec![uuid::Uuid::now_v7()],
        supersedes: Vec::new(),
        status: MemoryStatus::Current,
    }
}
