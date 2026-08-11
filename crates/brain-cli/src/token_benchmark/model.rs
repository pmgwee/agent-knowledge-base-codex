use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, ensure};
use brain_domain::ProjectId;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchmarkHarness {
    ClaudeCode,
    Codex,
}

impl BenchmarkHarness {
    pub const ALL: [Self; 2] = [Self::ClaudeCode, Self::Codex];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude_code",
            Self::Codex => "codex",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchmarkCondition {
    BrainOff,
    BrainOn,
}

impl BenchmarkCondition {
    pub const fn opposite(self) -> Self {
        match self {
            Self::BrainOff => Self::BrainOn,
            Self::BrainOn => Self::BrainOff,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchmarkTaskStratum {
    HistoricalRecall,
    CodebaseNavigation,
    DiagnosisPlanning,
    BoundedChange,
}

impl BenchmarkTaskStratum {
    pub const ALL: [Self; 4] = [
        Self::HistoricalRecall,
        Self::CodebaseNavigation,
        Self::DiagnosisPlanning,
        Self::BoundedChange,
    ];
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct BenchmarkTask {
    pub id: String,
    pub stratum: BenchmarkTaskStratum,
    pub prompt: String,
    pub fixture_commit: String,
    pub allowed_files: Vec<String>,
    pub rubric: String,
    pub reference_facts: Vec<String>,
    pub automated_check: Option<String>,
    pub critical_regression: String,
    pub combined_eligible: bool,
    pub max_turns: u32,
    pub max_tool_calls: u32,
    pub timeout_seconds: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct SuiteManifest {
    pub schema_version: u32,
    pub suite_id: String,
    pub pilot_task_ids: Vec<String>,
    pub tasks: Vec<BenchmarkTask>,
}

impl SuiteManifest {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.schema_version == 1, "unsupported suite schema version");
        ensure!(!self.suite_id.trim().is_empty(), "suite id is required");
        ensure!(!self.tasks.is_empty(), "suite has no tasks");
        let mut ids = BTreeSet::new();
        let mut strata = BTreeMap::new();
        for task in &self.tasks {
            ensure!(!task.id.trim().is_empty(), "task id is required");
            ensure!(
                ids.insert(task.id.as_str()),
                "duplicate task id {}",
                task.id
            );
            ensure!(
                !task.prompt.trim().is_empty(),
                "task {} has no prompt",
                task.id
            );
            ensure!(
                task.fixture_commit.len() == 40,
                "task {} fixture commit is not pinned",
                task.id
            );
            ensure!(task.max_turns > 0, "task {} has no turn budget", task.id);
            ensure!(
                task.max_tool_calls > 0,
                "task {} has no tool budget",
                task.id
            );
            ensure!(task.timeout_seconds > 0, "task {} has no timeout", task.id);
            *strata.entry(task.stratum).or_insert(0usize) += 1;
        }
        for stratum in BenchmarkTaskStratum::ALL {
            ensure!(
                strata.contains_key(&stratum),
                "suite is missing {stratum:?}"
            );
        }
        let counts: BTreeSet<_> = strata.values().copied().collect();
        ensure!(counts.len() == 1, "task strata are not balanced");
        for pilot in &self.pilot_task_ids {
            ensure!(ids.contains(pilot.as_str()), "unknown pilot task {pilot}");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct PlannedSample {
    pub sample_id: String,
    pub pair_id: String,
    pub task_id: String,
    pub harness: BenchmarkHarness,
    pub repeat: u32,
    pub condition: BenchmarkCondition,
    pub order: u8,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct RunManifest {
    pub schema_version: u32,
    pub run_id: Uuid,
    pub project_id: ProjectId,
    pub suite_id: String,
    pub suite_sha256: String,
    pub repository_commit: String,
    pub brain_commit: String,
    pub frozen_snapshot_sha256: String,
    pub seed: u64,
    pub repeats: u32,
    pub bootstrap_resamples: u32,
    pub claimable: bool,
    pub created_at: String,
    pub matrix: Vec<PlannedSample>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct NativeUsage {
    pub harness: BenchmarkHarness,
    pub input_tokens: u64,
    pub cache_creation_input_tokens: Option<u64>,
    pub cache_read_input_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub cache_write_input_tokens: Option<u64>,
    pub output_tokens: u64,
    pub reasoning_output_tokens: Option<u64>,
    pub total_tokens: u64,
    pub native_records: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SampleStatus {
    Completed,
    InvalidUsage,
    HarnessFailure,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct SampleRecord {
    pub schema_version: u32,
    pub sample: PlannedSample,
    pub status: SampleStatus,
    pub attempt: u32,
    pub native_usage: Option<NativeUsage>,
    pub answer: String,
    pub automated_test_passed: Option<bool>,
    pub stdout_sha256: String,
    pub stderr_sha256: String,
    pub error: Option<String>,
    pub completed_at: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GradeOutcome {
    Pass,
    Partial,
    Fail,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct GradeRecord {
    pub schema_version: u32,
    pub opaque_id: String,
    pub outcome: GradeOutcome,
    pub critical_regression: bool,
    pub reason: String,
    pub grader: String,
    pub grader_version: String,
    pub graded_at: String,
}

impl GradeOutcome {
    pub const fn is_success(self) -> bool {
        matches!(self, Self::Pass)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct BenchmarkPair {
    pub task_id: String,
    pub harness: BenchmarkHarness,
    pub repeat: u32,
    pub control_tokens: u64,
    pub treatment_tokens: u64,
    pub control_grade: GradeOutcome,
    pub treatment_grade: GradeOutcome,
    pub treatment_critical_regression: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct ValidityCheck {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct TokenEstimate {
    pub pairs: usize,
    pub control_tokens: u64,
    pub treatment_tokens: u64,
    pub saved_tokens: i64,
    pub savings_fraction: f64,
    pub confidence_low: f64,
    pub confidence_high: f64,
    pub bootstrap_resamples: u32,
    pub bootstrap_seed: u64,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct QualityEstimate {
    pub pairs: usize,
    pub control_success_fraction: f64,
    pub treatment_success_fraction: f64,
    pub difference: f64,
    pub one_sided_confidence_low: f64,
    pub critical_regressions: usize,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct HarnessBenchmarkReport {
    pub harness: BenchmarkHarness,
    pub tokens: TokenEstimate,
    pub quality: QualityEstimate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchmarkStatus {
    Proven,
    Inconclusive,
    QualityBlocked,
    Invalid,
}

impl BenchmarkStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Proven => "proven",
            Self::Inconclusive => "inconclusive",
            Self::QualityBlocked => "quality_blocked",
            Self::Invalid => "invalid",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct TokenBenchmarkReport {
    pub schema_version: u32,
    pub status: BenchmarkStatus,
    pub statement: String,
    pub minimum_pairs_per_harness: usize,
    pub overall: Option<TokenEstimate>,
    pub overall_quality: Option<QualityEstimate>,
    pub harnesses: Vec<HarnessBenchmarkReport>,
    pub validity_checks: Vec<ValidityCheck>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct BenchmarkSummary {
    pub schema_version: u32,
    pub run_id: Uuid,
    pub status: BenchmarkStatus,
    pub statement: String,
    pub completed_at: String,
    pub overall: Option<TokenEstimate>,
    pub harnesses: Vec<HarnessBenchmarkReport>,
    pub overall_quality: Option<QualityEstimate>,
    pub validity_checks: Vec<ValidityCheck>,
}
