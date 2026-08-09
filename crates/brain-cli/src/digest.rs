//! A periodic health reading of the brain, derived and written to the vault.
//!
//! The competitor pattern this answers has four scheduled agents — morning brief, nightly
//! consolidation, weekly review, vault-health check — and the claim attached to them is that the
//! knowledge base "maintains itself". Ours consolidates continuously already, so the missing half
//! was never the *schedule*: it was that nothing ever stepped back and asked whether what had been
//! built was still coherent.
//!
//! **The LLM half stays out.** A written review needs a provider, and that provider is rate-limited
//! today — but the review a maintainer actually acts on is arithmetic: how many contradictions, how
//! much of the corpus nothing ever retrieves, how far the queue is behind, how retention is
//! distributed. All of that is derived, so all of it works with the provider down.
//!
//! It writes into the vault rather than only to stdout, because a digest nobody sees is a cron job
//! with extra steps.

use anyhow::Result;
use brain_domain::ProjectId;
use brain_store::EventLedger;

/// A single health reading.
#[derive(Debug, serde::Serialize)]
pub struct Digest {
    pub project_id: ProjectId,
    pub generated_at: time::OffsetDateTime,
    pub events: u64,
    pub memories: u64,
    /// Memories retrieval has never reached.
    pub never_retrieved: u64,
    /// Memories whose retention has fallen below the stale line.
    pub stale: u64,
    /// Mean retention across the corpus — the single number that moves when the brain is being used.
    pub mean_retention: f64,
    pub contradictions: usize,
    pub undecidable_contradictions: usize,
    pub jobs_pending: u64,
    pub jobs_dead: u64,
    /// Sessions the brain pushed context into, mid-session.
    pub subjects: usize,
    /// Minutes since the newest captured event. `None` when nothing has been captured at all.
    ///
    /// The instrument this system was missing. Capture stalled for 100 minutes on 9 August with the
    /// service running, `capture_blocked: false`, and every panel green — because nothing anywhere
    /// compared the newest event against the wall clock. A stalled cursor and a quiet afternoon
    /// look identical unless you ask how old the newest thing is.
    pub capture_lag_minutes: Option<f64>,
    /// What changed since the previous digest, when there was one.
    pub notes: Vec<String>,
}

/// How stale capture may get before the digest says so.
///
/// The service rescans transcript roots every 120 s, so anything past a few minutes is either a
/// genuinely idle machine or a stall. Thirty minutes is quiet on a normal working day and loud on
/// the failure that actually happened — and the note states both readings rather than picking one,
/// because the digest cannot know whether you were at the keyboard.
const CAPTURE_STALE_AFTER_MINUTES: f64 = 30.0;

pub fn build(
    ledger: &EventLedger,
    project_id: ProjectId,
    now: time::OffsetDateTime,
) -> Result<Digest> {
    let retention = ledger.memory_retention(now).unwrap_or_default();
    let memories = retention.len() as u64;
    let stale = retention.iter().filter(|entry| entry.is_stale()).count() as u64;
    let never = retention
        .iter()
        .filter(|entry| entry.retrieved_count == 0)
        .count() as u64;
    let mean = if retention.is_empty() {
        0.0
    } else {
        retention.iter().map(|entry| entry.retention).sum::<f64>() / retention.len() as f64
    };
    let queue = ledger.consolidation_queue()?;
    let reconcile = crate::propose_reconciliation(ledger, project_id)?;
    let capture_lag_minutes = ledger
        .latest_event_at()
        .ok()
        .flatten()
        .map(|latest| ((now - latest).as_seconds_f64() / 60.0).max(0.0));

    let mut notes = Vec::new();
    // Each note is a reading, not an instruction. The digest states what is true and stops; what to
    // do about it is a judgement, and one this cannot make without inventing a priority.
    if memories > 0 && never * 2 > memories {
        notes.push(format!(
            "Retrieval has never reached {never} of {memories} memories ({:.0}%). A brain that is \
             mostly unread is storing rather than remembering — though a young access counter looks \
             identical to a neglected corpus, so read this against how long counting has run.",
            never as f64 / memories as f64 * 100.0
        ));
    }
    if reconcile.contradictions > 0 {
        notes.push(format!(
            "{} contradiction(s), {} of which derivation cannot separate. `brain reconcile` shows \
             every side.",
            reconcile.contradictions, reconcile.undecidable
        ));
    }
    // Capture staleness leads the notes when it fires, because every number below it is computed
    // from a corpus that stopped growing — they are stale in a way they cannot report themselves.
    if let Some(lag) = capture_lag_minutes
        && lag > CAPTURE_STALE_AFTER_MINUTES
    {
        notes.insert(
            0,
            format!(
                "Nothing captured for {lag:.0} minutes. If a session is open, capture has stalled \
                 and every figure here is computed from a corpus that stopped growing. Restarting \
                 AgentBrain.Service clears it — capture resumes losslessly from its cursors."
            ),
        );
    }
    if queue.dead_letter > 0 {
        notes.push(format!(
            "{} job(s) dead-lettered. These do not retry on their own.",
            queue.dead_letter
        ));
    }
    if queue.pending > 500 {
        notes.push(format!(
            "{} jobs pending. If provider deferrals are HTTP 429 this is quota rather than a fault, \
             and it drains itself.",
            queue.pending
        ));
    }
    if notes.is_empty() {
        notes.push("Nothing needs a decision.".to_owned());
    }

    Ok(Digest {
        project_id,
        generated_at: now,
        events: ledger.event_count()?,
        memories,
        never_retrieved: never,
        stale,
        mean_retention: mean,
        contradictions: reconcile.contradictions,
        undecidable_contradictions: reconcile.undecidable,
        jobs_pending: queue.pending,
        jobs_dead: queue.dead_letter,
        subjects: ledger.subject_synthesis_count().unwrap_or(0) as usize,
        capture_lag_minutes,
        notes,
    })
}

/// Render as Markdown, shaped for the vault rather than for a terminal.
pub fn render_markdown(digest: &Digest) -> String {
    let date = digest.generated_at.date();
    let mut out = format!(
        "## [{date}] health digest\n\n\
         | | |\n|---|---|\n\
         | Events | {events} |\n\
         | Memories | {memories} |\n\
         | Never retrieved | {never} ({never_pct:.0}%) |\n\
         | Stale | {stale} |\n\
         | Mean retention | {retention:.3} |\n\
         | Contradictions | {contradictions} ({undecidable} undecidable) |\n\
         | Jobs pending / dead | {pending} / {dead} |\n\
         | Capture lag | {lag} |\n\n",
        events = digest.events,
        memories = digest.memories,
        never = digest.never_retrieved,
        never_pct = if digest.memories > 0 {
            digest.never_retrieved as f64 / digest.memories as f64 * 100.0
        } else {
            0.0
        },
        stale = digest.stale,
        retention = digest.mean_retention,
        contradictions = digest.contradictions,
        undecidable = digest.undecidable_contradictions,
        pending = digest.jobs_pending,
        dead = digest.jobs_dead,
        lag = digest
            .capture_lag_minutes
            .map(|lag| format!("{lag:.0} min"))
            .unwrap_or_else(|| "—".to_owned()),
    );
    for note in &digest.notes {
        out.push_str(&format!("- {note}\n"));
    }
    out.push('\n');
    out
}

/// Render for a terminal.
pub fn render(digest: &Digest) -> String {
    let mut out = format!(
        "health digest — {}\n\n  {:>9} events\n  {:>9} memories · {} never retrieved · {} stale\n  \
         {:>9.3} mean retention\n  {:>9} contradictions ({} undecidable)\n  {:>9} jobs pending, {} dead\n  \
         {:>9} since the newest captured event\n\n",
        digest.generated_at.date(),
        digest.events,
        digest.memories,
        digest.never_retrieved,
        digest.stale,
        digest.mean_retention,
        digest.contradictions,
        digest.undecidable_contradictions,
        digest.jobs_pending,
        digest.jobs_dead,
        digest
            .capture_lag_minutes
            .map(|lag| format!("{lag:.0} min"))
            .unwrap_or_else(|| "—".to_owned()),
    );
    for note in &digest.notes {
        out.push_str(&format!("  · {note}\n"));
    }
    out
}
