use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use brain_adapters::{HermesActivation, HermesAdapter, SourceDescriptor};
use brain_domain::{ProjectId, WorktreeId};
use brain_service::ServiceLaunchConfig;
use brain_store::EventLedger;

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
    pub healthy: bool,
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
    if let Some(project) = project
        && project != config.project_id
    {
        bail!(
            "project {} is not the configured service scope {}",
            project.0,
            config.project_id.0
        );
    }
    let ledger = EventLedger::open(&config.ledger_path, config.project_id)
        .context("open configured project ledger")?;
    let mut backlog_bytes = 0;
    let mut quarantined_records = 0;
    let mut unresolved_capture_gaps = 0;
    for path in &config.claude_sources {
        let source = SourceDescriptor::file(path);
        let cursor = ledger.cursor(&source.source_id)?;
        backlog_bytes += std::fs::metadata(path)
            .map(|metadata| metadata.len().saturating_sub(cursor.byte_offset))
            .unwrap_or(0);
        quarantined_records += ledger.quarantine_count(&source.source_id)?;
        unresolved_capture_gaps += ledger.unresolved_capture_gap_count(&source.source_id)?;
    }
    let active_schema_drifts = ledger.active_schema_drift_count()?;

    Ok(BrainStatus {
        brain_home: brain_home.to_path_buf(),
        project_root: config.project_root,
        project_id: config.project_id,
        worktree_id: config.worktree_id,
        ledger_path: config.ledger_path,
        service_config_path,
        pipe_name: config.pipe_name,
        persisted_events: ledger.event_count()?,
        last_event_at: ledger.latest_event_at()?,
        source_count: config.claude_sources.len(),
        backlog_bytes,
        quarantined_records,
        unresolved_capture_gaps,
        active_schema_drifts,
        healthy: unresolved_capture_gaps == 0 && active_schema_drifts == 0,
    })
}

pub fn read_hermes_status(
    brain_home: impl AsRef<Path>,
    database_path: impl AsRef<Path>,
    project: Option<ProjectId>,
) -> Result<HermesStatus> {
    let config = ServiceLaunchConfig::load(ServiceLaunchConfig::default_path(brain_home))?;
    if let Some(project) = project
        && project != config.project_id
    {
        bail!(
            "project {} is not the configured service scope {}",
            project.0,
            config.project_id.0
        );
    }
    let adapter =
        HermesAdapter::reviewed_for_project(database_path.as_ref(), &config.project_root)?;
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
        project_root: config.project_root,
        project_id: config.project_id,
        activation,
        expected_fingerprint,
        observed_fingerprint,
        reason,
    })
}
