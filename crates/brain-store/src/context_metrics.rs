use brain_domain::{Harness, ProjectId};

/// One orientation handed to a coding agent.
///
/// The captured event stream records what the agents did. It does not record what this
/// system gave them, because a hook reply is not an event and is never written back to any
/// transcript. Delivered context size is therefore the one figure that cannot be
/// reconstructed after the fact, which is why it is recorded as it happens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextDelivery {
    pub project_id: ProjectId,
    pub harness: Harness,
    pub native_session_id: Option<String>,
    pub event_name: String,
    pub delivered_at: time::OffsetDateTime,
    pub total_tokens: u64,
    /// Tokens from compiled memory alone, excluding coordination.
    pub memory_tokens: u64,
    /// Tokens from lease warnings and path-claim overlap notices.
    pub coordination_tokens: u64,
    pub citation_count: u64,
}

/// Aggregate over a window, which is the shape a baseline comparison needs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ContextDeliverySummary {
    pub deliveries: u64,
    pub total_tokens: u64,
    pub max_tokens: u64,
}

impl ContextDeliverySummary {
    /// Mean tokens per delivery, or zero when nothing was delivered. Reported alongside the
    /// maximum because a mean alone hides a single oversized orientation.
    pub fn mean_tokens(&self) -> f64 {
        if self.deliveries == 0 {
            return 0.0;
        }
        self.total_tokens as f64 / self.deliveries as f64
    }
}
