use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use brain_adapters::{HermesActivation, HermesAdapter, SourceDescriptor};
use brain_domain::{ProjectId, WorktreeId};
use brain_service::ServiceLaunchConfig;
use brain_store::EventLedger;

use crate::session_status::{
    SessionFilter, SessionLifecycleState, SessionStatusOptions, read_session_status,
};

#[derive(Clone, Debug, serde::Serialize)]
pub struct BrainStatus {
    pub brain_home: PathBuf,
    pub project_root: PathBuf,
    pub project_id: ProjectId,
    pub worktree_id: WorktreeId,
    pub ledger_path: PathBuf,
    pub service_config_path: PathBuf,
    pub pipe_name: String,
    pub persisted_events: u64,
    pub last_event_at: Option<time::OffsetDateTime>,
    pub source_count: usize,
    pub backlog_bytes: u64,
    pub quarantined_records: u64,
    pub unresolved_capture_gaps: u64,
    pub active_schema_drifts: u64,
    pub session_lifecycle: SessionLifecycleCounts,
    pub healthy: bool,
}

#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct SessionLifecycleCounts {
    pub active: u64,
    pub stale_open: u64,
    pub closed: u64,
    pub historical_uninstrumented: u64,
    pub truncated: bool,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct HermesStatus {
    pub database_path: PathBuf,
    pub project_root: PathBuf,
    pub project_id: ProjectId,
    pub activation: String,
    pub expected_fingerprint: String,
    pub observed_fingerprint: String,
    pub reason: Option<String>,
}

pub fn read_status(
    brain_home: impl AsRef<Path>,
    project: Option<ProjectId>,
) -> Result<BrainStatus> {
    let brain_home = brain_home.as_ref();
    let service_config_path = ServiceLaunchConfig::default_path(brain_home);
    let config = ServiceLaunchConfig::load(&service_config_path)?;
    let project_config = config.project(project)?;
    let ledger = EventLedger::open(&project_config.ledger_path, project_config.project_id)
        .context("open configured project ledger")?;
    let mut backlog_bytes = 0;
    let mut quarantined_records = 0;
    let mut unresolved_capture_gaps = 0;
    for path in project_config
        .claude_sources
        .iter()
        .chain(&project_config.codex_sources)
    {
        let source = SourceDescriptor::file(path);
        let cursor = ledger.cursor(&source.source_id)?;
        backlog_bytes += std::fs::metadata(path)
            .map(|metadata| metadata.len().saturating_sub(cursor.byte_offset))
            .unwrap_or(0);
        quarantined_records += ledger.quarantine_count(&source.source_id)?;
        unresolved_capture_gaps += ledger.unresolved_capture_gap_count(&source.source_id)?;
    }
    if let Some(path) = &project_config.hermes_database {
        let source = SourceDescriptor::file(path);
        quarantined_records += ledger.quarantine_count(&source.source_id)?;
        unresolved_capture_gaps += ledger.unresolved_capture_gap_count(&source.source_id)?;
    }
    let active_schema_drifts = ledger.active_schema_drift_count()?;
    let session_page = read_session_status(
        &ledger,
        project_config.project_id,
        SessionStatusOptions {
            filter: SessionFilter::All,
            limit: 200,
            cursor: None,
            now: time::OffsetDateTime::now_utc(),
            stale_after: time::Duration::minutes(30),
        },
    )?;
    let mut session_lifecycle = SessionLifecycleCounts {
        truncated: session_page.truncated,
        ..SessionLifecycleCounts::default()
    };
    for session in session_page.sessions {
        match session.state {
            SessionLifecycleState::Active => session_lifecycle.active += 1,
            SessionLifecycleState::StaleOpen => session_lifecycle.stale_open += 1,
            SessionLifecycleState::Closed => session_lifecycle.closed += 1,
            SessionLifecycleState::HistoricalUninstrumented => {
                session_lifecycle.historical_uninstrumented += 1
            }
        }
    }

    Ok(BrainStatus {
        brain_home: brain_home.to_path_buf(),
        project_root: project_config.project_root.clone(),
        project_id: project_config.project_id,
        worktree_id: project_config.worktree_id,
        ledger_path: project_config.ledger_path.clone(),
        service_config_path,
        pipe_name: config.pipe_name.clone(),
        persisted_events: ledger.event_count()?,
        last_event_at: ledger.latest_event_at()?,
        source_count: project_config.claude_sources.len()
            + project_config.codex_sources.len()
            + usize::from(project_config.hermes_database.is_some()),
        backlog_bytes,
        quarantined_records,
        unresolved_capture_gaps,
        active_schema_drifts,
        session_lifecycle,
        healthy: unresolved_capture_gaps == 0 && active_schema_drifts == 0,
    })
}

pub fn read_hermes_status(
    brain_home: impl AsRef<Path>,
    database_path: impl AsRef<Path>,
    project: Option<ProjectId>,
) -> Result<HermesStatus> {
    let config = ServiceLaunchConfig::load(ServiceLaunchConfig::default_path(brain_home))?;
    let project_config = config.project(project)?;
    let adapter =
        HermesAdapter::reviewed_for_project(database_path.as_ref(), &project_config.project_root)?;
    let status = adapter.activation()?;
    let (activation, expected_fingerprint, observed_fingerprint, reason) = match status {
        HermesActivation::Active { fingerprint } => (
            "active".to_owned(),
            fingerprint.0.clone(),
            fingerprint.0,
            None,
        ),
        HermesActivation::FixtureOnly {
            expected,
            observed,
            reason,
        } => (
            "fixture_only".to_owned(),
            expected.0,
            observed.0,
            Some(reason),
        ),
    };
    Ok(HermesStatus {
        database_path: database_path.as_ref().to_path_buf(),
        project_root: project_config.project_root.clone(),
        project_id: project_config.project_id,
        activation,
        expected_fingerprint,
        observed_fingerprint,
        reason,
    })
}
