//! Retrieval quality on LongMemEval-S, scored the way the field scores it.
//!
//! Every published memory-system comparison this brain would be measured against reports a
//! number from a different dataset — one vendor's LongMemEval result placed beside another's
//! LoCoMo result in the same column, none of them independently reproduced. Those numbers
//! cannot be compared to each other, so they cannot be compared to ours either.
//!
//! This runs the public benchmark (500 questions, ICLR 2025) against the ledger's own search
//! and reports **R@5, R@10 and MRR** — the same three figures, on the same data, computed here.
//! A published BM25-only baseline exists for it, which is the honest thing to hold a keyword
//! index against.
//!
//! Scoring is **session-level recall**, matching the dataset's `answer_session_ids`: a question
//! is answered correctly at K if any of the top K hits comes from an evidence session. Each
//! instance is ingested into its own ledger, because its haystack is its own world — mixing
//! them would let a hit from another question's sessions count, which is not retrieval, it is
//! leakage.

use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result, ensure};
use brain_domain::{
    EventBatch, EventType, Harness, NormalizedEvent, ProjectId, SourceCursor, WorktreeId,
};
use brain_store::{EventLedger, SearchQuery};
use sha2::{Digest, Sha256};

/// Ranks scored. K=5 and K=10 are what the published comparisons report.
const RECALL_AT: [usize; 2] = [5, 10];

/// Deepest rank fetched. Must be at least the largest `RECALL_AT` entry; more than that only
/// changes MRR, which is defined over the full returned ranking.
const SEARCH_DEPTH: usize = 10;

#[derive(Clone, Debug, serde::Deserialize)]
pub struct LongMemEvalInstance {
    pub question_id: String,
    pub question_type: String,
    pub question: String,
    pub haystack_session_ids: Vec<String>,
    pub haystack_sessions: Vec<Vec<Turn>>,
    pub answer_session_ids: Vec<String>,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct Turn {
    pub role: String,
    pub content: String,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct LongMemEvalReport {
    pub instances_scored: usize,
    pub sessions_ingested: usize,
    pub turns_ingested: usize,
    /// Recall at K, keyed by K. A question counts as recalled at K when any of the top K hits
    /// comes from one of its evidence sessions.
    pub recall_at: Vec<(usize, f64)>,
    pub mean_reciprocal_rank: f64,
    /// Per question type, so a single headline number cannot hide that one category carries it.
    pub recall_at_5_by_type: Vec<(String, f64)>,
    pub elapsed_seconds: f64,
}

/// Run the benchmark over the first `limit` instances, or all of them when `None`.
///
/// `limit` exists for iteration, not for reporting: a number from a subset is not the benchmark
/// and the report says how many instances it covers so a partial run can never be mistaken for
/// a complete one.
pub fn run_longmemeval(
    dataset: &Path,
    workspace: &Path,
    limit: Option<usize>,
) -> Result<LongMemEvalReport> {
    let started = std::time::Instant::now();
    let raw = std::fs::read(dataset)
        .with_context(|| format!("read LongMemEval dataset {}", dataset.display()))?;
    let mut instances: Vec<LongMemEvalInstance> =
        serde_json::from_slice(&raw).context("parse LongMemEval dataset")?;
    if let Some(limit) = limit {
        instances.truncate(limit);
    }
    ensure!(!instances.is_empty(), "dataset contains no instances");
    std::fs::create_dir_all(workspace)?;

    let mut sessions_ingested = 0;
    let mut turns_ingested = 0;
    let mut reciprocal_ranks = Vec::with_capacity(instances.len());
    let mut hits_at: Vec<usize> = vec![0; RECALL_AT.len()];
    let mut by_type: std::collections::BTreeMap<String, (usize, usize)> = Default::default();

    for (index, instance) in instances.iter().enumerate() {
        let project = ProjectId(uuid::Uuid::now_v7());
        let worktree = WorktreeId(uuid::Uuid::now_v7());
        let ledger_path = workspace.join(format!("instance-{index}.sqlite"));
        let _ = std::fs::remove_file(&ledger_path);
        let mut ledger = EventLedger::open(&ledger_path, project)?;

        let (sessions, turns) = ingest_instance(&mut ledger, project, worktree, instance)?;
        sessions_ingested += sessions;
        turns_ingested += turns;

        let mut query = SearchQuery::text(project, instance.question.clone());
        query.limit = SEARCH_DEPTH;
        let hits = ledger.search(&query)?;

        let evidence: HashSet<&str> = instance
            .answer_session_ids
            .iter()
            .map(String::as_str)
            .collect();
        // The rank of the first hit from an evidence session, 1-based. `None` when the ranking
        // never surfaces one, which contributes 0 to MRR rather than being dropped — silently
        // skipping misses is how a retrieval score flatters itself.
        let first_hit = hits.iter().position(|hit| {
            hit.native_session_id
                .as_deref()
                .is_some_and(|id| evidence.contains(id))
        });

        for (slot, k) in RECALL_AT.iter().enumerate() {
            if first_hit.is_some_and(|rank| rank < *k) {
                hits_at[slot] += 1;
            }
        }
        reciprocal_ranks.push(first_hit.map_or(0.0, |rank| 1.0 / (rank as f64 + 1.0)));

        let entry = by_type.entry(instance.question_type.clone()).or_default();
        entry.1 += 1;
        if first_hit.is_some_and(|rank| rank < 5) {
            entry.0 += 1;
        }

        drop(ledger);
        let _ = std::fs::remove_file(&ledger_path);
    }

    let scored = instances.len() as f64;
    Ok(LongMemEvalReport {
        instances_scored: instances.len(),
        sessions_ingested,
        turns_ingested,
        recall_at: RECALL_AT
            .iter()
            .enumerate()
            .map(|(slot, k)| (*k, hits_at[slot] as f64 / scored))
            .collect(),
        mean_reciprocal_rank: reciprocal_ranks.iter().sum::<f64>() / scored,
        recall_at_5_by_type: by_type
            .into_iter()
            .map(|(kind, (hit, total))| (kind, hit as f64 / total as f64))
            .collect(),
        elapsed_seconds: started.elapsed().as_secs_f64(),
    })
}

/// Ingest one instance's haystack, tagging every event with the session it came from.
///
/// `native_session_id` carries the dataset's session id because that is what the ledger returns
/// on a hit, and session-level recall is decided by comparing it against `answer_session_ids`.
fn ingest_instance(
    ledger: &mut EventLedger,
    project: ProjectId,
    worktree: WorktreeId,
    instance: &LongMemEvalInstance,
) -> Result<(usize, usize)> {
    ensure!(
        instance.haystack_session_ids.len() == instance.haystack_sessions.len(),
        "instance {} has {} session ids for {} sessions",
        instance.question_id,
        instance.haystack_session_ids.len(),
        instance.haystack_sessions.len()
    );

    let base = time::OffsetDateTime::UNIX_EPOCH;
    let mut offset: i64 = 0;
    let mut events = Vec::new();
    for (session_index, (session_id, turns)) in instance
        .haystack_session_ids
        .iter()
        .zip(&instance.haystack_sessions)
        .enumerate()
    {
        for turn in turns {
            offset += 1;
            // Sessions are ordered oldest first in this dataset, so a monotonic clock derived
            // from position preserves that ordering without inventing precision the data does
            // not carry.
            let occurred = base + time::Duration::seconds(offset);
            let mut digest = Sha256::new();
            digest.update(instance.question_id.as_bytes());
            digest.update(session_id.as_bytes());
            digest.update(offset.to_le_bytes());
            let fingerprint: [u8; 32] = digest.finalize().into();
            events.push(NormalizedEvent {
                event_id: uuid::Uuid::now_v7(),
                project_id: project,
                worktree_id: worktree,
                task_id: None,
                harness: Harness::ClaudeCode,
                native_session_id: session_id.clone(),
                native_turn_id: Some(format!("{session_index}:{offset}")),
                event_type: match turn.role.as_str() {
                    "user" => EventType::UserPrompted,
                    _ => EventType::AgentResponded,
                },
                occurred_at: occurred,
                observed_at: occurred,
                source_locator: format!("longmemeval/{}", instance.question_id),
                source_offset: offset,
                source_schema: "longmemeval:s".to_owned(),
                raw_hash: fingerprint,
                idempotency_key: fingerprint,
                git_head: None,
                git_branch: None,
                payload: serde_json::json!({ "content": turn.content }),
                raw: serde_json::json!({ "role": turn.role, "content": turn.content }),
            });
        }
    }

    let sessions = instance.haystack_sessions.len();
    let turns = events.len();
    // One batch per instance: the ledger's cursor is per source, and the whole haystack is one
    // source here.
    ledger.append_batch(&EventBatch {
        source_id: instance.question_id.clone(),
        events,
        quarantined: Vec::new(),
        capture_gaps: Vec::new(),
        next_cursor: SourceCursor::byte_offset(u64::try_from(offset)?),
    })?;
    Ok((sessions, turns))
}
