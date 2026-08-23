use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, bail};
use brain_adapters::{NormalizeContext, ReadOutcome, SourceAdapter, SourceDescriptor};
use brain_domain::{
    CaptureGapRecord, EventBatch, EventType, ProjectId, QuarantinedRecord, SchemaDriftRecord,
};
use brain_store::ConsolidationReason;
use brain_store::EventLedger;

use crate::CaptureServiceConfig;
use crate::health::{BatchHealthUpdate, ServiceHealth, source_health_key};

#[derive(Clone)]
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

    fn source_key(&self) -> String {
        source_health_key(self.context.project_id, &self.source.source_id)
    }
}

pub struct CaptureSupervisor {
    bindings: Mutex<Vec<CaptureBinding>>,
    stores: HashMap<ProjectId, Mutex<EventLedger>>,
    source_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    config: CaptureServiceConfig,
    health: Mutex<ServiceHealth>,
    pressure: Mutex<crate::PressureController>,
    disk_probe: Arc<dyn crate::DiskProbe>,
    storage_roots: Vec<PathBuf>,
    degradation_tx: tokio::sync::watch::Sender<crate::DegradationState>,
}

impl CaptureSupervisor {
    pub fn new(bindings: Vec<CaptureBinding>) -> Result<Self> {
        Self::with_config(bindings, CaptureServiceConfig::default())
    }

    pub fn with_config(
        bindings: Vec<CaptureBinding>,
        config: CaptureServiceConfig,
    ) -> Result<Self> {
        Self::with_config_and_disk_probe(
            bindings,
            config,
            crate::PressurePolicy::default(),
            Arc::new(crate::FilesystemDiskProbe),
        )
    }

    pub fn with_config_and_disk_probe(
        bindings: Vec<CaptureBinding>,
        config: CaptureServiceConfig,
        pressure_policy: crate::PressurePolicy,
        disk_probe: Arc<dyn crate::DiskProbe>,
    ) -> Result<Self> {
        config.validate()?;
        let mut stores = HashMap::new();
        let mut paths = HashMap::<PathBuf, ProjectId>::new();
        let mut source_ids = std::collections::HashSet::new();
        for binding in &bindings {
            if !source_ids.insert((binding.context.project_id, binding.source.source_id.clone())) {
                bail!(
                    "duplicate source id {} for project {}",
                    binding.source.source_id,
                    binding.context.project_id.0
                );
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
            let active_drift = store.active_schema_drift(&binding.source.source_id)?;
            drop(store);
            let fingerprint = binding
                .adapter
                .fingerprint(&binding.source)
                .map(|fingerprint| fingerprint.0);
            let (schema_fingerprint, fingerprint_error) = match fingerprint {
                Ok(fingerprint) => (Some(fingerprint), None),
                Err(error) => (None, Some(error.to_string())),
            };
            let last_error = active_drift
                .as_ref()
                .map(|drift| {
                    format!(
                        "schema drift [{}]: expected {}, observed {}",
                        drift.diagnostic_id, drift.expected_fingerprint, drift.observed_fingerprint
                    )
                })
                .or(fingerprint_error);
            health.register_source(
                binding.source_key(),
                binding.source.source_id.clone(),
                binding.source.path.clone(),
                binding.context.project_id,
                cursor.clone(),
                quarantined_count,
                capture_gaps,
                backlog_bytes(&binding.source.path, &cursor),
                schema_fingerprint,
                active_drift.map(|drift| drift.diagnostic_id),
                last_error,
            );
        }
        let source_locks = bindings
            .iter()
            .map(|binding| (binding.source_key(), Arc::new(tokio::sync::Mutex::new(()))))
            .collect();
        let mut storage_roots = bindings
            .iter()
            .filter_map(|binding| binding.ledger_path.parent().map(Path::to_path_buf))
            .collect::<Vec<_>>();
        storage_roots.sort();
        storage_roots.dedup();
        let (degradation_tx, _) = tokio::sync::watch::channel(crate::DegradationState::default());
        Ok(Self {
            bindings: Mutex::new(bindings),
            stores,
            source_locks: Mutex::new(source_locks),
            config,
            health: Mutex::new(health),
            pressure: Mutex::new(crate::PressureController::new(pressure_policy)?),
            disk_probe,
            storage_roots,
            degradation_tx,
        })
    }

    pub async fn capture_once(&self) -> Result<()> {
        if self.evaluate_pressure()? {
            return Ok(());
        }
        let bindings = self
            .bindings
            .lock()
            .map_err(|_| anyhow::anyhow!("capture binding lock is poisoned"))?
            .clone();
        for binding in &bindings {
            let source_key = binding.source_key();
            let source_lock = self
                .source_locks
                .lock()
                .map_err(|_| anyhow::anyhow!("source lock map is poisoned"))?
                .get(&source_key)
                .cloned()
                .expect("every binding has a source lock");
            let _source_guard = source_lock.lock().await;
            let store = self.store(binding.context.project_id)?;
            let cursor = store
                .lock()
                .map_err(|_| anyhow::anyhow!("event ledger lock is poisoned"))?
                .cursor(&binding.source.source_id)?;
            let fingerprint = match binding.adapter.fingerprint(&binding.source) {
                Ok(fingerprint) => fingerprint,
                Err(error) => {
                    self.record_error(&source_key, error.to_string())?;
                    continue;
                }
            };
            let fingerprint_value = fingerprint.0;
            self.health
                .lock()
                .map_err(|_| anyhow::anyhow!("service health lock is poisoned"))?
                .record_fingerprint(&source_key, fingerprint_value.clone());
            let active_drift = store
                .lock()
                .map_err(|_| anyhow::anyhow!("event ledger lock is poisoned"))?
                .active_schema_drift(&binding.source.source_id)?;
            if let Some(active_drift) = active_drift {
                if fingerprint_value == active_drift.expected_fingerprint {
                    store
                        .lock()
                        .map_err(|_| anyhow::anyhow!("event ledger lock is poisoned"))?
                        .resolve_schema_drift(
                            &binding.source.source_id,
                            time::OffsetDateTime::now_utc(),
                        )?;
                    self.health
                        .lock()
                        .map_err(|_| anyhow::anyhow!("service health lock is poisoned"))?
                        .record_schema_drift_resolved(&source_key);
                } else {
                    self.record_error(
                        &source_key,
                        format!(
                            "schema drift [{}]: expected {}, observed {}",
                            active_drift.diagnostic_id,
                            active_drift.expected_fingerprint,
                            fingerprint_value
                        ),
                    )?;
                    continue;
                }
            }
            let outcome = match binding.adapter.read_increment(&binding.source, &cursor) {
                Ok(outcome) => outcome,
                Err(error) => {
                    self.record_error(&source_key, error.to_string())?;
                    continue;
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
                    let capture_attribution = events.last().map(|event| {
                        (
                            event.harness.clone(),
                            brain_store::SessionAttribution::Attributed(
                                event.native_session_id.clone(),
                            ),
                        )
                    });
                    let last_event_id = events.last().map(|event| event.event_id);
                    let consolidation_reason = consolidation_reason(&events);
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
                        if backlog == 0 {
                            let (harness, session) =
                                capture_attribution.clone().unwrap_or_else(|| {
                                    (
                                        brain_domain::Harness::Other("capture".to_owned()),
                                        brain_store::SessionAttribution::Unattributed,
                                    )
                                });
                            let receipt = brain_store::LifecycleEvent {
                                event_id: uuid::Uuid::now_v7(),
                                project_id: binding.context.project_id,
                                harness,
                                session,
                                correlation_id: None,
                                channel: brain_store::LifecycleChannel::Capture,
                                stage: brain_store::LifecycleStage::CaptureCaughtUp,
                                occurred_at: observed_at,
                                detail: serde_json::json!({
                                    "source_id": binding.source.source_id,
                                    "backlog_bytes": 0,
                                    "source_may_grow": true,
                                }),
                            };
                            if let Err(error) = store.record_lifecycle_event(&receipt) {
                                tracing::warn!(%error, "could not record capture caught-up receipt");
                            }
                        }
                        if let (Some(last), Some(reason)) = (last_event_id, consolidation_reason) {
                            store.enqueue_through_event_job(last, reason)?;
                        } else {
                            store.enqueue_event_threshold_job(200)?;
                        }
                        let persisted_events = store.event_count()?;
                        let last_event_at = store.latest_event_at()?;
                        (result, persisted_events, last_event_at)
                    };
                    self.health
                        .lock()
                        .map_err(|_| anyhow::anyhow!("service health lock is poisoned"))?
                        .record_batch(BatchHealthUpdate {
                            source_key: source_key.clone(),
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
                        .record_check(&source_key, backlog_bytes(&binding.source.path, &cursor));
                }
                ReadOutcome::SchemaDrift(drift) => {
                    let record = SchemaDriftRecord {
                        diagnostic_id: uuid::Uuid::now_v7(),
                        source_id: binding.source.source_id.clone(),
                        expected_fingerprint: drift.expected.0,
                        observed_fingerprint: drift.observed.0,
                        cursor,
                        sample_hash: drift.sample_hash,
                        reason: "adapter schema differs from its reviewed profile".to_owned(),
                        observed_at: time::OffsetDateTime::now_utc(),
                        resolved_at: None,
                    };
                    store
                        .lock()
                        .map_err(|_| anyhow::anyhow!("event ledger lock is poisoned"))?
                        .record_schema_drift(&record)?;
                    self.health
                        .lock()
                        .map_err(|_| anyhow::anyhow!("service health lock is poisoned"))?
                        .record_schema_drift(
                            &source_key,
                            record.diagnostic_id,
                            format!(
                                "schema drift [{}]: expected {}, observed {}",
                                record.diagnostic_id,
                                record.expected_fingerprint,
                                record.observed_fingerprint
                            ),
                        );
                }
                ReadOutcome::SourceUnavailable(unavailable) => {
                    self.health
                        .lock()
                        .map_err(|_| anyhow::anyhow!("service health lock is poisoned"))?
                        .record_error(&source_key, unavailable.reason.clone());
                }
            }
        }
        let now = time::OffsetDateTime::now_utc();
        for store in self.stores.values() {
            store
                .lock()
                .map_err(|_| anyhow::anyhow!("event ledger lock is poisoned"))?
                .enqueue_inactivity_job(now, time::Duration::minutes(30))?;
        }
        Ok(())
    }

    fn evaluate_pressure(&self) -> Result<bool> {
        let sample = self
            .storage_roots
            .iter()
            .filter_map(|root| self.disk_probe.sample(root).ok())
            .min_by(|left, right| {
                left.available_percent()
                    .total_cmp(&right.available_percent())
                    .then_with(|| left.available_bytes.cmp(&right.available_bytes))
            });
        let mut pressure = self
            .pressure
            .lock()
            .map_err(|_| anyhow::anyhow!("pressure controller lock is poisoned"))?;
        let (state, error) = if let Some(sample) = sample {
            (pressure.evaluate(sample), None)
        } else if self.storage_roots.is_empty() {
            (pressure.state(), None)
        } else {
            (
                pressure.state(),
                Some("disk free-space probe failed for every ledger root".to_owned()),
            )
        };
        self.health
            .lock()
            .map_err(|_| anyhow::anyhow!("service health lock is poisoned"))?
            .record_disk(sample, state, error);
        self.degradation_tx.send_if_modified(|current| {
            if *current == state {
                false
            } else {
                *current = state;
                true
            }
        });
        Ok(state.capture_blocked)
    }

    pub fn degradation_receiver(&self) -> tokio::sync::watch::Receiver<crate::DegradationState> {
        self.degradation_tx.subscribe()
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

    pub fn activate_bindings(&self, candidates: Vec<CaptureBinding>) -> Result<usize> {
        let existing = self
            .bindings
            .lock()
            .map_err(|_| anyhow::anyhow!("capture binding lock is poisoned"))?
            .iter()
            .map(CaptureBinding::source_key)
            .collect::<std::collections::HashSet<_>>();
        let additions = candidates
            .into_iter()
            .filter(|binding| !existing.contains(&binding.source_key()))
            .collect::<Vec<_>>();

        for binding in &additions {
            let store = self.store(binding.context.project_id)?;
            let store = store
                .lock()
                .map_err(|_| anyhow::anyhow!("event ledger lock is poisoned"))?;
            let cursor = store.cursor(&binding.source.source_id)?;
            let quarantined_count = store.quarantine_count(&binding.source.source_id)?;
            let capture_gaps = store.unresolved_capture_gap_count(&binding.source.source_id)?;
            let active_drift = store.active_schema_drift(&binding.source.source_id)?;
            drop(store);
            let fingerprint = binding
                .adapter
                .fingerprint(&binding.source)
                .map(|fingerprint| fingerprint.0);
            let (schema_fingerprint, fingerprint_error) = match fingerprint {
                Ok(fingerprint) => (Some(fingerprint), None),
                Err(error) => (None, Some(error.to_string())),
            };
            let last_error = active_drift
                .as_ref()
                .map(|drift| {
                    format!(
                        "schema drift [{}]: expected {}, observed {}",
                        drift.diagnostic_id, drift.expected_fingerprint, drift.observed_fingerprint
                    )
                })
                .or(fingerprint_error);
            self.health
                .lock()
                .map_err(|_| anyhow::anyhow!("service health lock is poisoned"))?
                .register_source(
                    binding.source_key(),
                    binding.source.source_id.clone(),
                    binding.source.path.clone(),
                    binding.context.project_id,
                    cursor.clone(),
                    quarantined_count,
                    capture_gaps,
                    backlog_bytes(&binding.source.path, &cursor),
                    schema_fingerprint,
                    active_drift.map(|drift| drift.diagnostic_id),
                    last_error,
                );
        }

        let added = additions.len();
        if added > 0 {
            let mut source_locks = self
                .source_locks
                .lock()
                .map_err(|_| anyhow::anyhow!("source lock map is poisoned"))?;
            let mut bindings = self
                .bindings
                .lock()
                .map_err(|_| anyhow::anyhow!("capture binding lock is poisoned"))?;
            for binding in additions {
                source_locks.insert(binding.source_key(), Arc::new(tokio::sync::Mutex::new(())));
                bindings.push(binding);
            }
        }
        Ok(added)
    }

    fn record_error(&self, source_id: &str, error: String) -> Result<()> {
        self.health
            .lock()
            .map_err(|_| anyhow::anyhow!("service health lock is poisoned"))?
            .record_error(source_id, error);
        Ok(())
    }

    pub(crate) fn watched_paths(&self) -> Vec<PathBuf> {
        self.bindings
            .lock()
            .expect("capture binding lock is not poisoned")
            .iter()
            .map(|binding| binding.source.path.clone())
            .collect()
    }

    pub(crate) fn config(&self) -> CaptureServiceConfig {
        self.config
    }
}

fn consolidation_reason(events: &[brain_domain::NormalizedEvent]) -> Option<ConsolidationReason> {
    if events
        .iter()
        .any(|event| event.event_type == EventType::SessionEnded)
    {
        Some(ConsolidationReason::SessionStopped)
    } else if events
        .iter()
        .any(|event| event.event_type == EventType::SessionCompacted)
    {
        Some(ConsolidationReason::SessionCompacted)
    } else if events
        .iter()
        .any(|event| event.event_type == EventType::CheckpointAuthored)
    {
        Some(ConsolidationReason::ExplicitCheckpoint)
    } else {
        None
    }
}

fn backlog_bytes(path: &Path, cursor: &brain_domain::SourceCursor) -> u64 {
    std::fs::metadata(path)
        .map(|metadata| metadata.len().saturating_sub(cursor.byte_offset))
        .unwrap_or(0)
}
