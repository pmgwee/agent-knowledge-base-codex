use anyhow::Result;
use brain_domain::MemoryStatus;
use brain_store::{EventLedger, SearchHit};

use crate::{RetrievalQuery, authority_rank};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScoreComponents {
    pub exact_task: f64,
    pub session_continuity: f64,
    pub path_match: f64,
    pub worktree_match: f64,
    pub authority: f64,
    pub current_validity: f64,
    pub recency: f64,
    pub bm25: f64,
    pub evidence_completeness: f64,
}

impl ScoreComponents {
    pub fn total(&self) -> f64 {
        self.exact_task
            + self.session_continuity
            + self.path_match
            + self.worktree_match
            + self.authority
            + self.current_validity
            + self.recency
            + self.bm25
            + self.evidence_completeness
    }
}

#[derive(Clone, Debug)]
pub struct RankedCandidate {
    pub hit: SearchHit,
    pub score: ScoreComponents,
    pub reasons: Vec<&'static str>,
}

impl RankedCandidate {
    pub fn total_score(&self) -> f64 {
        self.score.total()
    }
}

pub struct RetrievalEngine;

impl RetrievalEngine {
    pub fn retrieve(ledger: &EventLedger, query: &RetrievalQuery) -> Result<Vec<RankedCandidate>> {
        let mut candidates = ledger
            .search(&query.store_query())?
            .into_iter()
            .map(|hit| rank(hit, query))
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| {
            right
                .total_score()
                .total_cmp(&left.total_score())
                .then_with(|| right.hit.occurred_at.cmp(&left.hit.occurred_at))
                .then_with(|| right.hit.source_id.cmp(&left.hit.source_id))
        });
        candidates.truncate(query.limit);
        Ok(candidates)
    }
}

fn rank(hit: SearchHit, query: &RetrievalQuery) -> RankedCandidate {
    let mut score = ScoreComponents::default();
    let mut reasons = Vec::new();
    if query.task_id.is_some() && hit.task_id == query.task_id {
        score.exact_task = 5.0;
        reasons.push("exact_task");
    }
    if query.native_session_id.as_deref().is_some()
        && hit.native_session_id.as_deref() == query.native_session_id.as_deref()
    {
        score.session_continuity = 4.0;
        reasons.push("session_continuity");
    }
    if query.worktree_id.is_some() && hit.worktree_id == query.worktree_id {
        score.worktree_match = 3.0;
        reasons.push("worktree_match");
    }
    if path_matches(hit.path.as_deref(), &query.paths) {
        score.path_match = 3.0;
        reasons.push("path_match");
    }
    if let Some(authority) = &hit.authority {
        score.authority = f64::from(authority_rank(authority)) * 0.35;
        reasons.push("authority");
    }
    score.current_validity = match hit.status {
        Some(MemoryStatus::Current) | None => 1.5,
        Some(MemoryStatus::Conflict) => 0.75,
        Some(MemoryStatus::Proposed) => 0.25,
        Some(MemoryStatus::Superseded | MemoryStatus::Invalid) => 0.0,
    };
    if score.current_validity > 0.0 {
        reasons.push("current_validity");
    }
    let age_seconds = (query.now - hit.occurred_at).whole_seconds().max(0) as f64;
    let age_days = age_seconds / 86_400.0;
    score.recency = 2.0 / (1.0 + age_days / 30.0);
    reasons.push("recency");
    score.bm25 = (hit.bm25_score.max(0.0) * 1_000_000.0).ln_1p().min(3.0);
    if score.bm25 > 0.0 {
        reasons.push("bm25");
    }
    if hit.evidence_count > 0 {
        score.evidence_completeness = 1.0;
        reasons.push("evidence_complete");
    }
    if hit.late_observation {
        reasons.push("late_observation");
    }
    RankedCandidate {
        hit,
        score,
        reasons,
    }
}

fn path_matches(hit_path: Option<&str>, query_paths: &[String]) -> bool {
    let Some(hit_path) = hit_path else {
        return false;
    };
    let hit_path = normalize_path(hit_path);
    query_paths.iter().any(|path| {
        let path = normalize_path(path);
        hit_path == path || hit_path.ends_with(&path) || path.ends_with(&hit_path)
    })
}

fn normalize_path(path: &str) -> String {
    path.replace('\\', "/")
        .trim_start_matches("./")
        .to_lowercase()
}
