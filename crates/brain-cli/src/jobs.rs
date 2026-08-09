//! The consolidation queue, and the one action a stuck queue needs.
//!
//! `brain digest` has reported a dead-letter count since it shipped, and nothing could act on it.
//! Three jobs sat dead for three days — one on a plain HTTP 429, two on a schema mismatch — because
//! the only way to retry one was to edit SQLite by hand. A number nobody can respond to is a number
//! nobody reads, so this is the response.
//!
//! The listing carries `last_error` rather than only a count, because that string is what separates
//! the two cases that matter: a job that died on quota will succeed on retry and a job that died on
//! a malformed proposal will not, and no count can tell them apart.

use anyhow::Result;
use brain_store::{EventLedger, JobStatus};

/// How many dead letters to show. A queue with more than this many is not a list to read, it is a
/// bug to fix — and the count above the list still tells the truth.
const MAX_LISTED: usize = 25;

#[derive(Clone, Debug, serde::Serialize)]
pub struct DeadJob {
    pub job_id: uuid::Uuid,
    pub reason: String,
    pub attempt: u32,
    /// Why it died. Kept verbatim, and kept across a retry: if the retry fails the same way, the
    /// pair of messages is the evidence that quota was never the problem.
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct JobReport {
    pub pending: u64,
    pub leased: u64,
    pub completed: u64,
    pub dead_letter: u64,
    /// How many jobs this invocation returned to the queue. Zero unless `--retry-dead` was passed.
    pub retried: u64,
    pub dead_jobs: Vec<DeadJob>,
}

pub fn report(ledger: &EventLedger, retried: u64) -> Result<JobReport> {
    let queue = ledger.consolidation_queue()?;
    let dead_jobs = ledger
        .consolidation_jobs_with_status(JobStatus::DeadLetter, MAX_LISTED)?
        .into_iter()
        .map(|job| DeadJob {
            job_id: job.id,
            // The enum's own name, not a lowercased `Debug` — that rendered `EventThreshold` as
            // `eventthreshold`, which matches neither the column nor the docs.
            reason: job.reason.as_str().to_owned(),
            attempt: job.attempt,
            last_error: job.last_error,
        })
        .collect();
    Ok(JobReport {
        pending: queue.pending,
        leased: queue.leased,
        completed: queue.completed,
        dead_letter: queue.dead_letter,
        retried,
        dead_jobs,
    })
}

pub fn render(report: &JobReport) -> String {
    let mut out = String::new();
    if report.retried > 0 {
        out.push_str(&format!(
            "  returned {} dead-lettered job(s) to the queue\n\n",
            report.retried
        ));
    }
    out.push_str(&format!(
        "  {} pending · {} leased · {} completed · {} dead\n",
        report.pending, report.leased, report.completed, report.dead_letter
    ));
    if report.dead_jobs.is_empty() {
        return out;
    }
    out.push('\n');
    for job in &report.dead_jobs {
        out.push_str(&format!(
            "  {}  {}  attempt {}\n",
            &job.job_id.to_string()[..8],
            job.reason,
            job.attempt
        ));
        if let Some(error) = &job.last_error {
            // One line, so a listing of twenty stays readable. The full string is in `--json`.
            let flat = error.replace('\n', " ");
            let shown: String = flat.chars().take(150).collect();
            out.push_str(&format!("      {shown}\n"));
        }
    }
    out.push_str("\n  Retry them with --retry-dead once the cause is addressed.\n");
    out
}
