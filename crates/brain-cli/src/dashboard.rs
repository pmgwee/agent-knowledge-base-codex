use std::path::{Path, PathBuf};

use anyhow::Result;
use brain_domain::{Harness, ProjectId};
use brain_service::{DiskProbe, FilesystemDiskProbe, ServiceLaunchConfig};

use crate::benchmark::directory_bytes;
use crate::config_panel::{ConfigDashboard, read_config_panel};
use crate::deployment::{DeploymentDashboard, read_deployment};
use crate::providers::provider_status;
use crate::status::read_status;

// ---------------------------------------------------------------------------
// Snapshot structs — the single JSON document the dashboard consumes
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, serde::Serialize)]
pub struct DashboardSnapshot {
    pub schema_version: u32,
    pub generated_at: time::OffsetDateTime,
    pub brain_home: PathBuf,
    pub service: ServiceDashboard,
    /// Whether the installed binaries match the source they were built from. Rendered
    /// separately from health: a stale deploy runs perfectly well, which is what makes it
    /// worth showing.
    pub deployment: DeploymentDashboard,
    pub projects: Vec<ProjectDashboard>,
    pub storage: StorageDashboard,
    pub health: HealthDashboard,
    /// What retrieval would do with a query right now.
    pub retrieval: RetrievalDashboard,
    /// What the brain is wired to do, and whether each wire reaches anything.
    pub config: ConfigDashboard,
}

/// How retrieval is actually configured, and which of its stages can run.
///
/// Every retrieval defect found in this project so far was invisible in exactly the same way: the
/// *configuration* looked right and the *capability* was absent. The vector channel silently off
/// because no model was installed; the reranker installed but not enabled; a channel present and
/// contributing nothing. A count of memories cannot distinguish any of those from a brain with
/// less to say, so this states each stage's weight next to whether it is actually running.
#[derive(Clone, Debug, serde::Serialize)]
pub struct RetrievalDashboard {
    /// The fusion method, in words, so the number below is never read as a similarity.
    pub fusion: String,
    pub configuration: brain_store::RetrievalConfiguration,
    pub channels: Vec<RetrievalChannel>,
    pub rerank: RerankState,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct RetrievalChannel {
    pub name: String,
    pub weight: f64,
    /// Whether this channel can contribute at all right now.
    pub available: bool,
    pub detail: String,
}

/// The cross-encoder stage: installed, and separately, on.
///
/// Two booleans rather than one because they fail differently and both silently. Not installed
/// means no checkpoint on disk. Installed but off is the *normal* state — it costs ~90 ms per
/// candidate, so callers opt in per query — and a reader who sees only "installed" would reasonably
/// conclude their searches were being re-ranked when none of them were.
#[derive(Clone, Debug, serde::Serialize)]
pub struct RerankState {
    pub installed: bool,
    pub on_by_default: bool,
    pub depth: usize,
    pub measured_ms_per_candidate: u32,
    pub detail: String,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct ServiceDashboard {
    pub installed: bool,
    pub tasks: Vec<TaskStatus>,
    pub binaries_present: BinariesPresent,
    pub backup_root: Option<PathBuf>,
    pub drill_root: Option<PathBuf>,
    pub recovery_command: String,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct TaskStatus {
    pub name: String,
    pub installed: bool,
    pub running: bool,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct BinariesPresent {
    pub service: bool,
    pub brain: bool,
    pub hook: Option<bool>,
    pub mcp: Option<bool>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct ProjectDashboard {
    pub project_id: ProjectId,
    pub display_name: String,
    pub healthy: bool,
    pub persisted_events: u64,
    pub events_by_harness: HarnessCounts,
    pub latest_event_at: Option<time::OffsetDateTime>,
    pub backlog_bytes: u64,
    pub quarantined_records: u64,
    pub unresolved_capture_gaps: u64,
    pub active_schema_drifts: u64,
    pub memory_records: u64,
    /// The consolidation queue. Memories arriving slowly and a project where less happened look
    /// identical from outside, and only this tells them apart.
    pub consolidation: brain_store::ConsolidationQueue,
    /// Memory lifecycle: what retrieval has reached, and what has gone quiet.
    pub lifecycle: LifecycleSummary,
    /// How far the vector index has got. Retrieval quality depends on it while it is filling,
    /// and a half-built index is indistinguishable from a complete one that simply misses things
    /// unless something says so.
    pub embeddings: EmbeddingCoverage,
    pub ledger_bytes: u64,
    pub providers: Vec<ProviderState>,
    pub deliveries_today: DeliverySummary,
    pub deliveries_7d: DeliverySummary,
    pub deliveries_30d: DeliverySummary,
}

/// What the brain's memories are doing, as opposed to how many there are.
///
/// A count of memories says the brain is growing. These say whether it is being *used* — the
/// distinction between remembering and merely storing, and the one nothing on the dashboard could
/// previously make.
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct LifecycleSummary {
    /// Retrieval has never returned these.
    pub never_retrieved: u64,
    /// Old *and* never retrieved. Marked in the vault, demoted in the orientation, never deleted.
    pub stale: u64,
    /// Sharing evidence with no other memory, so no wikilink reaches them.
    pub unlinked: u64,
    /// Deliberately withdrawn.
    pub withdrawn: u64,
}

/// Vector index coverage for one project.
///
/// `model_installed` is separate from the counts on purpose. Zero embedded with no model is a
/// brain working exactly as designed on keyword search; zero embedded *with* a model is a
/// backfill that is not running, and the two look identical in a count alone.
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct EmbeddingCoverage {
    pub model_installed: bool,
    pub memories_embedded: u64,
    pub memories_remaining: u64,
    pub events_embedded: u64,
    pub events_remaining: u64,
}

#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct HarnessCounts {
    pub claude_code: u64,
    pub codex: u64,
    pub hermes: u64,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct ProviderState {
    pub provider: String,
    pub enabled: bool,
    pub usable: bool,
    pub detail: String,
}

#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct DeliverySummary {
    pub deliveries: u64,
    pub total_tokens: u64,
    pub max_tokens: u64,
    pub mean_tokens: f64,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct StorageDashboard {
    pub brain_home_bytes: u64,
    pub backup_root_bytes: Option<u64>,
    pub brain_drive: DiskInfo,
    pub backup_drive: Option<DiskInfo>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct DiskInfo {
    pub drive: String,
    pub available_bytes: u64,
    pub total_bytes: u64,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct HealthDashboard {
    pub healthy_projects: usize,
    pub unhealthy_projects: usize,
    pub capture_blocked: bool,
    pub service_running: bool,
    pub any_binary_missing: bool,
}

// ---------------------------------------------------------------------------
// Aggregator — composes existing readers into one snapshot
// ---------------------------------------------------------------------------

pub fn read_dashboard(brain_home: &Path) -> Result<DashboardSnapshot> {
    let now = time::OffsetDateTime::now_utc();
    let config = ServiceLaunchConfig::load(ServiceLaunchConfig::default_path(brain_home))?;
    let probe = FilesystemDiskProbe;

    // Service state (global, once)
    let service_report = crate::service::windows_service_status(brain_home)?;
    let manifest = crate::service::load_manifest(brain_home).ok();
    let tasks: Vec<TaskStatus> = service_report
        .tasks
        .iter()
        .map(|t| TaskStatus {
            name: t.task_name.clone(),
            installed: t.installed,
            running: t
                .detail
                .as_deref()
                .map(|d| d.contains("Status:") && d.contains("Running"))
                .unwrap_or(false),
        })
        .collect();

    let (backup_root, drill_root) = manifest
        .as_ref()
        .map(|m| (Some(m.backup_root.clone()), Some(m.drill_root.clone())))
        .unwrap_or((None, None));

    let binaries_present = check_binaries(&manifest, brain_home);
    let recovery_command = format_recovery_command(brain_home, &backup_root, &drill_root);

    let config_panel = read_config_panel(
        &config,
        &tasks
            .iter()
            .map(|task| (task.name.clone(), task.installed))
            .collect::<Vec<_>>(),
    )?;

    let service_running = tasks.first().map(|t| t.running).unwrap_or(false);
    let any_binary_missing = !binaries_present.service || !binaries_present.brain;

    // Per-project data
    let mut projects = Vec::with_capacity(config.projects.len());
    let mut healthy_count = 0usize;
    let mut unhealthy_count = 0usize;

    for project_config in &config.projects {
        let project_id = project_config.project_id;
        let display_name = project_config
            .project_root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_owned();

        // Delegate to the existing status reader for the standard health fields
        let status = match read_status(brain_home, Some(project_id)) {
            Ok(s) => s,
            Err(_) => {
                unhealthy_count += 1;
                continue;
            }
        };

        if status.healthy {
            healthy_count += 1;
        } else {
            unhealthy_count += 1;
        }

        // Open the ledger directly for harness counts and delivery summaries
        let ledger = brain_store::EventLedger::open(&project_config.ledger_path, project_id)?;
        let events_by_harness = HarnessCounts {
            claude_code: ledger.event_count_by_harness(Harness::ClaudeCode)?,
            codex: ledger.event_count_by_harness(Harness::Codex)?,
            hermes: ledger.event_count_by_harness(Harness::Hermes)?,
        };

        let memory_records = ledger.memory_count()?;
        let consolidation = ledger.consolidation_queue()?;
        let lifecycle = LifecycleSummary {
            never_retrieved: ledger.never_retrieved_memory_count().unwrap_or(0),
            stale: ledger
                .stale_memory_ids(now, time::Duration::days(90))
                .map(|ids| ids.len() as u64)
                .unwrap_or(0),
            unlinked: ledger.memories_without_shared_evidence().unwrap_or(0) as u64,
            withdrawn: ledger.tombstones().map(|t| t.len() as u64).unwrap_or(0),
        };
        let (memories_embedded, memories_remaining) = ledger.embedding_coverage()?;
        let (events_embedded, events_remaining) = ledger.event_embedding_coverage()?;
        let embeddings = EmbeddingCoverage {
            model_installed: brain_store::default_model_dir(brain_home).is_dir(),
            memories_embedded,
            memories_remaining,
            events_embedded,
            events_remaining,
        };
        let ledger_bytes = directory_bytes(
            project_config
                .ledger_path
                .parent()
                .unwrap_or(Path::new(".")),
        )
        .unwrap_or(0);

        // Delivery windows
        let today_start = start_of_today_local(now);
        let deliveries_today = delivery_summary(&ledger, today_start);
        let deliveries_7d = delivery_summary(&ledger, now - time::Duration::days(7));
        let deliveries_30d = delivery_summary(&ledger, now - time::Duration::days(30));

        // Providers
        let providers = provider_status(brain_home, &project_id.0.to_string())
            .unwrap_or_default()
            .into_iter()
            .map(|p| ProviderState {
                provider: format!("{:?}", p.provider).to_lowercase(),
                enabled: p.enabled,
                usable: p.usable,
                detail: p.detail,
            })
            .collect();

        projects.push(ProjectDashboard {
            project_id,
            display_name,
            healthy: status.healthy,
            persisted_events: status.persisted_events,
            events_by_harness,
            latest_event_at: status.last_event_at,
            backlog_bytes: status.backlog_bytes,
            quarantined_records: status.quarantined_records,
            unresolved_capture_gaps: status.unresolved_capture_gaps,
            active_schema_drifts: status.active_schema_drifts,
            memory_records,
            lifecycle,
            consolidation,
            embeddings,
            ledger_bytes,
            providers,
            deliveries_today,
            deliveries_7d,
            deliveries_30d,
        });
    }

    // Storage
    let brain_home_bytes = directory_bytes(brain_home).unwrap_or(0);
    let backup_root_bytes = backup_root
        .as_ref()
        .map(|r| directory_bytes(r).unwrap_or(0));
    let brain_drive = disk_info(brain_home, &probe);
    let backup_drive = backup_root.as_ref().map(|r| disk_info(r, &probe));

    let capture_blocked = !service_running || any_binary_missing;

    let retrieval = read_retrieval(brain_home);

    Ok(DashboardSnapshot {
        schema_version: 1,
        generated_at: now,
        brain_home: brain_home.to_path_buf(),
        service: ServiceDashboard {
            installed: service_report.installed,
            tasks,
            binaries_present,
            backup_root,
            drill_root,
            recovery_command,
        },
        deployment: read_deployment(brain_home),
        projects,
        retrieval,
        config: config_panel,
        storage: StorageDashboard {
            brain_home_bytes,
            backup_root_bytes,
            brain_drive,
            backup_drive,
        },
        health: HealthDashboard {
            healthy_projects: healthy_count,
            unhealthy_projects: unhealthy_count,
            capture_blocked,
            service_running,
            any_binary_missing,
        },
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn delivery_summary(
    ledger: &brain_store::EventLedger,
    since: time::OffsetDateTime,
) -> DeliverySummary {
    match ledger.context_delivery_summary(since) {
        Ok(s) => DeliverySummary {
            deliveries: s.deliveries,
            total_tokens: s.total_tokens,
            max_tokens: s.max_tokens,
            mean_tokens: s.mean_tokens(),
        },
        Err(_) => DeliverySummary::default(),
    }
}

fn start_of_today_local(now: time::OffsetDateTime) -> time::OffsetDateTime {
    // Best-effort midnight in UTC; the dashboard shows "today" which is close enough
    // for a single-user system. A timezone-aware version would need the user's offset.
    let date = now.date();
    time::Date::from_calendar_date(date.year(), date.month(), 1)
        .ok()
        .and_then(|_| time::Date::from_calendar_date(date.year(), date.month(), date.day()).ok())
        .map(|d| d.with_hms(0, 0, 0).unwrap().assume_utc())
        .unwrap_or(now - time::Duration::hours(24))
}

fn disk_info(path: &Path, probe: &FilesystemDiskProbe) -> DiskInfo {
    match probe.sample(path) {
        Ok(sample) => DiskInfo {
            drive: path
                .components()
                .next()
                .and_then(|c| c.as_os_str().to_str())
                .unwrap_or("?")
                .to_owned(),
            available_bytes: sample.available_bytes,
            total_bytes: sample.total_bytes,
        },
        Err(_) => DiskInfo {
            drive: "?".to_owned(),
            available_bytes: 0,
            total_bytes: 0,
        },
    }
}

fn check_binaries(
    manifest: &Option<crate::service::InstallManifest>,
    brain_home: &Path,
) -> BinariesPresent {
    // If there's an install manifest, check those exact paths.
    // Otherwise, check the stable bin directory.
    if let Some(m) = manifest {
        BinariesPresent {
            service: m.service_executable.is_file(),
            brain: m.brain_executable.is_file(),
            hook: m.hook_executable.as_ref().map(|p| p.is_file()),
            mcp: None,
        }
    } else {
        let bin = brain_home.join("bin");
        BinariesPresent {
            service: bin.join("brain-service.exe").is_file(),
            brain: bin.join("brain.exe").is_file(),
            hook: Some(bin.join("brain-hook.exe").is_file()),
            mcp: Some(bin.join("brain-mcp.exe").is_file()),
        }
    }
}

fn format_recovery_command(
    brain_home: &Path,
    backup_root: &Option<PathBuf>,
    drill_root: &Option<PathBuf>,
) -> String {
    let bin = brain_home.join("bin");
    let brain_exe = bin.join("brain.exe");
    let backup = backup_root
        .as_ref()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| "D:\\AgentBrainBackups".to_owned());
    let drill = drill_root
        .as_ref()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| format!("{}\\AgentBrainDrills", env!("USERPROFILE")));
    let mut cmd = String::new();
    cmd.push_str(&format!(
        "& \"{}\" --brain-home \"{}\" service install `\n",
        brain_exe.display(),
        brain_home.display()
    ));
    cmd.push_str(&format!(
        "  --service-executable \"{}\\brain-service.exe\" `\n",
        bin.display()
    ));
    cmd.push_str(&format!(
        "  --brain-executable \"{}\" `\n",
        brain_exe.display()
    ));
    cmd.push_str(&format!(
        "  --backup-root \"{backup}\" --drill-root \"{drill}\" `\n"
    ));
    cmd.push_str(&format!(
        "  --install-hooks --hook-executable \"{}\\brain-hook.exe\"",
        bin.display()
    ));
    cmd
}

/// Report retrieval's live shape: the weights, and which stages can actually fire.
///
/// Availability is decided by what is on disk, not by what the code supports, because those are the
/// two things that diverge. A brain with no `all-MiniLM-L6-v2` searches by keyword exactly as
/// designed — that is not a fault, and the panel says so rather than showing a red mark — but a
/// brain that *has* the model and is somehow not using it is a real defect, and only stating both
/// halves makes the two distinguishable.
fn read_retrieval(brain_home: &Path) -> RetrievalDashboard {
    let configuration = brain_store::retrieval_configuration();
    let model_installed = brain_store::default_model_dir(brain_home).is_dir();
    let reranker_installed = brain_store::default_reranker_dir(brain_home)
        .join("model.safetensors")
        .is_file();

    let channels = vec![
        RetrievalChannel {
            name: "BM25 (events)".to_owned(),
            weight: configuration.bm25_weight,
            available: true,
            detail: "FTS5 over captured turns. Always available.".to_owned(),
        },
        RetrievalChannel {
            name: "BM25 (memories)".to_owned(),
            weight: configuration.bm25_weight,
            available: true,
            // Worth stating on the panel rather than only in a commit message: merging these two
            // and sorting by raw score let the larger corpus take every slot, and no memory was
            // reachable at all while it looked like working code.
            detail: "A separate channel from events — different corpus, incomparable scores."
                .to_owned(),
        },
        RetrievalChannel {
            name: "Vector".to_owned(),
            weight: configuration.vector_weight,
            available: model_installed,
            detail: if model_installed {
                "all-MiniLM-L6-v2, 384-dim cosine.".to_owned()
            } else {
                "No model installed — retrieval is keyword-only, which is a supported state."
                    .to_owned()
            },
        },
        RetrievalChannel {
            name: "Graph".to_owned(),
            weight: configuration.graph_weight,
            available: true,
            detail: "One hop across shared evidence. Needs no model.".to_owned(),
        },
    ];

    RetrievalDashboard {
        fusion: format!(
            "Reciprocal Rank Fusion, k={}. Fused by rank — never by score.",
            configuration.rrf_k
        ),
        channels,
        rerank: RerankState {
            installed: reranker_installed,
            // Never on by default, and this is deliberate rather than pending. See the measurement
            // in `crates/brain-store/tests/reranker_model.rs`.
            on_by_default: false,
            depth: configuration.rerank_depth,
            measured_ms_per_candidate: 90,
            detail: if reranker_installed {
                "ms-marco-MiniLM-L6-v2, opt-in per query. Sharpens ordering among candidates that \
                 share the question's vocabulary; measured not to bridge a vocabulary gap."
                    .to_owned()
            } else {
                "No cross-encoder installed — the fused order stands.".to_owned()
            },
        },
        configuration,
    }
}
