use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, bail};
use brain_adapters::{NormalizeContext, ReadOutcome, SourceAdapter, SourceDescriptor};
use brain_domain::{CaptureGapRecord, EventBatch, ProjectId, QuarantinedRecord};
use brain_store::EventLedger;

use crate::CaptureServiceConfig;
use crate::health::{BatchHealthUpdate, ServiceHealth};

pub struct CaptureBinding {
    pub adapter: Arc<dyn SourceAdapter>,
    pub source: SourceDescriptor,
    pub context: NormalizeContext,
    pub ledger_path: PathBuf,
}

impl CaptureBinding {
    pub fn new(
        adapter: Arc<dyn SourceAdapter>,
        source: SourceDescriptor,
        context: NormalizeContext,
        ledger_path: impl AsRef<Path>,
    ) -> Self {
        Self {
            adapter,
            source,
            context,
            ledger_path: ledger_path.as_ref().to_path_buf(),
        }
    }
}

pub struct CaptureSupervisor {
    bindings: Vec<CaptureBinding>,
    stores: HashMap<ProjectId, Mutex<EventLedger>>,
    source_locks: HashMap<String, tokio::sync::Mutex<()>>,
    config: CaptureServiceConfig,
    health: Mutex<ServiceHealth>,
}

impl CaptureSupervisor {
    pub fn new(bindings: Vec<CaptureBinding>) -> Result<Self> {
        Self::with_config(bindings, CaptureServiceConfig::default())
    }

    pub fn with_config(
        bindings: Vec<CaptureBinding>,
        config: CaptureServiceConfig,
    ) -> Result<Self> {
        config.validate()?;
        let mut stores = HashMap::new();
        let mut paths = HashMap::<PathBuf, ProjectId>::new();
        let mut source_ids = std::collections::HashSet::new();
        for binding in &bindings {
            if !source_ids.insert(binding.source.source_id.clone()) {
                bail!("duplicate source id {}", binding.source.source_id);
            }
            if let Some(other_project) = paths.get(&binding.ledger_path)
                && *other_project != binding.context.project_id
            {
                bail!(
                    "ledger path {} is assigned to multiple projects",
                    binding.ledger_path.display()
                );
            }
            paths.insert(binding.ledger_path.clone(), binding.context.project_id);
            if let std::collections::hash_map::Entry::Vacant(entry) =
                stores.entry(binding.context.project_id)
            {
                entry.insert(Mutex::new(EventLedger::open(
                    &binding.ledger_path,
                    binding.context.project_id,
                )?));
            }
        }
        let mut health = ServiceHealth::new();
        for (project_id, store) in &stores {
            let store = store
                .lock()
                .map_err(|_| anyhow::anyhow!("event ledger lock is poisoned"))?;
            health.register_project(*project_id, store.event_count()?, store.latest_event_at()?);
        }
        for binding in &bindings {
            let store = stores
                .get(&binding.context.project_id)
                .expect("store was created for every binding")
                .lock()
                .map_err(|_| anyhow::anyhow!("event ledger lock is poisoned"))?;
            let cursor = store.cursor(&binding.source.source_id)?;
            let quarantined_count = store.quarantine_count(&binding.source.source_id)?;
            let capture_gaps = store.unresolved_capture_gap_count(&binding.source.source_id)?;
            drop(store);
            let fingerprint = binding
                .adapter
                .fingerprint(&binding.source)
                .map(|fingerprint| fingerprint.0);
            let (schema_fingerprint, last_error) = match fingerprint {
                Ok(fingerprint) => (Some(fingerprint), None),
                Err(error) => (None, Some(error.to_string())),
            };
            health.register_source(
                binding.source.source_id.clone(),
                binding.source.path.clone(),
                binding.context.project_id,
                cursor.clone(),
                quarantined_count,
                capture_gaps,
                backlog_bytes(&binding.source.path, &cursor),
                schema_fingerprint,
                last_error,
            );
        }
        let source_locks = bindings
            .iter()
            .map(|binding| {
                (
                    binding.source.source_id.clone(),
                    tokio::sync::Mutex::new(()),
                )
            })
            .collect();
        Ok(Self {
            bindings,
            stores,
            source_locks,
            config,
            health: Mutex::new(health),
        })
    }

    pub async fn capture_once(&self) -> Result<()> {
        for binding in &self.bindings {
            let _source_guard = self
                .source_locks
                .get(&binding.source.source_id)
                .expect("every binding has a source lock")
                .lock()
                .await;
            let store = self.store(binding.context.project_id)?;
            let cursor = store
                .lock()
                .map_err(|_| anyhow::anyhow!("event ledger lock is poisoned"))?
                .cursor(&binding.source.source_id)?;
            let fingerprint = match binding.adapter.fingerprint(&binding.source) {
                Ok(fingerprint) => fingerprint,
                Err(error) => {
                    self.record_error(&binding.source.source_id, error.to_string())?;
                    return Err(error);
                }
            };
            self.health
                .lock()
                .map_err(|_| anyhow::anyhow!("service health lock is poisoned"))?
                .record_fingerprint(&binding.source.source_id, fingerprint.0);
            let outcome = match binding.adapter.read_increment(&binding.source, &cursor) {
                Ok(outcome) => outcome,
                Err(error) => {
                    self.record_error(&binding.source.source_id, error.to_string())?;
                    return Err(error);
                }
            };
            match outcome {
                ReadOutcome::Batch(batch) => {
                    let mut events = Vec::new();
                    let observed_at = time::OffsetDateTime::now_utc();
                    let quarantined = batch
                        .records
                        .iter()
                        .filter_map(|record| {
                            record.parse_error.as_ref().map(|error| QuarantinedRecord {
                                source_locator: record.source_locator.clone(),
                                source_offset: i64::try_from(record.byte_offset)
                                    .expect("source offset is bounded by i64 for SQLite"),
                                raw_hash: record.raw_hash,
                                error: error.clone(),
                                observed_at,
                            })
                        })
                        .collect::<Vec<_>>();
                    let capture_gaps = batch
                        .rotation
                        .as_ref()
                        .map(|rotation| CaptureGapRecord {
                            expected_cursor: cursor.clone(),
                            observed_cursor: Some(batch.next_cursor.clone()),
                            reason: format!(
                                "source identity changed from {:?} to {} at byte {} (new size {})",
                                rotation.previous_identity,
                                rotation.current_identity,
                                rotation.previous_offset,
                                rotation.current_size
                            ),
                            observed_at,
                        })
                        .into_iter()
                        .collect::<Vec<_>>();
                    let next_cursor = batch.next_cursor.clone();
                    let backlog = backlog_bytes(&binding.source.path, &next_cursor);
                    for record in &batch.records {
                        events.extend(binding.adapter.normalize(record, &binding.context)?);
                    }
                    let (result, persisted_events, last_event_at) = {
                        let mut store = store
                            .lock()
                            .map_err(|_| anyhow::anyhow!("event ledger lock is poisoned"))?;
                        let result = store.append_batch(&EventBatch {
                            source_id: binding.source.source_id.clone(),
                            events,
                            quarantined,
                            capture_gaps,
                            next_cursor: batch.next_cursor,
                        })?;
                        let persisted_events = store.event_count()?;
                        let last_event_at = store.latest_event_at()?;
                        (result, persisted_events, last_event_at)
                    };
                    self.health
                        .lock()
                        .map_err(|_| anyhow::anyhow!("service health lock is poisoned"))?
                        .record_batch(BatchHealthUpdate {
                            source_id: binding.source.source_id.clone(),
                            project_id: binding.context.project_id,
                            cursor: next_cursor,
                            inserted: u64::try_from(result.inserted)?,
                            quarantined: u64::try_from(result.quarantined)?,
                            capture_gaps: u64::try_from(result.capture_gaps)?,
                            persisted_events,
                            last_event_at,
                            backlog_bytes: backlog,
                        });
                }
                ReadOutcome::NoChange => {
                    self.health
                        .lock()
                        .map_err(|_| anyhow::anyhow!("service health lock is poisoned"))?
                        .record_check(
                            &binding.source.source_id,
                            backlog_bytes(&binding.source.path, &cursor),
                        );
                }
                ReadOutcome::SchemaDrift(drift) => {
                    self.health
                        .lock()
                        .map_err(|_| anyhow::anyhow!("service health lock is poisoned"))?
                        .record_error(
                            &binding.source.source_id,
                            format!("schema drift: {}", drift.observed.0),
                        );
                    bail!("source {} has schema drift", drift.source_id);
                }
                ReadOutcome::SourceUnavailable(unavailable) => {
                    self.health
                        .lock()
                        .map_err(|_| anyhow::anyhow!("service health lock is poisoned"))?
                        .record_error(&binding.source.source_id, unavailable.reason.clone());
                    bail!(
                        "source {} is unavailable: {}",
                        unavailable.source_id,
                        unavailable.reason
                    );
                }
            }
        }
        Ok(())
    }

    pub async fn run(self: Arc<Self>, shutdown: tokio::sync::watch::Receiver<bool>) -> Result<()> {
        crate::reconcile::run(self, shutdown).await
    }

    pub fn event_count(&self, project_id: ProjectId) -> Result<u64> {
        self.store(project_id)?
            .lock()
            .map_err(|_| anyhow::anyhow!("event ledger lock is poisoned"))?
            .event_count()
    }

    pub fn raw_contains(&self, project_id: ProjectId, needle: &str) -> Result<bool> {
        self.store(project_id)?
            .lock()
            .map_err(|_| anyhow::anyhow!("event ledger lock is poisoned"))?
            .raw_contains(needle)
    }

    pub fn health(&self) -> Result<ServiceHealth> {
        self.health
            .lock()
            .map_err(|_| anyhow::anyhow!("service health lock is poisoned"))
            .map(|health| health.clone())
    }

    fn store(&self, project_id: ProjectId) -> Result<&Mutex<EventLedger>> {
        self.stores
            .get(&project_id)
            .with_context(|| format!("project {} has no event ledger", project_id.0))
    }

    fn record_error(&self, source_id: &str, error: String) -> Result<()> {
        self.health
            .lock()
            .map_err(|_| anyhow::anyhow!("service health lock is poisoned"))?
            .record_error(source_id, error);
        Ok(())
    }

    pub(crate) fn watched_paths(&self) -> impl Iterator<Item = &Path> {
        self.bindings
            .iter()
            .map(|binding| binding.source.path.as_path())
    }

    pub(crate) fn config(&self) -> CaptureServiceConfig {
        self.config
    }
}

fn backlog_bytes(path: &Path, cursor: &brain_domain::SourceCursor) -> u64 {
    std::fs::metadata(path)
        .map(|metadata| metadata.len().saturating_sub(cursor.byte_offset))
        .unwrap_or(0)
}
