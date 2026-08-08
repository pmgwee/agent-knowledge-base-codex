//! Why *that* result came back — per channel, for one query.
//!
//! The retrieval panel that shipped answers "what would a query do": which channels exist, their
//! weights, whether each can fire. That is configuration. It cannot tell you why a particular
//! document beat another one, which is the question you actually have when a result looks wrong.
//!
//! This runs each channel **separately**, reports the rank a document reached in each, and shows
//! the fused position beside them. The interesting rows are the disagreements: a document ranked 1
//! by vector and absent from keyword is exactly the case hybrid retrieval was built for, and a
//! document ranked 1 by keyword and absent from vector is the case the vector weight is allowed to
//! lose.
//!
//! Deliberately not an estimate. Every number here is a rank the ledger actually produced, not a
//! reconstruction of what the fusion arithmetic would have done.

use anyhow::Result;
use brain_domain::ProjectId;
use brain_store::{EventLedger, SearchQuery};

/// How one document fared, channel by channel.
#[derive(Debug, serde::Serialize)]
pub struct ExplainedHit {
    pub title: String,
    pub source: String,
    pub id: uuid::Uuid,
    /// Final position in the fused ranking, 1-based.
    pub fused_rank: usize,
    /// Position in the events keyword channel, when it appeared there at all.
    pub events_rank: Option<usize>,
    /// Position in the memories keyword channel.
    pub memories_rank: Option<usize>,
    pub rank_score: f64,
    /// A one-line reading of where this result came from.
    pub verdict: String,
}

#[derive(Debug, serde::Serialize)]
pub struct ExplainReport {
    pub project_id: ProjectId,
    pub query: String,
    /// Whether the vector channel could run at all — absent means no model installed.
    pub vector_available: bool,
    pub hits: Vec<ExplainedHit>,
    /// SQLite's own plan for the keyword statement, so an index regression is visible.
    pub query_plan: Vec<String>,
}

/// Explain one query against one project.
pub fn explain(
    ledger: &EventLedger,
    project_id: ProjectId,
    query_text: &str,
    limit: usize,
) -> Result<ExplainReport> {
    let base = SearchQuery::text(project_id, query_text).with_limit(limit);

    // Each keyword channel alone, so a rank in one can be compared against the fused position.
    let events: Vec<uuid::Uuid> = ledger
        .search(&base.clone().events_only())?
        .into_iter()
        .map(|hit| hit.source_id)
        .collect();
    let memories: Vec<uuid::Uuid> = ledger
        .search(&base.clone().memories_only())?
        .into_iter()
        .map(|hit| hit.source_id)
        .collect();
    let fused = ledger.search(&base)?;

    let position = |list: &[uuid::Uuid], id: uuid::Uuid| {
        list.iter()
            .position(|candidate| *candidate == id)
            .map(|i| i + 1)
    };

    let hits = fused
        .iter()
        .enumerate()
        .map(|(index, hit)| {
            let events_rank = position(&events, hit.source_id);
            let memories_rank = position(&memories, hit.source_id);
            let verdict = match (events_rank, memories_rank) {
                (None, None) => {
                    "found only by meaning or by evidence — no keyword channel matched it"
                        .to_owned()
                }
                (Some(rank), None) if rank > index + 1 => {
                    format!(
                        "keyword had it at {rank}; fusion moved it up to {}",
                        index + 1
                    )
                }
                (Some(rank), None) if rank < index + 1 => {
                    format!(
                        "keyword had it at {rank}; fusion moved it down to {}",
                        index + 1
                    )
                }
                (Some(_), None) => "keyword and fusion agree".to_owned(),
                (None, Some(rank)) => format!("a memory, ranked {rank} among memories"),
                (Some(events), Some(memories)) => {
                    format!("in both keyword channels — events {events}, memories {memories}")
                }
            };
            ExplainedHit {
                title: hit.title.clone(),
                source: format!("{:?}", hit.source).to_lowercase(),
                id: hit.memory_id.unwrap_or(hit.source_id),
                fused_rank: index + 1,
                events_rank,
                memories_rank,
                rank_score: hit.rank_score,
                verdict,
            }
        })
        .collect();

    Ok(ExplainReport {
        project_id,
        query: query_text.to_owned(),
        vector_available: ledger.vector_search_enabled(),
        hits,
        query_plan: ledger.explain_text_search(&base).unwrap_or_default(),
    })
}

pub fn render(report: &ExplainReport) -> String {
    let mut out = format!("\"{}\"\n", report.query);
    out.push_str(&format!(
        "  channels: BM25 events + BM25 memories + graph{}\n\n",
        if report.vector_available {
            " + vector"
        } else {
            " (no vector — no model installed)"
        }
    ));
    if report.hits.is_empty() {
        out.push_str("  nothing matched\n");
        return out;
    }
    for hit in &report.hits {
        out.push_str(&format!(
            "  {:>2}. [{}] {}\n      {}\n",
            hit.fused_rank,
            hit.source,
            truncate(&hit.title, 74),
            hit.verdict
        ));
    }
    out
}

fn truncate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_owned();
    }
    let head: String = text.chars().take(width.saturating_sub(1)).collect();
    format!("{head}…")
}
