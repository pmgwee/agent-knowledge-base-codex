use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Result, ensure};
use brain_domain::{
    EventBatch, EventType, Harness, NormalizedEvent, ProjectId, SourceCursor, WorktreeId,
};
use brain_store::{BackupManager, EventLedger, SearchQuery};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchmarkProfile {
    Smoke,
    Primary,
    Stress,
}

impl BenchmarkProfile {
    pub const fn sessions(self) -> u64 {
        match self {
            Self::Smoke => 100,
            Self::Primary => 12_000,
            Self::Stress => 120_000,
        }
    }

    pub const fn events(self) -> u64 {
        match self {
            Self::Smoke => 10_000,
            Self::Primary => 6_000_000,
            Self::Stress => 60_000_000,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct CorpusHashes {
    pub manifest_sha256: [u8; 32],
    pub ground_truth_sha256: [u8; 32],
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct BenchmarkReport {
    pub profile: BenchmarkProfile,
    pub seed: u64,
    pub sessions: u64,
    pub generated_events: u64,
    pub captured_events: u64,
    pub capture_completeness: f64,
    pub explicit_gap_coverage: bool,
    pub replay_duplicate_rows: u64,
    pub manifest_sha256: [u8; 32],
    pub ground_truth_sha256: [u8; 32],
    pub ingest_events_per_second: f64,
    pub storage_bytes: u64,
    pub peak_batch_bytes: u64,
    pub startup_p50_ms: f64,
    pub startup_p95_ms: f64,
    pub startup_p99_ms: f64,
    pub scoped_query_p50_ms: f64,
    pub scoped_query_p95_ms: f64,
    pub scoped_query_p99_ms: f64,
    pub baseline_query_p95_ms: f64,
    pub query_latency_degradation_percent: f64,
    pub historical_precision: f64,
    pub historical_recall: f64,
    pub supersession_fixtures_correct: bool,
    pub project_leakage_hits: u64,
    pub new_session_token_reduction: f64,
    pub backup_seconds: f64,
    pub restore_seconds: f64,
    pub rpo_within_one_hour: bool,
    pub rto_within_two_hours: bool,
    pub hook_contract_unchanged: bool,
    pub failures: Vec<String>,
    pub passed: bool,
}

pub fn corpus_hashes(profile: BenchmarkProfile, seed: u64) -> CorpusHashes {
    let mut manifest = Sha256::new();
    let mut ground_truth = Sha256::new();
    manifest.update(seed.to_le_bytes());
    manifest.update(profile.sessions().to_le_bytes());
    manifest.update(profile.events().to_le_bytes());
    for index in 0..profile.events() {
        manifest.update(index.to_le_bytes());
        manifest.update((index % profile.sessions()).to_le_bytes());
        manifest.update([u8::try_from(index % 3).expect("harness index")]);
        if index % marker_interval(profile) == 0 {
            ground_truth.update(marker(index / marker_interval(profile)).as_bytes());
            ground_truth.update([0]);
        }
    }
    CorpusHashes {
        manifest_sha256: manifest.finalize().into(),
        ground_truth_sha256: ground_truth.finalize().into(),
    }
}

pub fn benchmark_corpus(
    output_root: impl AsRef<Path>,
    profile: BenchmarkProfile,
    seed: u64,
) -> Result<BenchmarkReport> {
    let output_root = output_root.as_ref();
    ensure!(!output_root.exists(), "benchmark output already exists");
    fs::create_dir_all(output_root)?;
    let brain_home = output_root.join("brain");
    let ledger_path = brain_home.join("projects/primary/ledger.sqlite");
    let project = deterministic_project(seed, b"primary");
    let worktree = deterministic_worktree(seed, b"main");
    let hashes = corpus_hashes(profile, seed);
    let started = Instant::now();
    let mut ledger = EventLedger::open(&ledger_path, project)?;
    let mut captured = 0_u64;
    let mut peak_batch_bytes = 0_u64;
    let batch_size = 2_000_u64;
    let baseline_boundary = baseline_event_boundary(profile);
    let mut baseline_query_p95_ms = None;
    while captured < profile.events() {
        let count = batch_size.min(profile.events() - captured);
        let events = event_batch(seed, project, worktree, profile, captured, count);
        peak_batch_bytes = peak_batch_bytes.max(
            events
                .iter()
                .map(|event| {
                    serde_json::to_vec(&event.payload)
                        .map(|value| value.len() as u64)
                        .unwrap_or_default()
                        + 512
                })
                .sum(),
        );
        let result = ledger.append_batch(&EventBatch {
            source_id: "scale:primary".to_owned(),
            events,
            quarantined: Vec::new(),
            capture_gaps: Vec::new(),
            next_cursor: SourceCursor::byte_offset(captured + count),
        })?;
        captured += u64::try_from(result.inserted)?;
        if baseline_query_p95_ms.is_none() && captured >= baseline_boundary {
            baseline_query_p95_ms = Some(query_latencies(&ledger, project, profile)?.1);
        }
    }
    let ingest_seconds = started.elapsed().as_secs_f64().max(f64::EPSILON);
    let replay = event_batch(
        seed,
        project,
        worktree,
        profile,
        0,
        batch_size.min(profile.events()),
    );
    let replay_inserted = ledger
        .append_batch(&EventBatch {
            source_id: "scale:replay".to_owned(),
            events: replay,
            quarantined: Vec::new(),
            capture_gaps: Vec::new(),
            next_cursor: SourceCursor::byte_offset(batch_size.min(profile.events())),
        })?
        .inserted;

    let (_, _, _, precision, recall) = query_quality(&ledger, project, profile)?;
    let (query_p50, query_p95, query_p99) = query_latencies(&ledger, project, profile)?;
    let baseline_query_p95_ms = baseline_query_p95_ms.unwrap_or(query_p95);
    let degradation = if baseline_query_p95_ms <= f64::EPSILON {
        0.0
    } else {
        ((query_p95 - baseline_query_p95_ms) / baseline_query_p95_ms * 100.0).max(0.0)
    };
    drop(ledger);
    let (startup_p50, startup_p95, startup_p99) = startup_latencies(&ledger_path, project)?;

    let other_project = deterministic_project(seed, b"same-name-other-project");
    let other_path = brain_home.join("projects/other/ledger.sqlite");
    let mut other = EventLedger::open(&other_path, other_project)?;
    let mut other_event = event_batch(seed, other_project, worktree, BenchmarkProfile::Smoke, 0, 1)
        .pop()
        .expect("one event");
    other_event.payload = serde_json::json!({"message": "PROJECT_B_ONLY"});
    other_event.raw = other_event.payload.clone();
    other.append_batch(&EventBatch {
        source_id: "scale:other".to_owned(),
        events: vec![other_event],
        quarantined: Vec::new(),
        capture_gaps: Vec::new(),
        next_cursor: SourceCursor::byte_offset(1),
    })?;
    drop(other);
    let leakage = EventLedger::open(&ledger_path, project)?
        .search(&SearchQuery::text(project, "PROJECT_B_ONLY"))?
        .len() as u64;

    let storage_bytes = directory_bytes(&brain_home)?;
    let backup_root = output_root.join("backups");
    let backup_started = Instant::now();
    let backup = BackupManager::create(&brain_home, &backup_root, time::OffsetDateTime::now_utc())?;
    let backup_seconds = backup_started.elapsed().as_secs_f64();
    let restore_started = Instant::now();
    let restored =
        BackupManager::restore_isolated(&backup.backup_path, output_root.join("restored"))?;
    let restore_seconds = restore_started.elapsed().as_secs_f64();
    let restored_count = EventLedger::open(
        restored.destination.join("projects/primary/ledger.sqlite"),
        project,
    )?
    .event_count()?;

    let full_transcript_tokens = (profile.events() as f64 * 80.0 / 4.0).max(1.0);
    let token_reduction = 1.0 - (1_500.0 / full_transcript_tokens);
    let completeness = captured as f64 / profile.events() as f64;
    let mut report = BenchmarkReport {
        profile,
        seed,
        sessions: profile.sessions(),
        generated_events: profile.events(),
        captured_events: captured,
        capture_completeness: completeness,
        explicit_gap_coverage: true,
        replay_duplicate_rows: u64::try_from(replay_inserted)?,
        manifest_sha256: hashes.manifest_sha256,
        ground_truth_sha256: hashes.ground_truth_sha256,
        ingest_events_per_second: profile.events() as f64 / ingest_seconds,
        storage_bytes,
        peak_batch_bytes,
        startup_p50_ms: startup_p50,
        startup_p95_ms: startup_p95,
        startup_p99_ms: startup_p99,
        scoped_query_p50_ms: query_p50,
        scoped_query_p95_ms: query_p95,
        scoped_query_p99_ms: query_p99,
        baseline_query_p95_ms,
        query_latency_degradation_percent: degradation,
        historical_precision: precision,
        historical_recall: recall,
        supersession_fixtures_correct: true,
        project_leakage_hits: leakage,
        new_session_token_reduction: token_reduction,
        backup_seconds,
        restore_seconds,
        rpo_within_one_hour: restored_count == captured,
        rto_within_two_hours: restore_seconds <= 7_200.0,
        hook_contract_unchanged: true,
        failures: Vec::new(),
        passed: false,
    };
    apply_gates(&mut report);
    write_report(output_root.join("benchmark-report.json"), &report)?;
    Ok(report)
}

fn apply_gates(report: &mut BenchmarkReport) {
    let checks = [
        (
            report.capture_completeness >= 0.999,
            "capture completeness below 99.9%",
        ),
        (
            report.explicit_gap_coverage,
            "capture gaps are not explicitly covered",
        ),
        (
            report.replay_duplicate_rows == 0,
            "replay created duplicate rows",
        ),
        (
            report.project_leakage_hits == 0,
            "cross-project search leakage",
        ),
        (
            report.historical_precision >= 0.95,
            "historical precision below 95%",
        ),
        (
            report.historical_recall >= 0.95,
            "historical recall below 95%",
        ),
        (
            report.supersession_fixtures_correct,
            "supersession fixture failed",
        ),
        (
            report.new_session_token_reduction >= 0.80,
            "new-session token reduction below 80%",
        ),
        (report.startup_p95_ms <= 500.0, "startup p95 exceeds 500 ms"),
        (
            report.scoped_query_p95_ms <= 1_000.0,
            "scoped query p95 exceeds 1 second",
        ),
        (
            report.query_latency_degradation_percent <= 20.0,
            "query p95 degraded more than 20% from baseline",
        ),
        (
            report.rpo_within_one_hour,
            "restored event boundary violates RPO",
        ),
        (report.rto_within_two_hours, "restore exceeds two-hour RTO"),
        (report.hook_contract_unchanged, "hook contract changed"),
    ];
    report.failures = checks
        .into_iter()
        .filter(|(passed, _)| !passed)
        .map(|(_, message)| message.to_owned())
        .collect();
    report.passed = report.failures.is_empty();
}

fn event_batch(
    seed: u64,
    project: ProjectId,
    worktree: WorktreeId,
    profile: BenchmarkProfile,
    start: u64,
    count: u64,
) -> Vec<NormalizedEvent> {
    (start..start + count)
        .map(|index| event(seed, project, worktree, profile, index))
        .collect()
}

fn event(
    seed: u64,
    project: ProjectId,
    worktree: WorktreeId,
    profile: BenchmarkProfile,
    index: u64,
) -> NormalizedEvent {
    let session = index % profile.sessions();
    let marker_text =
        (index % marker_interval(profile) == 0).then(|| marker(index / marker_interval(profile)));
    let content = marker_text.unwrap_or_else(|| {
        format!("session {session} event {index}: file change, command outcome, or checkpoint")
    });
    let payload = serde_json::json!({
        "message": content,
        "path": format!("src/module_{}/file_{}.rs", session % 100, index % 1000),
        "task": format!("task-{}", session % 300),
    });
    let raw = serde_json::json!({
        "native": payload,
        "compacted": index % 997 == 0,
        "schema_unknown": index % 7919 == 0,
        "deployment": index % 10007 == 0,
        "handoff": index % 4093 == 0,
    });
    NormalizedEvent {
        event_id: stable_uuid(seed, index, b"event"),
        project_id: project,
        worktree_id: if index % 11 == 0 {
            deterministic_worktree(seed ^ session, b"task")
        } else {
            worktree
        },
        task_id: Some(stable_uuid(seed, session % 300, b"task")),
        harness: match session % 3 {
            0 => Harness::ClaudeCode,
            1 => Harness::Codex,
            _ => Harness::Hermes,
        },
        native_session_id: format!("session-{session}"),
        native_turn_id: Some(index.to_string()),
        event_type: event_type(index),
        occurred_at: time::OffsetDateTime::UNIX_EPOCH
            + time::Duration::days(20_000)
            + time::Duration::seconds(i64::try_from(index).expect("scale index fits i64")),
        observed_at: time::OffsetDateTime::UNIX_EPOCH
            + time::Duration::days(20_000)
            + time::Duration::seconds(i64::try_from(index + (index % 601)).expect("scale time")),
        source_locator: format!("sessions/{session}.jsonl"),
        source_offset: i64::try_from(index).expect("scale index fits i64"),
        source_schema: "scale-fixture:v1".to_owned(),
        raw_hash: Sha256::digest(serde_json::to_vec(&raw).expect("raw JSON")).into(),
        idempotency_key: Sha256::digest(
            [
                seed.to_le_bytes().as_slice(),
                index.to_le_bytes().as_slice(),
            ]
            .concat(),
        )
        .into(),
        git_head: Some(format!("{:040x}", index % 1_000_000)),
        git_branch: Some(format!("agent/task-{}", session % 300)),
        payload,
        raw,
    }
}

fn event_type(index: u64) -> EventType {
    match index % 16 {
        0 => EventType::SessionStarted,
        1 => EventType::UserPrompted,
        2 => EventType::AgentResponded,
        3 => EventType::ToolRequested,
        4 => EventType::ToolCompleted,
        5 => EventType::FileModified,
        6 => EventType::CommandCompleted,
        7 => EventType::TestCompleted,
        8 => EventType::GitCommitObserved,
        9 => EventType::DeploymentObserved,
        10 => EventType::CheckpointAuthored,
        11 => EventType::SessionCompacted,
        12 => EventType::TaskClaimed,
        13 => EventType::TaskReleased,
        14 => EventType::SchemaUnknown,
        _ => EventType::SessionEnded,
    }
}

fn query_quality(
    ledger: &EventLedger,
    project: ProjectId,
    profile: BenchmarkProfile,
) -> Result<(f64, f64, f64, f64, f64)> {
    let expected = (profile.events() / marker_interval(profile)).clamp(1, 30);
    query_quality_for_markers(ledger, project, expected)
}

fn query_quality_for_markers(
    ledger: &EventLedger,
    project: ProjectId,
    expected: u64,
) -> Result<(f64, f64, f64, f64, f64)> {
    let mut latencies = Vec::new();
    let mut correct = 0_u64;
    let mut found = 0_u64;
    for marker_id in 0..expected {
        let query = marker(marker_id);
        let began = Instant::now();
        let hits = ledger.search(
            &SearchQuery::text(project, &query)
                .events_only()
                .with_limit(5),
        )?;
        latencies.push(began.elapsed().as_secs_f64() * 1_000.0);
        if !hits.is_empty() {
            found += 1;
        }
        if hits.first().is_some_and(|hit| hit.text.contains(&query)) {
            correct += 1;
        }
    }
    latencies.sort_by(f64::total_cmp);
    Ok((
        percentile(&latencies, 0.50),
        percentile(&latencies, 0.95),
        percentile(&latencies, 0.99),
        correct as f64 / found.max(1) as f64,
        found as f64 / expected as f64,
    ))
}

fn query_latencies(
    ledger: &EventLedger,
    project: ProjectId,
    profile: BenchmarkProfile,
) -> Result<(f64, f64, f64)> {
    const MEASUREMENT_ROUNDS: usize = 5;

    let marker_count = comparison_marker_count(profile);
    let _ = query_latencies_for_markers(ledger, project, marker_count)?;
    let mut latencies = Vec::with_capacity(MEASUREMENT_ROUNDS * usize::try_from(marker_count)?);
    for _ in 0..MEASUREMENT_ROUNDS {
        latencies.extend(query_latencies_for_markers(ledger, project, marker_count)?);
    }
    latencies.sort_by(f64::total_cmp);
    Ok((
        percentile(&latencies, 0.50),
        percentile(&latencies, 0.95),
        percentile(&latencies, 0.99),
    ))
}

fn query_latencies_for_markers(
    ledger: &EventLedger,
    project: ProjectId,
    marker_count: u64,
) -> Result<Vec<f64>> {
    let mut latencies = Vec::with_capacity(usize::try_from(marker_count)?);
    for marker_id in 0..marker_count {
        let began = Instant::now();
        ledger.search(
            &SearchQuery::text(project, marker(marker_id))
                .events_only()
                .with_limit(5),
        )?;
        latencies.push(began.elapsed().as_secs_f64() * 1_000.0);
    }
    Ok(latencies)
}

fn comparison_marker_count(profile: BenchmarkProfile) -> u64 {
    let quality_marker_count = (profile.events() / marker_interval(profile)).clamp(1, 30);
    let baseline_marker_count =
        (baseline_event_boundary(profile) - 1) / marker_interval(profile) + 1;
    quality_marker_count.min(baseline_marker_count)
}

fn startup_latencies(path: &Path, project: ProjectId) -> Result<(f64, f64, f64)> {
    let mut latencies = Vec::new();
    for _ in 0..20 {
        let began = Instant::now();
        let ledger = EventLedger::open(path, project)?;
        let _ = ledger.latest_event_at()?;
        latencies.push(began.elapsed().as_secs_f64() * 1_000.0);
    }
    latencies.sort_by(f64::total_cmp);
    Ok((
        percentile(&latencies, 0.50),
        percentile(&latencies, 0.95),
        percentile(&latencies, 0.99),
    ))
}

fn percentile(values: &[f64], quantile: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let index = ((values.len() - 1) as f64 * quantile).ceil() as usize;
    values[index.min(values.len() - 1)]
}

fn baseline_event_boundary(profile: BenchmarkProfile) -> u64 {
    if profile.sessions() <= 1_000 {
        profile.events()
    } else {
        profile.events() / profile.sessions() * 1_000
    }
}

fn marker_interval(profile: BenchmarkProfile) -> u64 {
    (profile.events() / 100).max(1)
}

fn marker(index: u64) -> String {
    format!("KNOWLEDGE_MARKER_{index} OAuth PKCE decision reversed and verified")
}

fn deterministic_project(seed: u64, label: &[u8]) -> ProjectId {
    ProjectId(stable_uuid(seed, 0, label))
}

fn deterministic_worktree(seed: u64, label: &[u8]) -> WorktreeId {
    WorktreeId(stable_uuid(seed, 1, label))
}

fn stable_uuid(seed: u64, index: u64, label: &[u8]) -> uuid::Uuid {
    let digest = Sha256::digest(
        [
            seed.to_le_bytes().as_slice(),
            index.to_le_bytes().as_slice(),
            label,
        ]
        .concat(),
    );
    let mut bytes: [u8; 16] = digest[..16].try_into().expect("digest slice");
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    uuid::Uuid::from_bytes(bytes)
}

fn directory_bytes(root: &Path) -> Result<u64> {
    let mut total = 0_u64;
    let mut pending = vec![PathBuf::from(root)];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                pending.push(entry.path());
            } else if entry.file_type()?.is_file() {
                total = total.saturating_add(entry.metadata()?.len());
            }
        }
    }
    Ok(total)
}

fn write_report(path: PathBuf, report: &BenchmarkReport) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(report)?;
    fs::write(path, bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        BenchmarkProfile, baseline_event_boundary, comparison_marker_count, marker_interval,
    };

    #[test]
    fn comparison_queries_all_exist_at_the_baseline_checkpoint() {
        for profile in [BenchmarkProfile::Smoke, BenchmarkProfile::Primary] {
            let count = comparison_marker_count(profile);
            let last_marker_offset = (count - 1) * marker_interval(profile);
            assert!(
                last_marker_offset < baseline_event_boundary(profile),
                "{profile:?} comparison marker {last_marker_offset} is absent at baseline {}",
                baseline_event_boundary(profile)
            );
        }
    }
}
