//! Rendering an eviction plan for a human to approve.
//!
//! Dry by default and loud about the gate. The number that matters when reading one of these is not
//! how many memories it would retire but **what share** — a policy proposing to retire most of a
//! brain has found a retrieval problem, not a decay problem, and applying it would delete the
//! evidence for that.

use brain_store::{EvictionGate, EvictionPlan, MINIMUM_OBSERVATION, QUIET_FOR};

/// How many candidates to name before summarising the rest.
const SHOWN: usize = 15;

/// The share above which the plan is reported as suspicious rather than routine.
///
/// Not a hard stop — this prints, a human decides. But a brain retiring more than half of itself is
/// describing a corpus nothing retrieves from, and that reading should arrive with the number rather
/// than after it.
const SUSPICIOUS_SHARE: f64 = 0.5;

pub fn render(plan: &EvictionPlan) -> String {
    let mut out = String::new();
    match plan.gate {
        EvictionGate::TooYoung { days_remaining } => {
            out.push_str(&format!(
                "eviction is closed — access counting has run {} of the {} days it needs, \
                 {days_remaining} to go\n\n",
                plan.observed_days,
                MINIMUM_OBSERVATION.whole_days()
            ));
            out.push_str(
                "  Until then, \"never retrieved\" cannot tell a memory nothing wants from one\n  \
                 nothing has had the chance to want. What follows is a preview.\n\n",
            );
        }
        EvictionGate::Open => {
            out.push_str(&format!(
                "eviction is open — access counted for {} days\n\n",
                plan.observed_days
            ));
        }
    }

    if plan.candidates.is_empty() {
        out.push_str(&format!(
            "  nothing qualifies: no current memory has gone {} days unretrieved\n",
            QUIET_FOR.whole_days()
        ));
        if plan.protected > 0 {
            out.push_str(&format!("  {} held back by authority\n", plan.protected));
        }
        return out;
    }

    let share = plan.share();
    out.push_str(&format!(
        "  {} of {} current memories ({:.1}%) would be retired\n",
        plan.candidates.len(),
        plan.total_current,
        share * 100.0
    ));
    if plan.protected > 0 {
        out.push_str(&format!(
            "  {} met the test and are protected — a human filed them\n",
            plan.protected
        ));
    }
    if share > SUSPICIOUS_SHARE {
        out.push_str(
            "\n  ! This is most of the brain. That is a retrieval finding before it is a decay\n  \
             finding — check that search is reaching these before retiring them, because\n  \
             retiring them removes the evidence that it was not.\n",
        );
    }
    out.push('\n');

    for candidate in plan.candidates.iter().take(SHOWN) {
        out.push_str(&format!(
            "    {} — {}\n",
            truncate(&candidate.title, 68),
            candidate.reason
        ));
    }
    if plan.candidates.len() > SHOWN {
        out.push_str(&format!(
            "    … and {} more\n",
            plan.candidates.len() - SHOWN
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
