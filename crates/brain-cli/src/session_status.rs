use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, ensure};
use brain_domain::{Harness, ProjectId};
use brain_store::{
    EventLedger, LifecycleChannel, LifecycleEvent, LifecycleStage, RetrievalDecision,
    RetrievalOutcome, SessionAttribution, TelemetryQuery,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionFilter {
    All,
    Active,
    Closed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionLifecycleState {
    Active,
    StaleOpen,
    Closed,
    HistoricalUninstrumented,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelState {
    Delivered,
    HealthySilence,
    Failed,
    Pending,
    NotRequested,
    Missing,
    BoundaryStored,
    CaptureCaughtUp,
    HistoricalUninstrumented,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct SessionChannelStatus {
    pub state: ChannelState,
    #[serde(with = "time::serde::rfc3339::option")]
    pub last_observed_at: Option<time::OffsetDateTime>,
    pub detail: String,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct SessionStatus {
    pub harness: Harness,
    pub native_session_id: String,
    pub state: SessionLifecycleState,
    #[serde(with = "time::serde::rfc3339")]
    pub first_observed_at: time::OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub last_observed_at: time::OffsetDateTime,
    pub startup: SessionChannelStatus,
    pub prompt_push: SessionChannelStatus,
    pub mcp_pull: SessionChannelStatus,
    pub session_end: SessionChannelStatus,
    pub capture: SessionChannelStatus,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct SessionActivity {
    pub harness: Harness,
    pub native_session_id: String,
    #[serde(with = "time::serde::rfc3339")]
    pub first_observed_at: time::OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub last_observed_at: time::OffsetDateTime,
}

#[derive(Clone, Debug)]
pub struct SessionStatusOptions {
    pub filter: SessionFilter,
    pub limit: usize,
    pub cursor: Option<String>,
    pub now: time::OffsetDateTime,
    pub stale_after: time::Duration,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct SessionStatusPage {
    pub schema_version: u32,
    pub project_id: ProjectId,
    #[serde(with = "time::serde::rfc3339")]
    pub generated_at: time::OffsetDateTime,
    pub sessions: Vec<SessionStatus>,
    pub next_cursor: Option<String>,
    pub truncated: bool,
}

pub fn read_session_status(
    ledger: &EventLedger,
    project_id: ProjectId,
    options: SessionStatusOptions,
) -> Result<SessionStatusPage> {
    let query = TelemetryQuery {
        project_id,
        session: None,
        start: time::OffsetDateTime::UNIX_EPOCH,
        end: options.now + time::Duration::seconds(1),
        limit: 10_000,
    };
    let lifecycle = ledger.lifecycle_events(&query)?;
    let decisions = ledger.retrieval_decisions(&query)?;
    let mut activity: BTreeMap<(String, String), SessionActivity> = BTreeMap::new();
    for event in ledger.recent_events_as_of(project_id, options.now, 500)? {
        let key = (
            event.harness.as_str().to_owned(),
            event.native_session_id.clone(),
        );
        activity
            .entry(key)
            .and_modify(|current| {
                current.first_observed_at = current.first_observed_at.min(event.observed_at);
                current.last_observed_at = current.last_observed_at.max(event.observed_at);
            })
            .or_insert(SessionActivity {
                harness: event.harness,
                native_session_id: event.native_session_id,
                first_observed_at: event.observed_at,
                last_observed_at: event.observed_at,
            });
    }
    fold_session_status(
        project_id,
        &lifecycle,
        &decisions,
        &activity.into_values().collect::<Vec<_>>(),
        options,
    )
}

pub fn fold_session_status(
    project_id: ProjectId,
    lifecycle: &[LifecycleEvent],
    decisions: &[RetrievalDecision],
    activity: &[SessionActivity],
    options: SessionStatusOptions,
) -> Result<SessionStatusPage> {
    ensure!(
        (1..=200).contains(&options.limit),
        "session page limit must be 1-200"
    );
    ensure!(
        lifecycle.iter().all(|event| event.project_id == project_id)
            && decisions
                .iter()
                .all(|decision| decision.project_id == project_id),
        "session telemetry violates project scope"
    );
    let offset = options
        .cursor
        .as_deref()
        .map(str::parse::<usize>)
        .transpose()
        .map_err(|_| anyhow::anyhow!("invalid session cursor"))?
        .unwrap_or(0);
    let mut keys = BTreeSet::new();
    let instrumentation_started_at = lifecycle
        .iter()
        .map(|event| event.occurred_at)
        .chain(decisions.iter().map(|decision| decision.occurred_at))
        .min();
    for item in activity {
        keys.insert((
            item.harness.as_str().to_owned(),
            canonical_session_id(&item.harness, &item.native_session_id),
        ));
    }
    for event in lifecycle {
        if let SessionAttribution::Attributed(session_id) = &event.session {
            keys.insert((
                event.harness.as_str().to_owned(),
                canonical_session_id(&event.harness, session_id),
            ));
        }
    }
    for decision in decisions {
        if let SessionAttribution::Attributed(session_id) = &decision.session {
            keys.insert((
                decision.harness.as_str().to_owned(),
                canonical_session_id(&decision.harness, session_id),
            ));
        }
    }
    let mut sessions = Vec::new();
    for (harness_name, session_id) in keys {
        let session_events = lifecycle
            .iter()
            .filter(|event| {
                event.harness.as_str() == harness_name
                    && matches!(
                        &event.session,
                        SessionAttribution::Attributed(value)
                            if canonical_session_id(&event.harness, value) == session_id
                    )
            })
            .collect::<Vec<_>>();
        let session_decisions = decisions
            .iter()
            .filter(|decision| {
                decision.harness.as_str() == harness_name
                    && matches!(
                        &decision.session,
                        SessionAttribution::Attributed(value)
                            if canonical_session_id(&decision.harness, value) == session_id
                    )
            })
            .collect::<Vec<_>>();
        let observed = activity
            .iter()
            .filter(|item| {
                item.harness.as_str() == harness_name
                    && canonical_session_id(&item.harness, &item.native_session_id) == session_id
            })
            .collect::<Vec<_>>();
        let first = session_events
            .iter()
            .map(|event| event.occurred_at)
            .chain(
                session_decisions
                    .iter()
                    .map(|decision| decision.occurred_at),
            )
            .chain(observed.iter().map(|item| item.first_observed_at))
            .min()
            .unwrap_or(options.now);
        let last = session_events
            .iter()
            .map(|event| event.occurred_at)
            .chain(
                session_decisions
                    .iter()
                    .map(|decision| decision.occurred_at),
            )
            .chain(observed.iter().map(|item| item.last_observed_at))
            .max()
            .unwrap_or(first);
        let historical = session_events.is_empty() && session_decisions.is_empty();
        let predates_instrumentation = instrumentation_started_at.is_some_and(|started_at| {
            observed
                .iter()
                .any(|item| item.first_observed_at < started_at)
        });
        let channel_is_uninstrumented = |channel: LifecycleChannel| {
            predates_instrumentation
                && !session_events.iter().any(|event| event.channel == channel)
                && !session_decisions
                    .iter()
                    .any(|decision| decision.channel == channel)
        };
        let closed = has_stage(
            &session_events,
            LifecycleChannel::SessionEnd,
            LifecycleStage::SessionEndPersisted,
        );
        let state = if historical {
            SessionLifecycleState::HistoricalUninstrumented
        } else if closed {
            SessionLifecycleState::Closed
        } else if options.now - last >= options.stale_after {
            SessionLifecycleState::StaleOpen
        } else {
            SessionLifecycleState::Active
        };
        let harness = session_events
            .first()
            .map(|event| event.harness.clone())
            .or_else(|| {
                session_decisions
                    .first()
                    .map(|decision| decision.harness.clone())
            })
            .or_else(|| observed.first().map(|item| item.harness.clone()))
            .unwrap_or_else(|| Harness::Other(harness_name.clone()));
        sessions.push(SessionStatus {
            harness,
            native_session_id: session_id,
            state,
            first_observed_at: first,
            last_observed_at: last,
            startup: channel_status(
                LifecycleChannel::SessionStart,
                &session_events,
                &session_decisions,
                historical || channel_is_uninstrumented(LifecycleChannel::SessionStart),
            ),
            prompt_push: channel_status(
                LifecycleChannel::UserPromptSubmit,
                &session_events,
                &session_decisions,
                historical || channel_is_uninstrumented(LifecycleChannel::UserPromptSubmit),
            ),
            mcp_pull: channel_status(
                LifecycleChannel::BrainMcp,
                &session_events,
                &session_decisions,
                historical,
            ),
            session_end: boundary_status(&session_events, historical, closed),
            capture: capture_status(&session_events, historical, closed),
        });
    }
    sessions.retain(|session| match options.filter {
        SessionFilter::All => true,
        SessionFilter::Active => matches!(
            session.state,
            SessionLifecycleState::Active | SessionLifecycleState::StaleOpen
        ),
        SessionFilter::Closed => session.state == SessionLifecycleState::Closed,
    });
    sessions.sort_by(|left, right| {
        right
            .last_observed_at
            .cmp(&left.last_observed_at)
            .then_with(|| left.native_session_id.cmp(&right.native_session_id))
    });
    let total = sessions.len();
    let page = sessions
        .into_iter()
        .skip(offset)
        .take(options.limit)
        .collect::<Vec<_>>();
    let consumed = offset.saturating_add(page.len());
    let truncated = consumed < total;
    Ok(SessionStatusPage {
        schema_version: 1,
        project_id,
        generated_at: options.now,
        sessions: page,
        next_cursor: truncated.then(|| consumed.to_string()),
        truncated,
    })
}

fn canonical_session_id(harness: &Harness, native_session_id: &str) -> String {
    if !matches!(harness, Harness::Codex) {
        return native_session_id.to_owned();
    }
    native_session_id
        .get(native_session_id.len().saturating_sub(36)..)
        .and_then(|suffix| uuid::Uuid::parse_str(suffix).ok())
        .map(|session_id| session_id.to_string())
        .unwrap_or_else(|| native_session_id.to_owned())
}

fn channel_status(
    channel: LifecycleChannel,
    events: &[&LifecycleEvent],
    decisions: &[&RetrievalDecision],
    historical: bool,
) -> SessionChannelStatus {
    if historical {
        return status(
            ChannelState::HistoricalUninstrumented,
            None,
            "session predates lifecycle instrumentation",
        );
    }
    let relevant_events = events
        .iter()
        .filter(|event| event.channel == channel)
        .copied()
        .collect::<Vec<_>>();
    let relevant_decisions = decisions
        .iter()
        .filter(|decision| decision.channel == channel)
        .copied()
        .collect::<Vec<_>>();
    let last = relevant_events
        .iter()
        .map(|event| event.occurred_at)
        .chain(
            relevant_decisions
                .iter()
                .map(|decision| decision.occurred_at),
        )
        .max();
    if channel == LifecycleChannel::BrainMcp {
        if !relevant_events
            .iter()
            .any(|event| event.stage == LifecycleStage::McpRequest)
        {
            return status(
                ChannelState::NotRequested,
                last,
                "no on-demand pull was requested",
            );
        }
        let failed = relevant_events
            .iter()
            .rev()
            .find(|event| {
                matches!(
                    event.stage,
                    LifecycleStage::McpSucceeded | LifecycleStage::McpFailed
                )
            })
            .is_some_and(|event| event.stage == LifecycleStage::McpFailed);
        return if failed {
            status(
                ChannelState::Failed,
                last,
                "latest Brain MCP request failed",
            )
        } else {
            status(ChannelState::Delivered, last, "Brain MCP request completed")
        };
    }
    let hook_received = relevant_events
        .iter()
        .any(|event| event.stage == LifecycleStage::HookReceived);
    if !hook_received {
        return status(
            ChannelState::Missing,
            last,
            "expected hook receipt is absent",
        );
    }
    if relevant_decisions
        .iter()
        .rev()
        .any(|decision| decision.outcome == RetrievalOutcome::Failed)
    {
        return status(ChannelState::Failed, last, "retrieval decision failed");
    }
    let flushed = relevant_events
        .iter()
        .any(|event| event.stage == LifecycleStage::ReplyFlushed);
    if !flushed {
        return status(
            ChannelState::Pending,
            last,
            "hook arrived but reply was not confirmed flushed",
        );
    }
    if relevant_decisions
        .iter()
        .rev()
        .any(|decision| decision.outcome == RetrievalOutcome::HealthySilence)
    {
        status(
            ChannelState::HealthySilence,
            last,
            "hook ran and correctly returned no context",
        )
    } else {
        status(ChannelState::Delivered, last, "reply was confirmed flushed")
    }
}

fn boundary_status(
    events: &[&LifecycleEvent],
    historical: bool,
    closed: bool,
) -> SessionChannelStatus {
    if historical {
        return status(
            ChannelState::HistoricalUninstrumented,
            None,
            "boundary was not instrumented",
        );
    }
    let last = channel_last(events, LifecycleChannel::SessionEnd);
    if closed {
        status(
            ChannelState::BoundaryStored,
            last,
            "SessionEnd boundary persisted",
        )
    } else {
        status(ChannelState::NotRequested, last, "session is still open")
    }
}

fn capture_status(
    events: &[&LifecycleEvent],
    historical: bool,
    closed: bool,
) -> SessionChannelStatus {
    if historical {
        return status(
            ChannelState::HistoricalUninstrumented,
            None,
            "capture receipt was not instrumented",
        );
    }
    let last = channel_last(events, LifecycleChannel::Capture);
    if has_stage(
        events,
        LifecycleChannel::Capture,
        LifecycleStage::CaptureCaughtUp,
    ) {
        status(
            ChannelState::CaptureCaughtUp,
            last,
            "capture caught up without claiming EOF",
        )
    } else if closed {
        status(
            ChannelState::Pending,
            last,
            "boundary stored; capture has not reported caught up",
        )
    } else {
        status(
            ChannelState::NotRequested,
            last,
            "capture completion is evaluated at a boundary",
        )
    }
}

fn has_stage(events: &[&LifecycleEvent], channel: LifecycleChannel, stage: LifecycleStage) -> bool {
    events
        .iter()
        .any(|event| event.channel == channel && event.stage == stage)
}

fn channel_last(
    events: &[&LifecycleEvent],
    channel: LifecycleChannel,
) -> Option<time::OffsetDateTime> {
    events
        .iter()
        .filter(|event| event.channel == channel)
        .map(|event| event.occurred_at)
        .max()
}

fn status(
    state: ChannelState,
    last_observed_at: Option<time::OffsetDateTime>,
    detail: &str,
) -> SessionChannelStatus {
    SessionChannelStatus {
        state,
        last_observed_at,
        detail: detail.to_owned(),
    }
}
