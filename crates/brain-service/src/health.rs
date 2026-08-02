use std::collections::BTreeMap;
use std::path::PathBuf;

use brain_domain::{ProjectId, SourceCursor};

#[derive(Clone, Debug, serde::Serialize)]
pub struct ServiceHealth {
    pub started_at: time::OffsetDateTime,
    pub projects: BTreeMap<String, ProjectHealth>,
    pub sources: BTreeMap<String, SourceHealth>,
}

impl ServiceHealth {
    pub(crate) fn new() -> Self {
        Self {
            started_at: time::OffsetDateTime::now_utc(),
            projects: BTreeMap::new(),
            sources: BTreeMap::new(),
        }
    }

    pub fn is_healthy(&self) -> bool {
        self.sources
            .values()
            .all(|source| source.capture_gaps == 0 && source.last_error.is_none())
    }

    pub(crate) fn record_batch(&mut self, update: BatchHealthUpdate) {
        if let Some(source) = self.sources.get_mut(&update.source_id) {
            let now = time::OffsetDateTime::now_utc();
            source.last_cursor = update.cursor;
            source.captured_events += update.inserted;
            source.parse_failures += update.quarantined;
            source.quarantined_count += update.quarantined;
            source.capture_gaps += update.capture_gaps;
            source.backlog_bytes = update.backlog_bytes;
            source.last_checked_at = Some(now);
            source.last_success_at = Some(now);
            source.last_error = None;
        }
        if let Some(project) = self.projects.get_mut(&update.project_id.0.to_string()) {
            project.persisted_events = update.persisted_events;
            project.last_event_at = update.last_event_at;
        }
    }

    pub(crate) fn record_check(&mut self, source_id: &str, backlog_bytes: u64) {
        if let Some(source) = self.sources.get_mut(source_id) {
            source.last_checked_at = Some(time::OffsetDateTime::now_utc());
            source.backlog_bytes = backlog_bytes;
        }
    }

    pub(crate) fn record_error(&mut self, source_id: &str, error: String) {
        if let Some(source) = self.sources.get_mut(source_id) {
            source.last_checked_at = Some(time::OffsetDateTime::now_utc());
            source.last_error = Some(error);
        }
    }

    pub(crate) fn register_project(
        &mut self,
        project_id: ProjectId,
        persisted_events: u64,
        last_event_at: Option<time::OffsetDateTime>,
    ) {
        self.projects.insert(
            project_id.0.to_string(),
            ProjectHealth {
                project_id,
                persisted_events,
                last_event_at,
            },
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn register_source(
        &mut self,
        source_id: String,
        source_path: PathBuf,
        project_id: ProjectId,
        last_cursor: SourceCursor,
        quarantined_count: u64,
        capture_gaps: u64,
        backlog_bytes: u64,
        schema_fingerprint: Option<String>,
        last_error: Option<String>,
    ) {
        self.sources.insert(
            source_id,
            SourceHealth {
                source_path,
                project_id,
                last_cursor,
                captured_events: 0,
                parse_failures: quarantined_count,
                quarantined_count,
                capture_gaps,
                backlog_bytes,
                schema_fingerprint,
                schema_fingerprint_changes: 0,
                last_checked_at: None,
                last_success_at: None,
                last_error,
            },
        );
    }

    pub(crate) fn record_fingerprint(&mut self, source_id: &str, fingerprint: String) {
        if let Some(source) = self.sources.get_mut(source_id) {
            if source
                .schema_fingerprint
                .as_ref()
                .is_some_and(|previous| previous != &fingerprint)
            {
                source.schema_fingerprint_changes += 1;
            }
            source.schema_fingerprint = Some(fingerprint);
        }
    }
}

pub(crate) struct BatchHealthUpdate {
    pub source_id: String,
    pub project_id: ProjectId,
    pub cursor: SourceCursor,
    pub inserted: u64,
    pub quarantined: u64,
    pub capture_gaps: u64,
    pub persisted_events: u64,
    pub last_event_at: Option<time::OffsetDateTime>,
    pub backlog_bytes: u64,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct ProjectHealth {
    pub project_id: ProjectId,
    pub persisted_events: u64,
    pub last_event_at: Option<time::OffsetDateTime>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct SourceHealth {
    pub source_path: PathBuf,
    pub project_id: ProjectId,
    pub last_cursor: SourceCursor,
    pub captured_events: u64,
    pub parse_failures: u64,
    pub quarantined_count: u64,
    pub capture_gaps: u64,
    pub backlog_bytes: u64,
    pub schema_fingerprint: Option<String>,
    pub schema_fingerprint_changes: u64,
    pub last_checked_at: Option<time::OffsetDateTime>,
    pub last_success_at: Option<time::OffsetDateTime>,
    pub last_error: Option<String>,
}
