use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail, ensure};
use brain_domain::ProjectId;
use brain_service::{ServiceLaunchConfig, ServiceProjectConfig};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use super::{
    BenchmarkArtifacts, BenchmarkCondition, BenchmarkPair, BenchmarkStatus, BenchmarkSummary,
    GradeRecord, PlannedSample, ProductionConfigHashes, RunManifest, SampleRecord, SampleStatus,
    SuiteManifest, TokenBenchmarkReport, ValidityCheck, compare_condition_configs,
    evaluate_benchmark, freeze_project_snapshot, hash_optional_file, plan_matrix,
};

#[derive(Clone, Debug)]
pub struct BenchmarkPreflightOptions {
    pub brain_home: PathBuf,
    pub project_id: ProjectId,
    pub suite_path: PathBuf,
    pub repository: PathBuf,
    pub snapshot_source: Option<PathBuf>,
    pub run_id: Option<Uuid>,
    pub seed: u64,
    pub repeats: u32,
    pub pilot: bool,
    pub claude_control_config: PathBuf,
    pub claude_treatment_config: PathBuf,
    pub codex_control_config: PathBuf,
    pub codex_treatment_config: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct BenchmarkPreflightReport {
    pub schema_version: u32,
    pub run_id: Uuid,
    pub valid: bool,
    pub paid_sessions_launched: u32,
    pub suite_sha256: String,
    pub repository_commit: String,
    pub brain_commit: String,
    pub snapshot_sha256: String,
    pub pipe_name: String,
    pub claude_condition_diff: super::ConditionDiff,
    pub codex_condition_diff: super::ConditionDiff,
    pub production_config_hashes: ProductionConfigHashes,
    pub checkout: PathBuf,
    pub frozen_brain_home: PathBuf,
    pub validity_checks: Vec<ValidityCheck>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct BenchmarkRunPreview {
    pub schema_version: u32,
    pub run_id: Uuid,
    pub execute: bool,
    pub planned_samples: usize,
    pub completed_samples: usize,
    pub remaining_samples: usize,
    pub paid_sessions_launched: u32,
    pub artifact_directory: PathBuf,
}

pub fn preflight_benchmark(options: BenchmarkPreflightOptions) -> Result<BenchmarkPreflightReport> {
    ensure!(options.repeats > 0, "repeats must be positive");
    let suite_bytes = fs::read(&options.suite_path)
        .with_context(|| format!("read suite {}", options.suite_path.display()))?;
    let suite: SuiteManifest = serde_json::from_slice(&suite_bytes)?;
    suite.validate()?;
    let task_ids: Vec<String> = if options.pilot {
        ensure!(
            suite.pilot_task_ids.len() >= 10,
            "pilot needs at least ten tasks"
        );
        suite.pilot_task_ids.clone()
    } else {
        ensure!(
            (30..=50).contains(&suite.tasks.len()),
            "claimable suite needs 30-50 tasks"
        );
        ensure!(
            options.repeats >= 5,
            "claimable run needs at least five repeats"
        );
        suite.tasks.iter().map(|task| task.id.clone()).collect()
    };
    let fixture_commits: BTreeSet<_> = suite
        .tasks
        .iter()
        .map(|task| task.fixture_commit.as_str())
        .collect();
    ensure!(
        fixture_commits.len() == 1,
        "one run cannot mix fixture commits"
    );
    let fixture_commit = (*fixture_commits.iter().next().expect("suite has tasks")).to_owned();
    let brain_commit = git_output(&options.repository, &["rev-parse", "HEAD"])?;
    let run_id = options.run_id.unwrap_or_else(Uuid::now_v7);
    let artifacts = BenchmarkArtifacts::new(&options.brain_home, options.project_id, run_id)?;
    let checkout = artifacts.run_dir().join("checkout");
    materialize_checkout(&options.repository, &checkout, &fixture_commit)?;

    let service_config_path = ServiceLaunchConfig::default_path(&options.brain_home);
    let production_service = ServiceLaunchConfig::load(&service_config_path)?;
    let production_project = production_service
        .projects
        .iter()
        .find(|project| project.project_id == options.project_id)
        .cloned()
        .context("project is not registered in the service config")?;
    let snapshot_source = options.snapshot_source.unwrap_or_else(|| {
        options
            .brain_home
            .join("projects")
            .join(options.project_id.0.to_string())
    });
    let frozen_brain_home = artifacts.run_dir().join("frozen-brain");
    fs::create_dir_all(frozen_brain_home.join("projects"))?;
    let frozen_project = frozen_brain_home
        .join("projects")
        .join(options.project_id.0.to_string());
    let snapshot = freeze_project_snapshot(&snapshot_source, &frozen_project, options.project_id)?;
    let frozen_ledger = rebase_ledger_path(&production_project, &snapshot_source, &frozen_project)?;
    let pipe_name = format!("agent-brain-benchmark-{run_id}");
    let mut frozen_service = ServiceLaunchConfig::new(&pipe_name);
    frozen_service.review = production_service.review;
    frozen_service.projects.push(ServiceProjectConfig {
        project_root: checkout.clone(),
        project_id: production_project.project_id,
        worktree_id: production_project.worktree_id,
        ledger_path: frozen_ledger,
        claude_sources: Vec::new(),
        codex_sources: Vec::new(),
        hermes_database: None,
    });
    let frozen_service_path = ServiceLaunchConfig::default_path(&frozen_brain_home);
    fs::create_dir_all(frozen_service_path.parent().expect("service config parent"))?;
    fs::write(
        &frozen_service_path,
        serde_json::to_vec_pretty(&frozen_service)?,
    )?;

    let claude_control = read_json_value(&options.claude_control_config)?;
    let claude_treatment = read_json_value(&options.claude_treatment_config)?;
    let codex_control = read_json_value(&options.codex_control_config)?;
    let codex_treatment = read_json_value(&options.codex_treatment_config)?;
    let allowed = [
        "/hooks/agent_brain",
        "/mcp/agent_brain",
        "/environment/BRAIN_HOME",
        "/environment/BRAIN_PIPE_NAME",
    ];
    let claude_diff = compare_condition_configs(&claude_control, &claude_treatment, &allowed);
    let codex_diff = compare_condition_configs(&codex_control, &codex_treatment, &allowed);
    let production_hashes = production_config_hashes()?;
    let validity_checks = vec![
        ValidityCheck {
            name: "claude_condition_diff".to_owned(),
            passed: claude_diff.passed,
            detail: if claude_diff.passed {
                "only Agent Brain paths differ".to_owned()
            } else {
                format!(
                    "unexpected paths: {}",
                    claude_diff.unexpected_paths.join(", ")
                )
            },
        },
        ValidityCheck {
            name: "codex_condition_diff".to_owned(),
            passed: codex_diff.passed,
            detail: if codex_diff.passed {
                "only Agent Brain paths differ".to_owned()
            } else {
                format!(
                    "unexpected paths: {}",
                    codex_diff.unexpected_paths.join(", ")
                )
            },
        },
        ValidityCheck {
            name: "capture_isolation".to_owned(),
            passed: frozen_service.projects[0].claude_sources.is_empty()
                && frozen_service.projects[0].codex_sources.is_empty(),
            detail: "benchmark service has no transcript capture sources".to_owned(),
        },
    ];
    let suite_sha256 = hex::encode(Sha256::digest(&suite_bytes));
    let manifest = RunManifest {
        schema_version: 1,
        run_id,
        project_id: options.project_id,
        suite_id: suite.suite_id.clone(),
        suite_sha256: suite_sha256.clone(),
        repository_commit: fixture_commit.clone(),
        brain_commit: brain_commit.clone(),
        frozen_snapshot_sha256: snapshot.sha256.clone(),
        seed: options.seed,
        repeats: options.repeats,
        bootstrap_resamples: 10_000,
        claimable: !options.pilot,
        created_at: now_string()?,
        matrix: plan_matrix(&task_ids, options.repeats, options.seed),
    };
    artifacts.create_run(&manifest)?;
    artifacts.write_immutable_file(Path::new("suite.json"), &suite_bytes)?;
    let suite_directory = options
        .suite_path
        .parent()
        .context("suite path has no parent")?;
    let task_text: BTreeMap<String, (String, String)> = suite
        .tasks
        .iter()
        .map(|task| {
            let rubric = fs::read_to_string(suite_directory.join(&task.rubric))
                .with_context(|| format!("read rubric for {}", task.id))?;
            Ok((task.id.clone(), (task.prompt.clone(), rubric)))
        })
        .collect::<Result<_>>()?;
    artifacts.write_named_json("grading-tasks", &task_text)?;
    for (name, source) in [
        (
            "configs/claude-control.json",
            &options.claude_control_config,
        ),
        (
            "configs/claude-treatment.json",
            &options.claude_treatment_config,
        ),
        ("configs/codex-control.json", &options.codex_control_config),
        (
            "configs/codex-treatment.json",
            &options.codex_treatment_config,
        ),
    ] {
        artifacts.write_immutable_file(Path::new(name), &fs::read(source)?)?;
    }
    let report = BenchmarkPreflightReport {
        schema_version: 1,
        run_id,
        valid: validity_checks.iter().all(|check| check.passed),
        paid_sessions_launched: 0,
        suite_sha256,
        repository_commit: fixture_commit,
        brain_commit,
        snapshot_sha256: snapshot.sha256,
        pipe_name,
        claude_condition_diff: claude_diff,
        codex_condition_diff: codex_diff,
        production_config_hashes: production_hashes,
        checkout,
        frozen_brain_home,
        validity_checks,
    };
    artifacts.write_named_json("preflight", &report)?;
    Ok(report)
}

pub fn preview_benchmark_run(
    brain_home: &Path,
    project_id: ProjectId,
    run_id: Uuid,
    execute: bool,
) -> Result<BenchmarkRunPreview> {
    let artifacts = BenchmarkArtifacts::new(brain_home, project_id, run_id)?;
    let manifest = artifacts.manifest()?;
    let samples = artifacts.samples()?;
    let completed: BTreeSet<_> = samples
        .iter()
        .filter(|sample| sample.status == SampleStatus::Completed)
        .map(|sample| sample.sample.sample_id.as_str())
        .collect();
    Ok(BenchmarkRunPreview {
        schema_version: 1,
        run_id,
        execute,
        planned_samples: manifest.matrix.len(),
        completed_samples: completed.len(),
        remaining_samples: manifest.matrix.len().saturating_sub(completed.len()),
        paid_sessions_launched: 0,
        artifact_directory: artifacts.run_dir().to_path_buf(),
    })
}

pub fn build_report_from_artifacts(
    brain_home: &Path,
    project_id: ProjectId,
    run_id: Uuid,
) -> Result<TokenBenchmarkReport> {
    let artifacts = BenchmarkArtifacts::new(brain_home, project_id, run_id)?;
    let manifest = artifacts.manifest()?;
    let preflight: BenchmarkPreflightReport = artifacts.read_named_json("preflight")?;
    let samples = artifacts.samples()?;
    let grades = artifacts.grades()?;
    let key = read_grading_key(&artifacts.run_dir().join("grading-key.csv"))?;
    let grade_by_sample: BTreeMap<_, _> = grades
        .iter()
        .filter_map(|grade| {
            key.get(&grade.opaque_id)
                .map(|sample| (sample.clone(), grade))
        })
        .collect();
    let mut validity = preflight.validity_checks;
    let expected_ids: BTreeSet<_> = manifest
        .matrix
        .iter()
        .map(|sample| sample.sample_id.as_str())
        .collect();
    let sample_by_id: BTreeMap<_, _> = samples
        .iter()
        .map(|sample| (sample.sample.sample_id.as_str(), sample))
        .collect();
    validity.push(ValidityCheck {
        name: "matrix_complete".to_owned(),
        passed: expected_ids.iter().all(|id| sample_by_id.contains_key(id)),
        detail: format!(
            "{} of {} samples present",
            sample_by_id.len(),
            expected_ids.len()
        ),
    });
    validity.push(ValidityCheck {
        name: "native_usage_complete".to_owned(),
        passed: expected_ids.iter().all(|id| {
            sample_by_id.get(id).is_some_and(|sample| {
                sample.status == SampleStatus::Completed && sample.native_usage.is_some()
            })
        }),
        detail: "every planned sample must have exact native usage".to_owned(),
    });
    validity.push(ValidityCheck {
        name: "blind_grading_complete".to_owned(),
        passed: expected_ids
            .iter()
            .all(|id| grade_by_sample.contains_key(*id)),
        detail: "every planned sample must have a blind grade".to_owned(),
    });
    let pairs = build_pairs(&manifest.matrix, &sample_by_id, &grade_by_sample);
    if pairs.is_empty() {
        let failed = validity
            .iter()
            .find(|check| !check.passed)
            .map(|check| format!("{}: {}", check.name, check.detail))
            .unwrap_or_else(|| "no complete matched pairs".to_owned());
        let report = TokenBenchmarkReport {
            schema_version: 1,
            status: BenchmarkStatus::Invalid,
            statement: format!("This run cannot support a token-savings claim because {failed}."),
            minimum_pairs_per_harness: manifest.matrix.len() / 4,
            overall: None,
            overall_quality: None,
            harnesses: Vec::new(),
            validity_checks: validity,
        };
        artifacts.write_report(&report)?;
        return Ok(report);
    }
    let report = evaluate_benchmark(
        &pairs,
        &validity,
        manifest.bootstrap_resamples,
        manifest.seed,
        manifest.matrix.len() / 4,
    )?;
    artifacts.write_report(&report)?;
    let summary = BenchmarkSummary {
        schema_version: 1,
        run_id,
        status: report.status,
        statement: report.statement.clone(),
        completed_at: now_string()?,
        overall: report.overall.clone(),
        harnesses: report.harnesses.clone(),
        overall_quality: report.overall_quality.clone(),
        validity_checks: report.validity_checks.clone(),
    };
    artifacts.write_summary(&summary)?;
    Ok(report)
}

fn build_pairs(
    matrix: &[PlannedSample],
    samples: &BTreeMap<&str, &SampleRecord>,
    grades: &BTreeMap<String, &GradeRecord>,
) -> Vec<BenchmarkPair> {
    let mut by_pair: BTreeMap<&str, Vec<&PlannedSample>> = BTreeMap::new();
    for planned in matrix {
        by_pair.entry(&planned.pair_id).or_default().push(planned);
    }
    by_pair
        .into_values()
        .filter_map(|planned| {
            let control = planned
                .iter()
                .find(|sample| sample.condition == BenchmarkCondition::BrainOff)?;
            let treatment = planned
                .iter()
                .find(|sample| sample.condition == BenchmarkCondition::BrainOn)?;
            let control_sample = *samples.get(control.sample_id.as_str())?;
            let treatment_sample = *samples.get(treatment.sample_id.as_str())?;
            let control_usage = control_sample.native_usage.as_ref()?;
            let treatment_usage = treatment_sample.native_usage.as_ref()?;
            let control_grade = *grades.get(&control.sample_id)?;
            let treatment_grade = *grades.get(&treatment.sample_id)?;
            Some(BenchmarkPair {
                task_id: control.task_id.clone(),
                harness: control.harness,
                repeat: control.repeat,
                control_tokens: control_usage.total_tokens,
                treatment_tokens: treatment_usage.total_tokens,
                control_grade: control_grade.outcome,
                treatment_grade: treatment_grade.outcome,
                treatment_critical_regression: treatment_grade.critical_regression,
            })
        })
        .collect()
}

fn production_config_hashes() -> Result<ProductionConfigHashes> {
    let profile = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .context("USERPROFILE is unavailable")?;
    Ok(ProductionConfigHashes {
        claude_settings: hash_optional_file(&profile.join(".claude/settings.json"))?,
        codex_hooks: hash_optional_file(&profile.join(".codex/hooks.json"))?,
        codex_config: hash_optional_file(&profile.join(".codex/config.toml"))?,
    })
}

fn read_json_value(path: &Path) -> Result<serde_json::Value> {
    serde_json::from_slice(&fs::read(path)?).with_context(|| format!("read {}", path.display()))
}

fn materialize_checkout(repository: &Path, checkout: &Path, commit: &str) -> Result<()> {
    if checkout.is_dir() {
        let actual = git_output(checkout, &["rev-parse", "HEAD"])?;
        ensure!(
            actual == commit,
            "existing benchmark checkout is at {actual}, expected {commit}"
        );
        return Ok(());
    }
    ensure!(repository.is_dir(), "repository does not exist");
    let parent = checkout.parent().context("checkout has no parent")?;
    fs::create_dir_all(parent)?;
    let status = Command::new("git")
        .args(["clone", "--no-hardlinks", "--no-checkout"])
        .arg(repository)
        .arg(checkout)
        .status()?;
    ensure!(status.success(), "git clone failed");
    let status = Command::new("git")
        .current_dir(checkout)
        .args(["checkout", "--detach", commit])
        .status()?;
    ensure!(status.success(), "git checkout failed");
    Ok(())
}

fn git_output(repository: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .current_dir(repository)
        .args(args)
        .output()?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn rebase_ledger_path(
    project: &ServiceProjectConfig,
    source: &Path,
    destination: &Path,
) -> Result<PathBuf> {
    let relative = project.ledger_path.strip_prefix(source).with_context(|| {
        format!(
            "ledger {} is outside snapshot source",
            project.ledger_path.display()
        )
    })?;
    Ok(destination.join(relative))
}

fn read_grading_key(path: &Path) -> Result<BTreeMap<String, String>> {
    if !path.is_file() {
        return Ok(BTreeMap::new());
    }
    #[derive(Deserialize)]
    struct Row {
        opaque_id: String,
        sample_id: String,
    }
    csv::Reader::from_path(path)?
        .deserialize::<Row>()
        .map(|row| {
            let row = row?;
            Ok((row.opaque_id, row.sample_id))
        })
        .collect()
}

fn now_string() -> Result<String> {
    Ok(time::OffsetDateTime::now_utc().format(&Rfc3339)?)
}
