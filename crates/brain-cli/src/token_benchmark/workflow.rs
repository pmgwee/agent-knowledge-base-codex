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
    BenchmarkArtifacts, BenchmarkCondition, BenchmarkHarness, BenchmarkMetadata,
    BenchmarkObservation, BenchmarkPair, BenchmarkStatus, BenchmarkSummary, ExecutablePin,
    ExecutionTemplates, GradeRecord, PairAudit, PlannedSample, ProductionConfigHashes, RunManifest,
    SampleRecord, SampleStatus, SuiteManifest, ThreeGoalEvaluationOptions, TokenBenchmarkReport,
    ValidityCheck, compare_condition_configs, evaluate_benchmark, evaluate_three_goal_benchmark,
    freeze_project_snapshot, hash_optional_file, plan_matrix,
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
    pub smoke: bool,
    pub pilot: bool,
    pub condition_profiles: PathBuf,
    pub execution_templates: PathBuf,
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
    pub execution_templates_sha256: String,
    pub pipe_name: String,
    pub executable_pins: BTreeMap<String, ExecutablePin>,
    pub claude_condition_diff: super::ConditionDiff,
    pub codex_condition_diff: super::ConditionDiff,
    #[serde(default)]
    pub condition_profiles: Option<super::ConditionProfileValidation>,
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
    ensure!(
        !(options.smoke && options.pilot),
        "smoke and pilot modes are mutually exclusive"
    );
    let suite_bytes = fs::read(&options.suite_path)
        .with_context(|| format!("read suite {}", options.suite_path.display()))?;
    let suite: SuiteManifest = serde_json::from_slice(&suite_bytes)?;
    suite.validate()?;
    let task_ids: Vec<String> = if options.smoke {
        ensure!(options.repeats == 1, "schema smoke uses exactly one repeat");
        ensure!(
            suite.pilot_task_ids.len() >= 2,
            "smoke needs two pilot tasks"
        );
        suite.pilot_task_ids.iter().take(2).cloned().collect()
    } else if options.pilot {
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
    let source_status = git_output(
        &options.repository,
        &["status", "--porcelain", "--untracked-files=all"],
    )?;
    let mut execution_templates: ExecutionTemplates =
        serde_json::from_slice(&fs::read(&options.execution_templates).with_context(|| {
            format!(
                "read execution templates {}",
                options.execution_templates.display()
            )
        })?)?;
    let executable_pins = validate_and_pin_execution_templates(&mut execution_templates)?;
    let execution_templates_bytes = serde_json::to_vec_pretty(&execution_templates)?;
    let execution_templates_sha256 = hex::encode(Sha256::digest(&execution_templates_bytes));
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
    let pipe_name = format!(r"\\.\pipe\agent-brain-benchmark-{run_id}");
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

    let profile_validation = super::validate_condition_profiles(&options.condition_profiles)?;
    let claude_control = read_json_value(&options.condition_profiles.join(
        super::preflight::condition_profile_name(
            BenchmarkHarness::ClaudeCode,
            BenchmarkCondition::C0,
        ),
    ))?;
    let claude_treatment = read_json_value(&options.condition_profiles.join(
        super::preflight::condition_profile_name(
            BenchmarkHarness::ClaudeCode,
            BenchmarkCondition::C4,
        ),
    ))?;
    let codex_control = read_json_value(&options.condition_profiles.join(
        super::preflight::condition_profile_name(BenchmarkHarness::Codex, BenchmarkCondition::C0),
    ))?;
    let codex_treatment = read_json_value(&options.condition_profiles.join(
        super::preflight::condition_profile_name(BenchmarkHarness::Codex, BenchmarkCondition::C4),
    ))?;
    let allowed = [
        "/condition",
        "/tools/codegraph",
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
            name: "condition_profile_matrix".to_owned(),
            passed: profile_validation.valid,
            detail: if profile_validation.valid {
                "all ten profiles match the exact cumulative C0-C4 capability table".to_owned()
            } else {
                profile_validation.errors.join("; ")
            },
        },
        ValidityCheck {
            name: "claude_condition_diff".to_owned(),
            passed: claude_diff.passed,
            detail: if claude_diff.passed {
                "only the preregistered cumulative CodeGraph and Agent Brain paths differ"
                    .to_owned()
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
                "only the preregistered cumulative CodeGraph and Agent Brain paths differ"
                    .to_owned()
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
        ValidityCheck {
            name: "brain_source_clean".to_owned(),
            passed: source_status.is_empty(),
            detail: if source_status.is_empty() {
                format!("benchmark binaries correspond to clean commit {brain_commit}")
            } else {
                "source tree has uncommitted changes; this preflight is diagnostic only and cannot support a release claim"
                    .to_owned()
            },
        },
    ];
    let suite_sha256 = hex::encode(Sha256::digest(&suite_bytes));
    let manifest = RunManifest {
        schema_version: 2,
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
        claimable: !(options.smoke || options.pilot),
        created_at: now_string()?,
        matrix: plan_matrix(&task_ids, options.repeats, options.seed),
    };
    artifacts.create_run(&manifest)?;
    artifacts.write_named_json(
        "environment",
        &serde_json::json!({
            "schema_version": 1,
            "run_class": if options.smoke { "smoke" } else if options.pilot { "pilot" } else { "claimable" },
            "architecture_label": "current_state",
            "native_defaults": {
                "model": "native_default",
                "effort": "native_default",
                "permissions": "native_default",
                "native_memory": "fresh_sample_home"
            },
            "machine": {
                "os": std::env::consts::OS,
                "architecture": std::env::consts::ARCH,
                "family": std::env::consts::FAMILY
            },
            "codegraph_state": "fresh checkout-local index in C1-C4; absent in C0",
            "brain_state": "frozen writable snapshot per C2-C4 sample; absent in C0-C1",
            "production_config_hashes": &production_hashes,
            "condition_profile_hashes": &profile_validation.profile_sha256,
            "executable_pins": &executable_pins,
            "generated_at": now_string()?
        }),
    )?;
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
    artifacts.write_immutable_file(
        Path::new("execution-templates.json"),
        &execution_templates_bytes,
    )?;
    for harness in BenchmarkHarness::ALL {
        for condition in BenchmarkCondition::ALL {
            let name = super::preflight::condition_profile_name(harness, condition);
            artifacts.write_immutable_file(
                &Path::new("configs").join(name),
                &fs::read(options.condition_profiles.join(name))?,
            )?;
        }
    }
    let instructions = options
        .condition_profiles
        .parent()
        .context("condition profile directory has no parent")?
        .join("instructions");
    for name in ["AGENTS.md", "CLAUDE.md"] {
        artifacts.write_immutable_file(
            &Path::new("instructions").join(name),
            &fs::read(instructions.join(name))?,
        )?;
    }
    let report = BenchmarkPreflightReport {
        schema_version: 3,
        run_id,
        valid: validity_checks.iter().all(|check| check.passed),
        paid_sessions_launched: 0,
        suite_sha256,
        repository_commit: fixture_commit,
        brain_commit,
        snapshot_sha256: snapshot.sha256,
        execution_templates_sha256,
        pipe_name,
        executable_pins,
        claude_condition_diff: claude_diff,
        codex_condition_diff: codex_diff,
        condition_profiles: Some(profile_validation),
        production_config_hashes: production_hashes,
        checkout,
        frozen_brain_home,
        validity_checks,
    };
    artifacts.write_named_json("preflight", &report)?;
    Ok(report)
}

fn validate_and_pin_execution_templates(
    templates: &mut ExecutionTemplates,
) -> Result<BTreeMap<String, ExecutablePin>> {
    templates.validate()?;
    ensure!(
        (1..=3).contains(&templates.max_attempts),
        "max_attempts must be between one and three"
    );
    templates.brain_service_program = canonical_executable(&templates.brain_service_program)?;
    const LAUNCHER_PATHS: [&str; 8] = [
        "BENCHMARK_CLAUDE_EXECUTABLE",
        "BENCHMARK_CODEX_EXECUTABLE",
        "BENCHMARK_CODEGRAPH_NODE",
        "BENCHMARK_CODEGRAPH_SCRIPT",
        "BENCHMARK_BRAIN_HOOK",
        "BENCHMARK_BRAIN_MCP",
        "BENCHMARK_CLAUDE_CREDENTIALS",
        "BENCHMARK_CODEX_AUTH",
    ];
    let actual_launcher_keys = templates
        .launcher_environment
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let expected_launcher_keys = LAUNCHER_PATHS.into_iter().collect::<BTreeSet<_>>();
    ensure!(
        actual_launcher_keys == expected_launcher_keys,
        "launcher_environment must contain exactly the eight benchmark-scoped path bindings"
    );
    for key in LAUNCHER_PATHS {
        let path = PathBuf::from(
            templates
                .launcher_environment
                .get(key)
                .expect("validated launcher key"),
        );
        let canonical = canonical_executable(&path)?;
        templates
            .launcher_environment
            .insert(key.to_owned(), canonical.to_string_lossy().to_string());
    }
    for harness in super::BenchmarkHarness::ALL {
        let conditions = match harness {
            super::BenchmarkHarness::ClaudeCode => &mut templates.claude_code,
            super::BenchmarkHarness::Codex => &mut templates.codex,
        };
        for template in conditions.conditions.values_mut() {
            template.program = canonical_executable(&template.program)?;
        }
        let control = conditions
            .conditions
            .get(&BenchmarkCondition::C0)
            .expect("validated execution templates include c0")
            .clone();
        ensure!(
            control
                .args
                .iter()
                .any(|argument| argument.contains("{{prompt}}")),
            "{} execution template has no {{prompt}} placeholder",
            harness.as_str()
        );
        ensure!(
            control
                .args
                .iter()
                .any(|argument| argument.contains("{{condition_config}}")),
            "{} execution template has no {{condition_config}} placeholder",
            harness.as_str()
        );
        for placeholder in ["{{sample_harness_home}}", "{{sample_codegraph_home}}"] {
            ensure!(
                control
                    .args
                    .iter()
                    .any(|argument| argument.contains(placeholder)),
                "{} execution template has no {} placeholder",
                harness.as_str(),
                placeholder
            );
        }
        ensure!(
            control.timeout_seconds > 0,
            "{} execution timeout must be positive",
            harness.as_str()
        );
        let mut control_environment = control.environment.clone();
        control_environment.remove("BRAIN_HOME");
        control_environment.remove("BRAIN_PIPE_NAME");
        for (condition, template) in &conditions.conditions {
            ensure!(
                template.program == control.program,
                "{} {} uses a different executable",
                harness.as_str(),
                condition.as_str()
            );
            ensure!(
                template.args == control.args,
                "{} {} arguments differ from c0",
                harness.as_str(),
                condition.as_str()
            );
            ensure!(
                template.timeout_seconds == control.timeout_seconds,
                "{} {} timeout differs from c0",
                harness.as_str(),
                condition.as_str()
            );
            let mut environment = template.environment.clone();
            if condition.requires_brain_service() {
                ensure!(
                    environment.remove("BRAIN_HOME").as_deref() == Some("{{sample_brain_home}}")
                        && environment.remove("BRAIN_PIPE_NAME").as_deref()
                            == Some("{{pipe_name}}"),
                    "{} {} must use benchmark-scoped brain placeholders",
                    harness.as_str(),
                    condition.as_str()
                );
            } else {
                ensure!(
                    !environment.contains_key("BRAIN_HOME")
                        && !environment.contains_key("BRAIN_PIPE_NAME"),
                    "{} {} exposes a brain endpoint",
                    harness.as_str(),
                    condition.as_str()
                );
            }
            ensure!(
                environment == control_environment,
                "{} {} environment differs outside Agent Brain",
                harness.as_str(),
                condition.as_str()
            );
        }
    }
    let launcher_path = |key: &str| {
        PathBuf::from(
            templates
                .launcher_environment
                .get(key)
                .expect("validated launcher path"),
        )
    };
    let pin_specs = vec![
        (
            "claude_launcher",
            templates
                .claude_code
                .conditions
                .get(&BenchmarkCondition::C0)
                .expect("validated c0 template")
                .program
                .clone(),
            true,
        ),
        (
            "codex_launcher",
            templates
                .codex
                .conditions
                .get(&BenchmarkCondition::C0)
                .expect("validated c0 template")
                .program
                .clone(),
            true,
        ),
        (
            "brain_service",
            templates.brain_service_program.clone(),
            false,
        ),
        (
            "claude_code",
            launcher_path("BENCHMARK_CLAUDE_EXECUTABLE"),
            true,
        ),
        ("codex", launcher_path("BENCHMARK_CODEX_EXECUTABLE"), true),
        (
            "codegraph_node",
            launcher_path("BENCHMARK_CODEGRAPH_NODE"),
            true,
        ),
        (
            "codegraph_script",
            launcher_path("BENCHMARK_CODEGRAPH_SCRIPT"),
            false,
        ),
        ("brain_hook", launcher_path("BENCHMARK_BRAIN_HOOK"), false),
        ("brain_mcp", launcher_path("BENCHMARK_BRAIN_MCP"), false),
    ];
    let mut pins = BTreeMap::new();
    for (name, path, require_version) in pin_specs {
        pins.insert(name.to_owned(), pin_executable(&path, require_version)?);
    }
    Ok(pins)
}

fn canonical_executable(path: &Path) -> Result<PathBuf> {
    ensure!(
        path.is_file(),
        "executable {} does not exist",
        path.display()
    );
    path.canonicalize()
        .with_context(|| format!("resolve executable {}", path.display()))
}

fn pin_executable(path: &Path, require_version: bool) -> Result<ExecutablePin> {
    let version = if require_version {
        let output = Command::new(path)
            .arg("--version")
            .output()
            .with_context(|| format!("read version from {}", path.display()))?;
        ensure!(
            output.status.success(),
            "{} --version failed",
            path.display()
        );
        let text = if output.stdout.is_empty() {
            String::from_utf8(output.stderr)?
        } else {
            String::from_utf8(output.stdout)?
        };
        let version = text.lines().next().unwrap_or_default().trim().to_owned();
        ensure!(
            !version.is_empty(),
            "{} returned no version",
            path.display()
        );
        version
    } else {
        "content-addressed executable".to_owned()
    };
    Ok(ExecutablePin {
        path: path.to_path_buf(),
        sha256: hex::encode(Sha256::digest(fs::read(path)?)),
        version,
    })
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
        schema_version: 2,
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
    let suite: SuiteManifest =
        serde_json::from_slice(&fs::read(artifacts.run_dir().join("suite.json"))?)?;
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
    let observations =
        build_observations(&manifest.matrix, &sample_by_id, &grade_by_sample, &suite);
    let three_goal = evaluate_three_goal_benchmark(
        &observations,
        ThreeGoalEvaluationOptions {
            bootstrap_resamples: manifest.bootstrap_resamples,
            seed: manifest.seed,
        },
    )?;
    let minimum_pairs_per_harness = planned_blocks_per_harness(&manifest.matrix);
    let pairs = build_pairs(&manifest.matrix, &sample_by_id, &grade_by_sample);
    if pairs.is_empty() {
        let failed = validity
            .iter()
            .find(|check| !check.passed)
            .map(|check| format!("{}: {}", check.name, check.detail))
            .unwrap_or_else(|| "no complete matched pairs".to_owned());
        let report = TokenBenchmarkReport {
            schema_version: 2,
            status: BenchmarkStatus::Invalid,
            statement: format!("This run cannot support a token-savings claim because {failed}."),
            minimum_pairs_per_harness,
            overall: None,
            overall_quality: None,
            harnesses: Vec::new(),
            validity_checks: validity,
            three_goal: Some(three_goal),
        };
        artifacts.write_report(&report)?;
        let summary = benchmark_summary(&manifest, &report, &pairs, &artifacts)?;
        artifacts.write_summary(&summary)?;
        write_extended_artifacts(&artifacts, &manifest, &suite, &report, &samples)?;
        return Ok(report);
    }
    let mut report = evaluate_benchmark(
        &pairs,
        &validity,
        manifest.bootstrap_resamples,
        manifest.seed,
        minimum_pairs_per_harness,
    )?;
    report.three_goal = Some(three_goal);
    artifacts.write_report(&report)?;
    let summary = benchmark_summary(&manifest, &report, &pairs, &artifacts)?;
    artifacts.write_summary(&summary)?;
    write_extended_artifacts(&artifacts, &manifest, &suite, &report, &samples)?;
    Ok(report)
}

fn write_extended_artifacts(
    artifacts: &BenchmarkArtifacts,
    manifest: &RunManifest,
    suite: &SuiteManifest,
    report: &TokenBenchmarkReport,
    samples: &[SampleRecord],
) -> Result<()> {
    let contrasts = report
        .three_goal
        .as_ref()
        .map(|three_goal| three_goal.contrasts.as_slice())
        .unwrap_or(&[]);
    artifacts.write_named_json("contrasts", &contrasts)?;
    artifacts.write_benchmark_markdown(&render_benchmark_markdown(
        artifacts, manifest, suite, report, samples,
    )?)?;
    artifacts.write_checksums()?;
    Ok(())
}

fn render_benchmark_markdown(
    artifacts: &BenchmarkArtifacts,
    manifest: &RunManifest,
    suite: &SuiteManifest,
    report: &TokenBenchmarkReport,
    samples: &[SampleRecord],
) -> Result<String> {
    let run_class = if manifest.claimable {
        "claimable"
    } else if manifest.matrix.len() <= 20 {
        "smoke"
    } else {
        "pilot"
    };
    let latest = samples.iter().fold(
        BTreeMap::<&str, &SampleRecord>::new(),
        |mut latest, sample| {
            let id = sample.sample.sample_id.as_str();
            if latest
                .get(id)
                .is_none_or(|prior| prior.attempt < sample.attempt)
            {
                latest.insert(id, sample);
            }
            latest
        },
    );
    let completed = latest
        .values()
        .filter(|sample| sample.status == SampleStatus::Completed)
        .count();
    let invalid = manifest.matrix.len().saturating_sub(completed);
    let invalid_percent = if manifest.matrix.is_empty() {
        0.0
    } else {
        invalid as f64 * 100.0 / manifest.matrix.len() as f64
    };
    let mut markdown = format!(
        "# Secondary Brain benchmark {}\n\n- Run class: **{}**\n- Architecture: **current_state**\n- Claimable: **{}**\n- Planned native sessions: **{}**\n- Latest completed samples: **{}**\n- Invalid or missing samples: **{} ({:.2}%)**\n- Brain commit: `{}`\n- Suite hash: `{}`\n- Full local evidence: `{}`\n- Integrity manifest: `checksums.sha256`\n\n",
        manifest.run_id,
        run_class,
        manifest.claimable,
        manifest.matrix.len(),
        completed,
        invalid,
        invalid_percent,
        manifest.brain_commit,
        manifest.suite_sha256,
        artifacts.run_dir().display(),
    );
    markdown.push_str("## Required final result\n\n");
    markdown.push_str("| Goal | C0 native | C4 full Brain | Change | 95% CI | Claude | Codex | Gate | Verdict |\n");
    markdown.push_str("|---|---:|---:|---:|---:|---:|---:|---|---|\n");
    let headline = report.three_goal.as_ref().and_then(|three_goal| {
        three_goal
            .contrasts
            .iter()
            .find(|contrast| contrast.contrast_id == "c4_vs_c0")
    });
    if let Some(headline) = headline {
        markdown.push_str(&ratio_goal_row(
            "Net native tokens",
            &headline.tokens,
            "≥10%, lower bound >0",
        ));
        markdown.push_str(&ratio_goal_row(
            "End-to-end time",
            &headline.speed,
            "≥10%, lower bound >0",
        ));
        markdown.push_str(&quality_goal_row(
            "Final-answer pass rate",
            &headline.quality,
            "+5 pp historical; −2 pp safety",
        ));
    } else {
        for (goal, gate) in [
            ("Net native tokens", "≥10%, lower bound >0"),
            ("End-to-end time", "≥10%, lower bound >0"),
            ("Final-answer pass rate", "+5 pp historical; −2 pp safety"),
        ] {
            markdown.push_str(&format!(
                "| {goal} | Not measured | Not measured | Not measured | Not measured | Not measured | Not measured | {gate} | Not run |\n"
            ));
        }
    }
    markdown.push_str("\n## External published context (not a C0-C4 result)\n\n");
    markdown.push_str("These independently published or modeled rows are comparison context only. They never populate Goal 1, Goal 2, or Goal 3; only this run's matched native traces and grades can do that.\n\n");
    markdown.push_str("| System | Benchmark or scenario | Metric | Published value | Evidence class | Comparability |\n");
    markdown.push_str("|---|---|---|---:|---|---|\n");
    if suite.external_references.is_empty() {
        markdown.push_str("| None pinned | Not available | Not available | Not available | Not available | No external comparison was preregistered |\n");
    } else {
        for reference in &suite.external_references {
            for claim in &reference.claims {
                markdown.push_str(&format!(
                    "| {} | {} | {} | {} | {} | {} |\n",
                    markdown_cell(&reference.system),
                    markdown_cell(&claim.benchmark),
                    markdown_cell(&claim.metric),
                    markdown_cell(&claim.value),
                    markdown_cell(&claim.evidence_class),
                    markdown_cell(&claim.comparability),
                ));
            }
        }
        markdown.push_str("\nPinned sources:\n\n");
        for reference in &suite.external_references {
            markdown.push_str(&format!(
                "- {}: [{}]({}) at `{}`; downloaded-file SHA-256 `{}`.\n",
                markdown_cell(&reference.system),
                markdown_cell(&reference.source_url),
                reference.source_url,
                reference.source_revision,
                reference.source_sha256,
            ));
        }
    }
    markdown.push_str("\n## Component contrasts\n\n");
    markdown.push_str("| Contrast | Token change | Time change | Quality change | Token verdict | Time verdict | Quality verdict |\n");
    markdown.push_str("|---|---:|---:|---:|---|---|---|\n");
    if let Some(three_goal) = &report.three_goal {
        for contrast in &three_goal.contrasts {
            let token = contrast
                .tokens
                .estimate
                .as_ref()
                .map_or("Not measured".to_owned(), |estimate| {
                    format!("{:.2}%", estimate.reduction_percent)
                });
            let speed = contrast
                .speed
                .estimate
                .as_ref()
                .map_or("Not measured".to_owned(), |estimate| {
                    format!("{:.2}%", estimate.reduction_percent)
                });
            let quality = contrast
                .quality
                .estimate
                .as_ref()
                .map_or("Not measured".to_owned(), |estimate| {
                    format!("{:+.2} pp", estimate.difference_percentage_points)
                });
            markdown.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} | {} |\n",
                contrast.contrast_id,
                token,
                speed,
                quality,
                contrast.tokens.status.as_str(),
                contrast.speed.status.as_str(),
                contrast.quality.status.as_str(),
            ));
        }
    }
    markdown.push_str("\n## Retrieval and lifecycle\n\n");
    markdown.push_str("Retrieval path percentages remain **Not measured** until the calibration/locked evaluator artifacts are attached to this run. Lifecycle reliability is preserved in `lifecycle.jsonl`; healthy silence and MCP not-requested are neutral states, not failures.\n\n");
    markdown.push_str("## Exact recorded commands\n\n");
    let commands = artifacts.commands()?;
    if commands.is_empty() {
        markdown.push_str(
            "No CLI invocation was recorded for this library-generated test artifact.\n\n",
        );
    } else {
        for command in commands {
            markdown.push_str(&format!(
                "### {} — {}\n\n```json\n{}\n```\n\n",
                command.stage,
                command.recorded_at,
                serde_json::to_string(&command.argv)?
            ));
        }
    }
    markdown.push_str("## Caveats\n\n");
    markdown.push_str(&format!(
        "- Overall evaluator statement: {}\n- This compact file contains no raw transcript or credentials. Raw native output remains only in the local evidence directory.\n- Publish claims only when `claimable` is true and every validity gate passes.\n",
        report.statement
    ));
    Ok(markdown)
}

fn markdown_cell(value: &str) -> String {
    value.replace('|', "\\|").replace(['\r', '\n'], " ")
}

fn ratio_goal_row(
    label: &str,
    goal: &super::GoalEstimate<super::RatioOfSumsEstimate>,
    gate: &str,
) -> String {
    let Some(estimate) = &goal.estimate else {
        return format!(
            "| {label} | Not measured | Not measured | Not measured | Not measured | Not measured | Not measured | {gate} | {} |\n",
            goal.status.as_str()
        );
    };
    let harness = |harness| {
        goal.harness_estimates
            .get(&harness)
            .map_or("Not measured".to_owned(), |value| {
                format!("{:.2}%", value.reduction_percent)
            })
    };
    format!(
        "| {label} | {} | {} | {:.2}% | [{:.2}%, {:.2}%] | {} | {} | {gate} | {} |\n",
        estimate.baseline_total,
        estimate.treatment_total,
        estimate.reduction_percent,
        estimate.confidence_low_percent,
        estimate.confidence_high_percent,
        harness(BenchmarkHarness::ClaudeCode),
        harness(BenchmarkHarness::Codex),
        goal.status.as_str(),
    )
}

fn quality_goal_row(
    label: &str,
    goal: &super::GoalEstimate<super::PassRateEstimate>,
    gate: &str,
) -> String {
    let Some(estimate) = &goal.estimate else {
        return format!(
            "| {label} | Not measured | Not measured | Not measured | Not measured | Not measured | Not measured | {gate} | {} |\n",
            goal.status.as_str()
        );
    };
    let harness = |harness| {
        goal.harness_estimates
            .get(&harness)
            .map_or("Not measured".to_owned(), |value| {
                format!("{:+.2} pp", value.difference_percentage_points)
            })
    };
    format!(
        "| {label} | {:.2}% | {:.2}% | {:+.2} pp | [{:+.2}, {:+.2}] pp | {} | {} | {gate} | {} |\n",
        estimate.baseline_pass_fraction * 100.0,
        estimate.treatment_pass_fraction * 100.0,
        estimate.difference_percentage_points,
        estimate.confidence_low * 100.0,
        estimate.confidence_high * 100.0,
        harness(BenchmarkHarness::ClaudeCode),
        harness(BenchmarkHarness::Codex),
        goal.status.as_str(),
    )
}

fn benchmark_summary(
    manifest: &RunManifest,
    report: &TokenBenchmarkReport,
    pairs: &[BenchmarkPair],
    artifacts: &BenchmarkArtifacts,
) -> Result<BenchmarkSummary> {
    let preflight: BenchmarkPreflightReport = artifacts.read_named_json("preflight")?;
    let suite: SuiteManifest =
        serde_json::from_slice(&fs::read(artifacts.run_dir().join("suite.json"))?)?;
    let mut models = BTreeMap::new();
    for (harness, file) in [
        ("claude_code", "configs/claude-c0-native-default.json"),
        ("codex", "configs/codex-c0-native-default.json"),
    ] {
        let config: serde_json::Value =
            serde_json::from_slice(&fs::read(artifacts.run_dir().join(file))?)?;
        if let Some(model) = config.get("model").and_then(serde_json::Value::as_str) {
            models.insert(harness.to_owned(), model.to_owned());
        }
    }
    let pair_audit = pairs
        .iter()
        .map(|pair| PairAudit {
            task_id: pair.task_id.clone(),
            harness: pair.harness,
            repeat: pair.repeat,
            control_tokens: pair.control_tokens,
            treatment_tokens: pair.treatment_tokens,
            control_grade: pair.control_grade,
            treatment_grade: pair.treatment_grade,
            treatment_critical_regression: pair.treatment_critical_regression,
        })
        .collect();
    let harness_versions = ["claude_code", "codex"]
        .into_iter()
        .filter_map(|harness| {
            preflight
                .executable_pins
                .get(harness)
                .map(|pin| (harness.to_owned(), pin.version.clone()))
        })
        .collect();
    Ok(BenchmarkSummary {
        schema_version: 2,
        run_id: manifest.run_id,
        status: report.status,
        statement: report.statement.clone(),
        completed_at: now_string()?,
        overall: report.overall.clone(),
        harnesses: report.harnesses.clone(),
        overall_quality: report.overall_quality.clone(),
        validity_checks: report.validity_checks.clone(),
        metadata: BenchmarkMetadata {
            suite_id: manifest.suite_id.clone(),
            repository_commit: manifest.repository_commit.clone(),
            brain_commit: manifest.brain_commit.clone(),
            frozen_snapshot_sha256: manifest.frozen_snapshot_sha256.clone(),
            repeats: manifest.repeats,
            tasks: suite.tasks.len(),
            models,
            harness_versions,
        },
        pair_audit,
        three_goal: report.three_goal.clone(),
    })
}

fn planned_blocks_per_harness(matrix: &[PlannedSample]) -> usize {
    BenchmarkHarness::ALL
        .into_iter()
        .map(|harness| {
            matrix
                .iter()
                .filter(|sample| sample.harness == harness)
                .map(|sample| (&sample.task_id, sample.repeat))
                .collect::<BTreeSet<_>>()
                .len()
        })
        .min()
        .unwrap_or(0)
}

fn build_observations(
    matrix: &[PlannedSample],
    samples: &BTreeMap<&str, &SampleRecord>,
    grades: &BTreeMap<String, &GradeRecord>,
    suite: &SuiteManifest,
) -> Vec<BenchmarkObservation> {
    let strata = suite
        .tasks
        .iter()
        .map(|task| (task.id.as_str(), task.stratum))
        .collect::<BTreeMap<_, _>>();
    matrix
        .iter()
        .filter_map(|planned| {
            let sample = *samples.get(planned.sample_id.as_str())?;
            let grade = *grades.get(&planned.sample_id)?;
            let error = sample.error.as_deref().unwrap_or_default().to_lowercase();
            Some(BenchmarkObservation {
                task_id: planned.task_id.clone(),
                harness: planned.harness,
                repeat: planned.repeat,
                stratum: *strata.get(planned.task_id.as_str())?,
                condition: planned.condition,
                native_tokens: sample.native_usage.as_ref().map(|usage| usage.total_tokens),
                elapsed_ms: sample.elapsed_ms,
                grade: grade.outcome,
                critical_regression: grade.critical_regression,
                timed_out: error.contains("timed out") || error.contains("timeout"),
                harness_failed: sample.status == SampleStatus::HarnessFailure,
                brain_mcp_calls: sample
                    .native_trace
                    .as_ref()
                    .map_or(0, |trace| trace.brain_mcp_calls),
            })
        })
        .collect()
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

pub(super) fn production_config_hashes() -> Result<ProductionConfigHashes> {
    let profile = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .context("USERPROFILE is unavailable")?;
    Ok(ProductionConfigHashes {
        claude_settings: hash_optional_file(&profile.join(".claude/settings.json"))?,
        claude_mcp: hash_optional_file(&profile.join(".claude.json"))?,
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
