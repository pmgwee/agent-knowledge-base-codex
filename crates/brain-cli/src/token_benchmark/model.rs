use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

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

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
pub enum BenchmarkCondition {
    #[serde(rename = "c0", alias = "brain_off")]
    C0,
    #[serde(rename = "c1")]
    C1,
    #[serde(rename = "c2")]
    C2,
    #[serde(rename = "c3")]
    C3,
    #[serde(rename = "c4", alias = "brain_on")]
    C4,
}

impl BenchmarkCondition {
    pub const ALL: [Self; 5] = [Self::C0, Self::C1, Self::C2, Self::C3, Self::C4];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::C0 => "c0",
            Self::C1 => "c1",
            Self::C2 => "c2",
            Self::C3 => "c3",
            Self::C4 => "c4",
        }
    }

    pub const fn requires_brain_service(self) -> bool {
        matches!(self, Self::C2 | Self::C3 | Self::C4)
    }

    #[allow(non_upper_case_globals)]
    pub const BrainOff: Self = Self::C0;

    #[allow(non_upper_case_globals)]
    pub const BrainOn: Self = Self::C4;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BenchmarkContrast {
    pub id: &'static str,
    pub baseline: BenchmarkCondition,
    pub treatment: BenchmarkCondition,
}

pub const PRIMARY_CONTRASTS: [BenchmarkContrast; 6] = [
    BenchmarkContrast {
        id: "c1_vs_c0",
        baseline: BenchmarkCondition::C0,
        treatment: BenchmarkCondition::C1,
    },
    BenchmarkContrast {
        id: "c2_vs_c1",
        baseline: BenchmarkCondition::C1,
        treatment: BenchmarkCondition::C2,
    },
    BenchmarkContrast {
        id: "c3_vs_c2",
        baseline: BenchmarkCondition::C2,
        treatment: BenchmarkCondition::C3,
    },
    BenchmarkContrast {
        id: "c4_vs_c3",
        baseline: BenchmarkCondition::C3,
        treatment: BenchmarkCondition::C4,
    },
    BenchmarkContrast {
        id: "c4_vs_c0",
        baseline: BenchmarkCondition::C0,
        treatment: BenchmarkCondition::C4,
    },
    BenchmarkContrast {
        id: "c4_vs_c1",
        baseline: BenchmarkCondition::C1,
        treatment: BenchmarkCondition::C4,
    },
];

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
    #[serde(default)]
    pub trace_markers: Option<TraceMarkers>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct ExternalBenchmarkClaim {
    pub benchmark: String,
    pub metric: String,
    pub value: String,
    pub evidence_class: String,
    pub comparability: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct ExternalBenchmarkReference {
    pub system: String,
    pub source_url: String,
    pub source_revision: String,
    pub source_sha256: String,
    pub claims: Vec<ExternalBenchmarkClaim>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct SuiteManifest {
    pub schema_version: u32,
    pub suite_id: String,
    pub pilot_task_ids: Vec<String>,
    #[serde(default)]
    pub external_references: Vec<ExternalBenchmarkReference>,
    pub tasks: Vec<BenchmarkTask>,
}

impl SuiteManifest {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            matches!(self.schema_version, 1 | 2),
            "unsupported suite schema version"
        );
        ensure!(!self.suite_id.trim().is_empty(), "suite id is required");
        ensure!(!self.tasks.is_empty(), "suite has no tasks");
        for reference in &self.external_references {
            ensure!(
                !reference.system.trim().is_empty(),
                "external reference system is required"
            );
            ensure!(
                !reference.source_url.trim().is_empty(),
                "external reference source URL is required"
            );
            ensure!(
                !reference.source_revision.trim().is_empty(),
                "external reference source revision is required"
            );
            ensure!(
                reference.source_sha256.len() == 64
                    && reference
                        .source_sha256
                        .chars()
                        .all(|c| c.is_ascii_hexdigit()),
                "external reference source SHA-256 must be 64 hexadecimal characters"
            );
            ensure!(
                !reference.claims.is_empty(),
                "external reference has no claims"
            );
            for claim in &reference.claims {
                ensure!(
                    [
                        claim.benchmark.as_str(),
                        claim.metric.as_str(),
                        claim.value.as_str(),
                        claim.evidence_class.as_str(),
                        claim.comparability.as_str(),
                    ]
                    .iter()
                    .all(|value| !value.trim().is_empty()),
                    "external reference claim fields are required"
                );
            }
        }
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
    #[serde(rename = "block_id", alias = "pair_id")]
    pub pair_id: String,
    pub task_id: String,
    pub harness: BenchmarkHarness,
    pub repeat: u32,
    pub condition: BenchmarkCondition,
    pub order: u8,
}

/// A fully materialized, inspectable native harness launch profile.
///
/// Profiles are supplied at preflight rather than inferred from a user's live settings. This is
/// deliberate: inference can silently omit a hook, MCP server, provider flag, or permission and
/// create a control/treatment difference that is not the brain. Every placeholder is expanded by
/// the benchmark runner and the immutable profile is retained with the run artifacts.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct ExecutionTemplate {
    pub program: PathBuf,
    pub args: Vec<String>,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    pub timeout_seconds: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct HarnessExecutionTemplates {
    pub conditions: BTreeMap<BenchmarkCondition, ExecutionTemplate>,
}

impl<'de> Deserialize<'de> for HarnessExecutionTemplates {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            #[serde(default)]
            conditions: Option<BTreeMap<BenchmarkCondition, ExecutionTemplate>>,
            #[serde(default)]
            brain_off: Option<ExecutionTemplate>,
            #[serde(default)]
            brain_on: Option<ExecutionTemplate>,
        }

        let wire = Wire::deserialize(deserializer)?;
        if let Some(conditions) = wire.conditions {
            if wire.brain_off.is_some() || wire.brain_on.is_some() {
                return Err(serde::de::Error::custom(
                    "condition map cannot be combined with legacy brain_off/brain_on",
                ));
            }
            return Ok(Self { conditions });
        }
        let brain_off = wire
            .brain_off
            .ok_or_else(|| serde::de::Error::missing_field("conditions"))?;
        let brain_on = wire
            .brain_on
            .ok_or_else(|| serde::de::Error::missing_field("brain_on"))?;
        Ok(Self {
            conditions: BTreeMap::from([
                (BenchmarkCondition::C0, brain_off),
                (BenchmarkCondition::C4, brain_on),
            ]),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct ExecutionTemplates {
    pub schema_version: u32,
    pub max_attempts: u32,
    pub brain_service_program: PathBuf,
    #[serde(default)]
    pub launcher_environment: BTreeMap<String, String>,
    pub claude_code: HarnessExecutionTemplates,
    pub codex: HarnessExecutionTemplates,
}

impl ExecutionTemplates {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            matches!(self.schema_version, 1 | 2),
            "unsupported execution-template schema"
        );
        let expected = if self.schema_version == 1 {
            BTreeSet::from([BenchmarkCondition::C0, BenchmarkCondition::C4])
        } else {
            BenchmarkCondition::ALL.into_iter().collect()
        };
        for (name, harness) in [("claude_code", &self.claude_code), ("codex", &self.codex)] {
            let actual = harness.conditions.keys().copied().collect::<BTreeSet<_>>();
            if let Some(missing) = expected.difference(&actual).next() {
                anyhow::bail!(
                    "{name} execution templates are missing {}",
                    missing.as_str()
                );
            }
            if let Some(extra) = actual.difference(&expected).next() {
                anyhow::bail!(
                    "{name} execution templates contain unexpected {}",
                    extra.as_str()
                );
            }
        }
        Ok(())
    }

    pub fn template(
        &self,
        harness: BenchmarkHarness,
        condition: BenchmarkCondition,
    ) -> &ExecutionTemplate {
        let harness = match harness {
            BenchmarkHarness::ClaudeCode => &self.claude_code,
            BenchmarkHarness::Codex => &self.codex,
        };
        harness
            .conditions
            .get(&condition)
            .expect("validated execution templates contain every planned condition")
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct ExecutablePin {
    pub path: PathBuf,
    pub sha256: String,
    pub version: String,
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

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct TraceMarkers {
    pub first_correct_file: String,
    pub first_edit: String,
    pub first_passing_test: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceObservation {
    Observed {
        elapsed_ms: u64,
    },
    #[default]
    NotObservable,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
pub struct NativeTrace {
    pub turns: u32,
    pub total_tool_calls: u32,
    pub tool_calls_by_name: BTreeMap<String, u32>,
    pub brain_mcp_calls: u32,
    pub codegraph_calls: u32,
    pub file_read_calls: u32,
    pub unique_files: BTreeSet<String>,
    pub edit_calls: u32,
    pub test_commands: u32,
    pub first_correct_file: TraceObservation,
    pub first_edit: TraceObservation,
    pub first_passing_test: TraceObservation,
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
    #[serde(default)]
    pub elapsed_ms: Option<u64>,
    #[serde(default)]
    pub native_trace: Option<NativeTrace>,
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

/// One condition assignment in the five-way repeated-measures analysis.
///
/// Missing native counters remain `None`; the report invalidates that goal instead of estimating
/// usage from text or silently removing a failed sample. Grades remain present for failed tasks so
/// quality denominators follow intention-to-treat.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct BenchmarkObservation {
    pub task_id: String,
    pub harness: BenchmarkHarness,
    pub repeat: u32,
    pub stratum: BenchmarkTaskStratum,
    pub condition: BenchmarkCondition,
    pub native_tokens: Option<u64>,
    pub elapsed_ms: Option<u64>,
    pub grade: GradeOutcome,
    pub critical_regression: bool,
    pub timed_out: bool,
    pub harness_failed: bool,
    pub brain_mcp_calls: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    Proven,
    Inconclusive,
    Regressed,
    Invalid,
    NotRun,
}

impl GoalStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Proven => "proven",
            Self::Inconclusive => "inconclusive",
            Self::Regressed => "regressed",
            Self::Invalid => "invalid",
            Self::NotRun => "not_run",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct RatioOfSumsEstimate {
    pub samples: usize,
    pub baseline_total: u64,
    pub treatment_total: u64,
    pub absolute_reduction: i64,
    pub reduction_fraction: f64,
    pub reduction_percent: f64,
    pub confidence_low: f64,
    pub confidence_high: f64,
    pub confidence_low_percent: f64,
    pub confidence_high_percent: f64,
    pub bootstrap_resamples: u32,
    pub bootstrap_seed: u64,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct PassRateEstimate {
    pub samples: usize,
    pub baseline_pass_fraction: f64,
    pub treatment_pass_fraction: f64,
    pub difference: f64,
    pub difference_percentage_points: f64,
    pub confidence_low: f64,
    pub confidence_high: f64,
    pub historical_difference: Option<f64>,
    pub historical_confidence_low: Option<f64>,
    pub critical_regressions: usize,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct GoalEstimate<T> {
    pub status: GoalStatus,
    pub estimate: Option<T>,
    pub harness_estimates: BTreeMap<BenchmarkHarness, T>,
    pub invalid_reasons: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct McpAdoptionDiagnostic {
    pub assigned: u64,
    pub called: u64,
    pub adoption_percent: Option<f64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct IntentionToTreatDiagnostic {
    pub complete_pairs: usize,
    pub missing_pairs: usize,
    pub invalid_token_samples: usize,
    pub invalid_time_samples: usize,
    pub timed_out_samples: usize,
    pub harness_failure_samples: usize,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct ThreeGoalContrastReport {
    pub contrast_id: String,
    pub baseline: BenchmarkCondition,
    pub treatment: BenchmarkCondition,
    pub tokens: GoalEstimate<RatioOfSumsEstimate>,
    pub speed: GoalEstimate<RatioOfSumsEstimate>,
    pub quality: GoalEstimate<PassRateEstimate>,
    pub retrieval: Option<RetrievalContrastMetrics>,
    pub intention_to_treat: IntentionToTreatDiagnostic,
    pub mcp_adoption: McpAdoptionDiagnostic,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct RetrievalContrastMetrics {
    pub precision_at_5_percent: f64,
    pub recall_at_5_percent: f64,
    pub citation_precision_percent: f64,
    pub faithfulness_percent: f64,
    pub cross_project_leakage_percent: f64,
    pub stale_contradiction_percent: f64,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct ThreeGoalBenchmarkReport {
    pub schema_version: u32,
    pub bootstrap_resamples: u32,
    pub bootstrap_seed: u64,
    pub statement: String,
    pub contrasts: Vec<ThreeGoalContrastReport>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ThreeGoalEvaluationOptions {
    pub bootstrap_resamples: u32,
    pub seed: u64,
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
    #[serde(default)]
    pub three_goal: Option<ThreeGoalBenchmarkReport>,
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
    #[serde(default)]
    pub metadata: BenchmarkMetadata,
    #[serde(default)]
    pub pair_audit: Vec<PairAudit>,
    #[serde(default)]
    pub three_goal: Option<ThreeGoalBenchmarkReport>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
pub struct BenchmarkMetadata {
    pub suite_id: String,
    pub repository_commit: String,
    pub brain_commit: String,
    pub frozen_snapshot_sha256: String,
    pub repeats: u32,
    pub tasks: usize,
    pub models: BTreeMap<String, String>,
    pub harness_versions: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct PairAudit {
    pub task_id: String,
    pub harness: BenchmarkHarness,
    pub repeat: u32,
    pub control_tokens: u64,
    pub treatment_tokens: u64,
    pub control_grade: GradeOutcome,
    pub treatment_grade: GradeOutcome,
    pub treatment_critical_regression: bool,
}
