use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use brain_domain::{HOOK_PROTOCOL_VERSION, Harness, HookEnvelope, ProjectId};
use brain_service::ServiceLaunchConfig;
use brain_store::{
    EventLedger, LifecycleChannel, LifecycleEvent, LifecycleStage, RetrievalDecision,
    TelemetryQuery,
};
use sha2::{Digest, Sha256};
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use super::{
    BenchmarkArtifacts, BenchmarkHarness, BenchmarkPreflightReport, BenchmarkRunPreview,
    CommandPlan, ExecutablePin, ExecutionTemplate, ExecutionTemplates, PlannedSample,
    ProcessOutput, ProcessRunner, SampleRecord, SampleStatus, SuiteManifest, SystemProcessRunner,
    TraceMarkers, execute_command_plan, parse_claude_usage, parse_codex_usage, parse_native_trace,
};

/// Execute the immutable matrix. The public entry point always uses the real process runner; the
/// generic form exists so the zero-cost suite can prove ordering, retries and normalization.
pub fn execute_benchmark_run(
    brain_home: &Path,
    project_id: ProjectId,
    run_id: Uuid,
) -> Result<BenchmarkRunPreview> {
    execute_benchmark_run_with(brain_home, project_id, run_id, &SystemProcessRunner)
}

pub fn execute_benchmark_run_with(
    brain_home: &Path,
    project_id: ProjectId,
    run_id: Uuid,
    runner: &impl ProcessRunner,
) -> Result<BenchmarkRunPreview> {
    let artifacts = BenchmarkArtifacts::new(brain_home, project_id, run_id)?;
    let manifest = artifacts.manifest()?;
    let preflight: BenchmarkPreflightReport = artifacts.read_named_json("preflight")?;
    ensure!(preflight.valid, "benchmark preflight is invalid");
    let templates_path = artifacts.run_dir().join("execution-templates.json");
    let template_bytes = fs::read(&templates_path)?;
    ensure!(
        sha256(&template_bytes) == preflight.execution_templates_sha256,
        "execution templates changed after preflight"
    );
    let templates: ExecutionTemplates = serde_json::from_slice(&template_bytes)?;
    verify_executable_pins(&preflight.executable_pins)?;
    ensure_runtime_isolation(brain_home, &preflight)?;
    verify_frozen_snapshot(project_id, &preflight)?;
    ensure!(
        production_config_hashes()? == preflight.production_config_hashes,
        "production configuration changed after preflight"
    );

    let suite: SuiteManifest =
        serde_json::from_slice(&fs::read(artifacts.run_dir().join("suite.json"))?)?;
    let tasks: BTreeMap<_, _> = suite
        .tasks
        .iter()
        .map(|task| (task.id.as_str(), task))
        .collect();
    let mut paid_sessions_launched = 0u32;
    let mut samples = artifacts.samples()?;
    let mut completed: BTreeSet<String> = samples
        .iter()
        .filter(|sample| sample.status == SampleStatus::Completed)
        .map(|sample| sample.sample.sample_id.clone())
        .collect();

    for planned in &manifest.matrix {
        if completed.contains(&planned.sample_id) {
            continue;
        }
        let task = tasks
            .get(planned.task_id.as_str())
            .with_context(|| format!("unknown task {}", planned.task_id))?;
        let first_attempt = samples
            .iter()
            .filter(|record| record.sample.sample_id == planned.sample_id)
            .map(|record| record.attempt)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        for attempt in first_attempt..=templates.max_attempts {
            let sample_root = artifacts
                .run_dir()
                .join("attempts")
                .join(&planned.sample_id)
                .join(format!("{attempt}-{}", Uuid::now_v7()));
            fs::create_dir_all(&sample_root)?;
            let checkout = materialize_sample_checkout(
                &preflight.checkout,
                &sample_root.join("checkout"),
                &manifest.repository_commit,
            )?;
            sanitize_sample_checkout(&checkout, &artifacts.run_dir().join("instructions"))?;
            ensure_checkout_is_unregistered(brain_home, &checkout)?;
            let (sample_brain_home, pipe_name, _service) = if planned
                .condition
                .requires_brain_service()
            {
                let prepared =
                    prepare_sample_brain(&preflight, &sample_root, &checkout, planned, attempt)?;
                let service = start_sample_service(
                    &templates.brain_service_program,
                    &prepared.0,
                    &sample_root.join("logs"),
                    &prepared.1,
                    &checkout,
                )?;
                (prepared.0, prepared.1, Some(service))
            } else {
                (
                    sample_root.join("brain-off"),
                    format!(r"\\.\pipe\agent-brain-disabled-{}", Uuid::now_v7()),
                    None,
                )
            };
            let condition_config = condition_config_path(&artifacts, planned);
            let sample_harness_home = sample_root.join("harness-home");
            let sample_codegraph_home = sample_root.join("codegraph-home");
            fs::create_dir_all(&sample_harness_home)?;
            fs::create_dir_all(&sample_codegraph_home)?;
            let plan = command_plan(PlanContext {
                template: templates.template(planned.harness, planned.condition),
                launcher_environment: &templates.launcher_environment,
                sample: planned,
                prompt: &task.prompt,
                max_turns: task.max_turns,
                max_tool_calls: task.max_tool_calls,
                checkout: &checkout,
                sample_root: &sample_root,
                sample_brain_home: &sample_brain_home,
                sample_harness_home: &sample_harness_home,
                sample_codegraph_home: &sample_codegraph_home,
                pipe_name: &pipe_name,
                condition_config: &condition_config,
            })?;
            let launched_at = time::OffsetDateTime::now_utc();
            paid_sessions_launched = paid_sessions_launched.saturating_add(1);
            let output = match execute_command_plan(&plan, true, runner) {
                Ok(super::RunOutcome::Executed(output)) => output,
                Ok(super::RunOutcome::DryRun(_)) => unreachable!("execute=true"),
                Err(error) => ProcessOutput {
                    exit_code: None,
                    stdout: Vec::new(),
                    stderr: error.to_string().into_bytes(),
                    timed_out: false,
                    elapsed_ms: None,
                },
            };
            scrub_sample_credentials(&sample_harness_home)?;
            let condition_exposure = if planned.condition.requires_brain_service() {
                condition_lifecycle_observed(
                    &artifacts,
                    &sample_brain_home,
                    project_id,
                    planned.harness,
                    planned.condition,
                    launched_at,
                )?
            } else {
                ConditionExposure {
                    valid: true,
                    detail: "condition has no Brain endpoint by design".to_owned(),
                }
            };
            drop(_service);
            let mut record = normalize_process_output(
                &artifacts,
                planned,
                attempt,
                output,
                task.trace_markers.as_ref(),
            )?;
            enforce_sample_limits(&mut record, task.max_turns, task.max_tool_calls);
            if !condition_exposure.valid {
                record.status = SampleStatus::HarnessFailure;
                record.error = Some(condition_exposure.detail);
            }
            artifacts.append_sample(&record)?;
            samples.push(record.clone());
            ensure!(
                production_config_hashes()? == preflight.production_config_hashes,
                "production configuration changed during sample {}",
                planned.sample_id
            );
            verify_frozen_snapshot(project_id, &preflight)?;
            if record.status == SampleStatus::Completed {
                completed.insert(planned.sample_id.clone());
                break;
            }
        }
    }

    Ok(BenchmarkRunPreview {
        schema_version: 2,
        run_id,
        execute: true,
        planned_samples: manifest.matrix.len(),
        completed_samples: completed.len(),
        remaining_samples: manifest.matrix.len().saturating_sub(completed.len()),
        paid_sessions_launched,
        artifact_directory: artifacts.run_dir().to_path_buf(),
    })
}

struct PlanContext<'a> {
    template: &'a ExecutionTemplate,
    launcher_environment: &'a BTreeMap<String, String>,
    sample: &'a PlannedSample,
    prompt: &'a str,
    max_turns: u32,
    max_tool_calls: u32,
    checkout: &'a Path,
    sample_root: &'a Path,
    sample_brain_home: &'a Path,
    sample_harness_home: &'a Path,
    sample_codegraph_home: &'a Path,
    pipe_name: &'a str,
    condition_config: &'a Path,
}

fn command_plan(context: PlanContext<'_>) -> Result<CommandPlan> {
    let replacements = BTreeMap::from([
        ("{{prompt}}", context.prompt.to_owned()),
        (
            "{{checkout}}",
            context.checkout.to_string_lossy().to_string(),
        ),
        (
            "{{sample_root}}",
            context.sample_root.to_string_lossy().to_string(),
        ),
        (
            "{{sample_brain_home}}",
            context.sample_brain_home.to_string_lossy().to_string(),
        ),
        (
            "{{sample_harness_home}}",
            context.sample_harness_home.to_string_lossy().to_string(),
        ),
        (
            "{{sample_codegraph_home}}",
            context.sample_codegraph_home.to_string_lossy().to_string(),
        ),
        ("{{pipe_name}}", context.pipe_name.to_owned()),
        (
            "{{condition_config}}",
            context.condition_config.to_string_lossy().to_string(),
        ),
        ("{{sample_id}}", context.sample.sample_id.clone()),
        ("{{max_turns}}", context.max_turns.to_string()),
        ("{{max_tool_calls}}", context.max_tool_calls.to_string()),
    ]);
    let expand = |value: &str| {
        let mut expanded = value.to_owned();
        for (placeholder, replacement) in &replacements {
            expanded = expanded.replace(placeholder, replacement);
        }
        ensure!(
            !expanded.contains("{{"),
            "unresolved execution placeholder in {value}"
        );
        Ok(expanded)
    };
    let args = context
        .template
        .args
        .iter()
        .map(|argument| expand(argument))
        .collect::<Result<Vec<_>>>()?;
    let environment = context
        .launcher_environment
        .iter()
        .chain(context.template.environment.iter())
        .map(|(key, value)| Ok((key.clone(), expand(value)?)))
        .collect::<Result<BTreeMap<_, _>>>()?;
    Ok(CommandPlan {
        sample_id: context.sample.sample_id.clone(),
        program: context.template.program.clone(),
        args,
        current_dir: context.checkout.to_path_buf(),
        environment,
        timeout_seconds: context.template.timeout_seconds,
    })
}

fn normalize_process_output(
    artifacts: &BenchmarkArtifacts,
    sample: &PlannedSample,
    attempt: u32,
    output: ProcessOutput,
    trace_markers: Option<&TraceMarkers>,
) -> Result<SampleRecord> {
    let stdout = artifacts.write_raw(
        &sample.sample_id,
        &format!("stdout-attempt-{attempt}"),
        &output.stdout,
    )?;
    let stderr = artifacts.write_raw(
        &sample.sample_id,
        &format!("stderr-attempt-{attempt}"),
        &output.stderr,
    )?;
    let raw = String::from_utf8_lossy(&output.stdout);
    let native_trace = parse_native_trace(sample.harness, &raw, trace_markers).ok();
    let process_error = if output.timed_out {
        Some("harness timed out".to_owned())
    } else if output.exit_code != Some(0) {
        Some(format!("harness exited with {:?}", output.exit_code))
    } else {
        None
    };
    let (status, usage, error) = if let Some(error) = process_error {
        (SampleStatus::HarnessFailure, None, Some(error))
    } else {
        match sample.harness {
            BenchmarkHarness::ClaudeCode => parse_claude_usage(&raw),
            BenchmarkHarness::Codex => parse_codex_usage(&raw),
        }
        .map(|usage| (SampleStatus::Completed, Some(usage), None))
        .unwrap_or_else(|error| (SampleStatus::InvalidUsage, None, Some(error.to_string())))
    };
    Ok(SampleRecord {
        schema_version: 2,
        sample: sample.clone(),
        status,
        attempt,
        native_usage: usage,
        elapsed_ms: output.elapsed_ms,
        native_trace,
        answer: extract_answer(sample.harness, &raw),
        automated_test_passed: None,
        stdout_sha256: stdout.sha256,
        stderr_sha256: stderr.sha256,
        error,
        completed_at: time::OffsetDateTime::now_utc().format(&Rfc3339)?,
    })
}

fn extract_answer(harness: BenchmarkHarness, raw: &str) -> String {
    match harness {
        BenchmarkHarness::ClaudeCode => {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) {
                return value
                    .get("result")
                    .and_then(|result| result.as_str())
                    .map(str::to_owned)
                    .unwrap_or_default();
            }
            raw.lines()
                .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
                .filter(|value| value.get("type").and_then(|kind| kind.as_str()) == Some("result"))
                .filter_map(|value| value.get("result")?.as_str().map(str::to_owned))
                .next_back()
                .unwrap_or_default()
        }
        BenchmarkHarness::Codex => raw
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .filter_map(|value| {
                value
                    .get("item")
                    .filter(|item| {
                        item.get("type").and_then(|value| value.as_str()) == Some("agent_message")
                    })
                    .and_then(|item| item.get("text"))
                    .and_then(|text| text.as_str())
                    .map(str::to_owned)
                    .or_else(|| {
                        value
                            .get("payload")
                            .filter(|payload| {
                                payload.get("type").and_then(|value| value.as_str())
                                    == Some("agent_message")
                            })
                            .and_then(|payload| payload.get("message"))
                            .and_then(|text| text.as_str())
                            .map(str::to_owned)
                    })
            })
            .next_back()
            .unwrap_or_default(),
    }
}

fn condition_config_path(artifacts: &BenchmarkArtifacts, sample: &PlannedSample) -> PathBuf {
    artifacts
        .run_dir()
        .join("configs")
        .join(super::preflight::condition_profile_name(
            sample.harness,
            sample.condition,
        ))
}

fn materialize_sample_checkout(source: &Path, target: &Path, commit: &str) -> Result<PathBuf> {
    ensure!(!target.exists(), "sample checkout already exists");
    let parent = target.parent().context("sample checkout has no parent")?;
    fs::create_dir_all(parent)?;
    let cloned = Command::new("git")
        .args(["clone", "--no-hardlinks", "--no-checkout"])
        .arg(source)
        .arg(target)
        .status()?;
    ensure!(cloned.success(), "sample git clone failed");
    let checked_out = Command::new("git")
        .current_dir(target)
        .args(["checkout", "--detach", commit])
        .status()?;
    ensure!(checked_out.success(), "sample git checkout failed");
    target.canonicalize().context("resolve sample checkout")
}

fn sanitize_sample_checkout(checkout: &Path, instructions: &Path) -> Result<()> {
    ensure!(checkout.is_dir(), "sample checkout is missing");
    for relative in [
        ".mcp.json",
        ".claude.json",
        ".claude/settings.json",
        ".claude/settings.local.json",
        ".claude/hooks.json",
        ".codex/config.toml",
        ".codex/hooks.json",
    ] {
        let target = checkout.join(relative);
        if target.is_file() {
            fs::remove_file(&target)
                .with_context(|| format!("remove inherited integration {}", target.display()))?;
        }
    }
    let inherited_codegraph = checkout.join(".codegraph");
    if inherited_codegraph.is_dir() {
        fs::remove_dir_all(&inherited_codegraph).with_context(|| {
            format!(
                "remove inherited CodeGraph state {}",
                inherited_codegraph.display()
            )
        })?;
    }
    for name in ["AGENTS.md", "CLAUDE.md"] {
        let source = instructions.join(name);
        ensure!(
            source.is_file(),
            "archived benchmark instruction is missing"
        );
        fs::copy(&source, checkout.join(name))
            .with_context(|| format!("overlay neutral {name}"))?;
    }
    Ok(())
}

fn scrub_sample_credentials(harness_home: &Path) -> Result<()> {
    for name in [".credentials.json", "auth.json"] {
        let path = harness_home.join(name);
        if path.is_file() {
            fs::remove_file(&path)
                .with_context(|| format!("scrub sample credential {}", path.display()))?;
        }
    }
    Ok(())
}

fn enforce_sample_limits(record: &mut SampleRecord, max_turns: u32, max_tool_calls: u32) {
    if record.status != SampleStatus::Completed {
        return;
    }
    let Some(trace) = &record.native_trace else {
        record.status = SampleStatus::HarnessFailure;
        record.error =
            Some("native trace was unavailable; sample limits cannot be audited".to_owned());
        return;
    };
    let mut violations = Vec::new();
    if trace.turns > max_turns {
        violations.push(format!("{} turns exceeded limit {max_turns}", trace.turns));
    }
    if trace.total_tool_calls > max_tool_calls {
        violations.push(format!(
            "{} tool calls exceeded limit {max_tool_calls}",
            trace.total_tool_calls
        ));
    }
    if !violations.is_empty() {
        record.status = SampleStatus::HarnessFailure;
        record.error = Some(format!(
            "benchmark bound violation: {}",
            violations.join("; ")
        ));
    }
}

fn prepare_sample_brain(
    preflight: &BenchmarkPreflightReport,
    sample_root: &Path,
    checkout: &Path,
    sample: &PlannedSample,
    attempt: u32,
) -> Result<(PathBuf, String)> {
    let target = sample_root.join("brain-home");
    copy_tree_writable(&preflight.frozen_brain_home, &target)?;
    let source_config_path = ServiceLaunchConfig::default_path(&preflight.frozen_brain_home);
    let mut config = ServiceLaunchConfig::load(&source_config_path)?;
    let pipe_name = format!(
        r"\\.\pipe\agent-brain-benchmark-{}-{attempt}",
        sample.sample_id
    );
    config.pipe_name = pipe_name.clone();
    for project in &mut config.projects {
        let relative = project
            .ledger_path
            .strip_prefix(&preflight.frozen_brain_home)
            .context("frozen ledger escaped frozen brain home")?;
        project.ledger_path = target.join(relative);
        project.project_root = checkout.to_path_buf();
        project.claude_sources.clear();
        project.codex_sources.clear();
    }
    let config_path = ServiceLaunchConfig::default_path(&target);
    fs::create_dir_all(config_path.parent().context("service config parent")?)?;
    fs::write(config_path, serde_json::to_vec_pretty(&config)?)?;
    Ok((target, pipe_name))
}

struct ServiceGuard(Child);

impl Drop for ServiceGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start_sample_service(
    program: &Path,
    brain_home: &Path,
    log_dir: &Path,
    pipe_name: &str,
    checkout: &Path,
) -> Result<ServiceGuard> {
    fs::create_dir_all(log_dir)?;
    let child = Command::new(program)
        .arg("--brain-home")
        .arg(brain_home)
        .arg("--log-dir")
        .arg(log_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("launch benchmark service {}", program.display()))?;
    let guard = ServiceGuard(child);
    let envelope = HookEnvelope {
        protocol: HOOK_PROTOCOL_VERSION,
        harness: Harness::Other("benchmark-probe".to_owned()),
        event_name: "BenchmarkProbe".to_owned(),
        received_at: time::OffsetDateTime::now_utc(),
        nonce: Uuid::now_v7(),
        payload: serde_json::json!({ "cwd": checkout }),
    };
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(brain_hook::protocol::request(
            pipe_name,
            &envelope,
            Duration::from_secs(20),
        ))
        .context("benchmark brain endpoint did not become ready")?;
    Ok(guard)
}

struct ConditionExposure {
    valid: bool,
    detail: String,
}

fn condition_lifecycle_observed(
    artifacts: &BenchmarkArtifacts,
    brain_home: &Path,
    project_id: ProjectId,
    harness: BenchmarkHarness,
    condition: super::BenchmarkCondition,
    since: time::OffsetDateTime,
) -> Result<ConditionExposure> {
    let config = ServiceLaunchConfig::load(ServiceLaunchConfig::default_path(brain_home))?;
    let project = config.project(Some(project_id))?;
    let ledger = EventLedger::open(&project.ledger_path, project_id)?;
    let query = TelemetryQuery {
        project_id,
        session: None,
        start: since,
        end: time::OffsetDateTime::now_utc() + time::Duration::seconds(1),
        limit: 10_000,
    };
    let events = ledger.lifecycle_events(&query)?;
    let decisions = ledger.retrieval_decisions(&query)?;
    for event in &events {
        artifacts.append_lifecycle_record(
            &format!("lifecycle:{}", event.event_id),
            "lifecycle_event",
            event,
        )?;
    }
    for decision in &decisions {
        artifacts.append_lifecycle_record(
            &format!("retrieval:{}", decision.decision_id),
            "retrieval_decision",
            decision,
        )?;
    }
    let expected = match harness {
        BenchmarkHarness::ClaudeCode => Harness::ClaudeCode,
        BenchmarkHarness::Codex => Harness::Codex,
    };
    Ok(validate_condition_exposure(
        condition, &expected, &events, &decisions,
    ))
}

fn validate_condition_exposure(
    condition: super::BenchmarkCondition,
    harness: &Harness,
    events: &[LifecycleEvent],
    decisions: &[RetrievalDecision],
) -> ConditionExposure {
    let event = |channel, stage| {
        events.iter().any(|event| {
            &event.harness == harness && event.channel == channel && event.stage == stage
        })
    };
    let decision = |channel| {
        decisions
            .iter()
            .any(|decision| &decision.harness == harness && decision.channel == channel)
    };
    let startup = event(LifecycleChannel::SessionStart, LifecycleStage::HookReceived)
        && event(LifecycleChannel::SessionStart, LifecycleStage::ReplyFlushed)
        && decision(LifecycleChannel::SessionStart);
    let ended = event(
        LifecycleChannel::SessionEnd,
        LifecycleStage::SessionEndPersisted,
    );
    let prompt = event(
        LifecycleChannel::UserPromptSubmit,
        LifecycleStage::HookReceived,
    ) && decision(LifecycleChannel::UserPromptSubmit);
    let unexpected_prompt = condition == super::BenchmarkCondition::C2
        && events.iter().any(|event| {
            &event.harness == harness && event.channel == LifecycleChannel::UserPromptSubmit
        });
    let unexpected_mcp = condition != super::BenchmarkCondition::C4
        && events
            .iter()
            .any(|event| &event.harness == harness && event.channel == LifecycleChannel::BrainMcp);
    let mcp_terminal_complete = events
        .iter()
        .filter(|event| {
            &event.harness == harness
                && event.channel == LifecycleChannel::BrainMcp
                && event.stage == LifecycleStage::McpRequest
        })
        .all(|request| {
            events.iter().any(|terminal| {
                &terminal.harness == harness
                    && terminal.channel == LifecycleChannel::BrainMcp
                    && matches!(
                        &terminal.stage,
                        LifecycleStage::McpSucceeded | LifecycleStage::McpFailed
                    )
                    && terminal.correlation_id == request.correlation_id
            })
        });
    let prompt_expected = matches!(
        condition,
        super::BenchmarkCondition::C3 | super::BenchmarkCondition::C4
    );
    let valid = startup
        && ended
        && (!prompt_expected || prompt)
        && !unexpected_prompt
        && !unexpected_mcp
        && mcp_terminal_complete;
    ConditionExposure {
        valid,
        detail: if valid {
            "condition-specific lifecycle receipts observed".to_owned()
        } else {
            format!(
                "condition exposure mismatch: startup={startup}, session_end={ended}, prompt={prompt}, unexpected_prompt={unexpected_prompt}, unexpected_mcp={unexpected_mcp}, mcp_terminal_complete={mcp_terminal_complete}"
            )
        },
    }
}

fn copy_tree_writable(source: &Path, target: &Path) -> Result<()> {
    ensure!(source.is_dir(), "frozen brain home is missing");
    ensure!(!target.exists(), "sample brain home already exists");
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let destination = target.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree_writable(&entry.path(), &destination)?;
        } else if entry.file_type()?.is_file() {
            fs::copy(entry.path(), &destination)?;
            make_file_writable(&destination)?;
        }
    }
    Ok(())
}

#[cfg(windows)]
#[allow(clippy::permissions_set_readonly_false)]
fn make_file_writable(path: &Path) -> Result<()> {
    // The benchmark transport is Windows named pipes. Here this clears the Windows read-only file
    // attribute; it does not grant broad Unix write permissions.
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_readonly(false);
    fs::set_permissions(path, permissions)?;
    Ok(())
}

#[cfg(unix)]
fn make_file_writable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = fs::metadata(path)?;
    let mut permissions = metadata.permissions();
    permissions.set_mode(permissions.mode() | 0o200);
    fs::set_permissions(path, permissions)?;
    Ok(())
}

fn ensure_runtime_isolation(brain_home: &Path, preflight: &BenchmarkPreflightReport) -> Result<()> {
    let production = ServiceLaunchConfig::load(ServiceLaunchConfig::default_path(brain_home))?;
    ensure!(
        preflight.pipe_name != production.pipe_name,
        "benchmark endpoint is the production endpoint"
    );
    let run_root = preflight
        .frozen_brain_home
        .parent()
        .context("frozen brain home has no run root")?
        .canonicalize()?;
    let production_home = brain_home.canonicalize()?;
    ensure!(
        run_root.starts_with(&production_home) && run_root != production_home,
        "benchmark artifacts are not scoped beneath BRAIN_HOME"
    );
    Ok(())
}

fn ensure_checkout_is_unregistered(brain_home: &Path, checkout: &Path) -> Result<()> {
    let production = ServiceLaunchConfig::load(ServiceLaunchConfig::default_path(brain_home))?;
    for project in production.projects {
        if let Ok(root) = project.project_root.canonicalize() {
            ensure!(
                !checkout.starts_with(&root) && !root.starts_with(checkout),
                "benchmark checkout overlaps registered project {}",
                root.display()
            );
        }
    }
    Ok(())
}

fn verify_frozen_snapshot(
    project_id: ProjectId,
    preflight: &BenchmarkPreflightReport,
) -> Result<()> {
    let project = preflight
        .frozen_brain_home
        .join("projects")
        .join(project_id.0.to_string());
    ensure!(
        hash_tree(&project)? == preflight.snapshot_sha256,
        "frozen brain snapshot changed after preflight"
    );
    Ok(())
}

fn hash_tree(root: &Path) -> Result<String> {
    let mut files = Vec::new();
    collect_files(root, root, &mut files)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));
    let mut digest = Sha256::new();
    for (relative, path) in files {
        digest.update(relative.to_string_lossy().as_bytes());
        digest.update([0]);
        digest.update(fs::read(path)?);
    }
    Ok(hex::encode(digest.finalize()))
}

fn collect_files(root: &Path, directory: &Path, files: &mut Vec<(PathBuf, PathBuf)>) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            collect_files(root, &entry.path(), files)?;
        } else if entry.file_type()?.is_file() {
            files.push((entry.path().strip_prefix(root)?.to_path_buf(), entry.path()));
        }
    }
    Ok(())
}

fn verify_executable_pins(pins: &BTreeMap<String, ExecutablePin>) -> Result<()> {
    for (name, pin) in pins {
        ensure!(pin.path.is_file(), "pinned {name} executable is missing");
        ensure!(
            sha256(&fs::read(&pin.path)?) == pin.sha256,
            "pinned {name} executable changed after preflight"
        );
    }
    Ok(())
}

fn production_config_hashes() -> Result<super::ProductionConfigHashes> {
    let profile = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .context("USERPROFILE is unavailable")?;
    Ok(super::ProductionConfigHashes {
        claude_settings: super::hash_optional_file(&profile.join(".claude/settings.json"))?,
        claude_mcp: super::hash_optional_file(&profile.join(".claude.json"))?,
        codex_hooks: super::hash_optional_file(&profile.join(".codex/hooks.json"))?,
        codex_config: super::hash_optional_file(&profile.join(".codex/config.toml"))?,
    })
}

fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use brain_domain::{Harness, ProjectId};
    use brain_store::{
        LifecycleChannel, LifecycleEvent, LifecycleStage, RetrievalDecision, RetrievalOutcome,
        RetrievalReasonCode, SessionAttribution,
    };
    use std::collections::BTreeMap;

    use crate::token_benchmark::BenchmarkCondition;

    use super::{
        BenchmarkArtifacts, BenchmarkHarness, ExecutionTemplate, PlanContext, PlannedSample,
        ProcessOutput, SampleStatus, command_plan, enforce_sample_limits, normalize_process_output,
        sanitize_sample_checkout, validate_condition_exposure,
    };

    fn sample(harness: BenchmarkHarness) -> PlannedSample {
        PlannedSample {
            sample_id: format!("{}-sample", harness.as_str()),
            pair_id: "pair".to_owned(),
            task_id: "task".to_owned(),
            harness,
            repeat: 1,
            condition: BenchmarkCondition::BrainOff,
            order: 0,
        }
    }

    #[test]
    fn command_profile_expands_only_benchmark_scoped_values() {
        let temp = tempfile::tempdir().expect("temp");
        let executable = temp.path().join("launcher.exe");
        std::fs::write(&executable, b"fixture").expect("launcher");
        let template = ExecutionTemplate {
            program: executable,
            args: vec![
                "--prompt={{prompt}}".to_owned(),
                "--config={{condition_config}}".to_owned(),
                "--turns={{max_turns}}".to_owned(),
            ],
            environment: BTreeMap::new(),
            timeout_seconds: 30,
        };
        let checkout = temp.path().to_path_buf();
        let planned = sample(BenchmarkHarness::ClaudeCode);
        let sample_brain_home = temp.path().join("brain");
        let sample_harness_home = temp.path().join("harness");
        let sample_codegraph_home = temp.path().join("codegraph");
        let condition_config = temp.path().join("control.json");
        let plan = command_plan(PlanContext {
            template: &template,
            launcher_environment: &BTreeMap::new(),
            sample: &planned,
            prompt: "bounded prompt",
            max_turns: 8,
            max_tool_calls: 20,
            checkout: &checkout,
            sample_root: temp.path(),
            sample_brain_home: &sample_brain_home,
            sample_harness_home: &sample_harness_home,
            sample_codegraph_home: &sample_codegraph_home,
            pipe_name: r"\\.\pipe\fixture",
            condition_config: &condition_config,
        })
        .expect("plan");
        assert!(plan.args.iter().any(|arg| arg == "--prompt=bounded prompt"));
        assert!(plan.args.iter().any(|arg| arg == "--turns=8"));
    }

    #[test]
    fn native_outputs_are_preserved_before_normalization() {
        let temp = tempfile::tempdir().expect("temp");
        let artifacts = BenchmarkArtifacts::new(
            temp.path(),
            ProjectId(uuid::Uuid::now_v7()),
            uuid::Uuid::now_v7(),
        )
        .expect("artifacts");
        let claude = normalize_process_output(
            &artifacts,
            &sample(BenchmarkHarness::ClaudeCode),
            1,
            ProcessOutput {
                exit_code: Some(0),
                stdout: br#"{"result":"answer","usage":{"input_tokens":100,"cache_creation_input_tokens":20,"cache_read_input_tokens":300,"output_tokens":40}}"#.to_vec(),
                stderr: Vec::new(),
                timed_out: false,
                elapsed_ms: Some(125),
            },
            None,
        )
        .expect("Claude record");
        assert_eq!(claude.status, SampleStatus::Completed);
        assert_eq!(claude.answer, "answer");
        assert_eq!(claude.elapsed_ms, Some(125));
        assert!(claude.native_trace.is_some());
        assert_eq!(claude.native_usage.expect("usage").total_tokens, 460);

        let codex = normalize_process_output(
            &artifacts,
            &sample(BenchmarkHarness::Codex),
            1,
            ProcessOutput {
                exit_code: Some(0),
                stdout: concat!(
                    "{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"codex answer\"}}\n",
                    "{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"total_token_usage\":{\"input_tokens\":700,\"cached_input_tokens\":500,\"output_tokens\":200,\"total_tokens\":900}}}}\n"
                )
                .as_bytes()
                .to_vec(),
                stderr: Vec::new(),
                timed_out: false,
                elapsed_ms: Some(225),
            },
            None,
        )
        .expect("Codex record");
        assert_eq!(codex.status, SampleStatus::Completed);
        assert_eq!(codex.answer, "codex answer");
        assert_eq!(codex.elapsed_ms, Some(225));
        assert!(codex.native_trace.is_some());
        assert_eq!(codex.native_usage.expect("usage").total_tokens, 900);
        assert!(artifacts.run_dir().join("raw").is_dir());
    }

    #[test]
    fn malformed_native_usage_remains_visible_and_retryable() {
        let temp = tempfile::tempdir().expect("temp");
        let artifacts = BenchmarkArtifacts::new(
            temp.path(),
            ProjectId(uuid::Uuid::now_v7()),
            uuid::Uuid::now_v7(),
        )
        .expect("artifacts");
        let record = normalize_process_output(
            &artifacts,
            &sample(BenchmarkHarness::ClaudeCode),
            2,
            ProcessOutput {
                exit_code: Some(0),
                stdout: br#"{"result":"short but unmetered"}"#.to_vec(),
                stderr: Vec::new(),
                timed_out: false,
                elapsed_ms: Some(50),
            },
            None,
        )
        .expect("record");
        assert_eq!(record.status, SampleStatus::InvalidUsage);
        assert_eq!(record.attempt, 2);
        assert!(record.error.expect("error").contains("usage"));
    }

    #[test]
    fn neutral_instructions_replace_inherited_agent_integrations() {
        let temp = tempfile::tempdir().expect("temp");
        let checkout = temp.path().join("checkout");
        let instructions = temp.path().join("instructions");
        std::fs::create_dir_all(checkout.join(".claude")).expect("claude dir");
        std::fs::create_dir_all(checkout.join(".codex")).expect("codex dir");
        std::fs::create_dir_all(checkout.join(".codegraph")).expect("codegraph dir");
        std::fs::create_dir_all(&instructions).expect("instructions");
        std::fs::write(checkout.join(".mcp.json"), "inherited").expect("mcp");
        std::fs::write(checkout.join(".claude/settings.json"), "inherited").expect("claude");
        std::fs::write(checkout.join(".codex/config.toml"), "inherited").expect("codex");
        std::fs::write(instructions.join("AGENTS.md"), "neutral").expect("agents");
        std::fs::write(instructions.join("CLAUDE.md"), "neutral").expect("claude instructions");

        sanitize_sample_checkout(&checkout, &instructions).expect("sanitize");

        assert!(!checkout.join(".mcp.json").exists());
        assert!(!checkout.join(".claude/settings.json").exists());
        assert!(!checkout.join(".codex/config.toml").exists());
        assert!(!checkout.join(".codegraph").exists());
        assert_eq!(
            std::fs::read_to_string(checkout.join("AGENTS.md")).unwrap(),
            "neutral"
        );
        assert_eq!(
            std::fs::read_to_string(checkout.join("CLAUDE.md")).unwrap(),
            "neutral"
        );
    }

    #[test]
    fn samples_over_the_native_tool_bound_are_invalidated() {
        let temp = tempfile::tempdir().expect("temp");
        let artifacts = BenchmarkArtifacts::new(
            temp.path(),
            ProjectId(uuid::Uuid::now_v7()),
            uuid::Uuid::now_v7(),
        )
        .expect("artifacts");
        let mut record = normalize_process_output(
            &artifacts,
            &sample(BenchmarkHarness::ClaudeCode),
            1,
            ProcessOutput {
                exit_code: Some(0),
                stdout: concat!(
                    "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"tool_use\",\"name\":\"Read\",\"input\":{\"path\":\"a\"}}]}}\n",
                    "{\"type\":\"result\",\"result\":\"answer\",\"usage\":{\"input_tokens\":1,\"cache_creation_input_tokens\":1,\"cache_read_input_tokens\":1,\"output_tokens\":1}}\n"
                )
                .as_bytes()
                .to_vec(),
                stderr: Vec::new(),
                timed_out: false,
                elapsed_ms: Some(1),
            },
            None,
        )
        .expect("record");
        enforce_sample_limits(&mut record, 1, 0);
        assert_eq!(record.status, SampleStatus::HarnessFailure);
        assert!(record.error.unwrap().contains("tool calls"));
    }

    #[test]
    fn healthy_silence_is_valid_condition_exposure_but_a_missing_prompt_receipt_is_not() {
        let project_id = ProjectId(uuid::Uuid::now_v7());
        let harness = Harness::Codex;
        let mut events = vec![
            lifecycle(
                project_id,
                LifecycleChannel::SessionStart,
                LifecycleStage::HookReceived,
            ),
            lifecycle(
                project_id,
                LifecycleChannel::SessionStart,
                LifecycleStage::ReplyFlushed,
            ),
            lifecycle(
                project_id,
                LifecycleChannel::SessionEnd,
                LifecycleStage::SessionEndPersisted,
            ),
        ];
        let mut decisions = vec![decision(project_id, LifecycleChannel::SessionStart)];
        assert!(
            validate_condition_exposure(BenchmarkCondition::C2, &harness, &events, &decisions,)
                .valid
        );
        assert!(
            !validate_condition_exposure(BenchmarkCondition::C3, &harness, &events, &decisions,)
                .valid
        );
        events.push(lifecycle(
            project_id,
            LifecycleChannel::UserPromptSubmit,
            LifecycleStage::HookReceived,
        ));
        decisions.push(decision(project_id, LifecycleChannel::UserPromptSubmit));
        assert!(
            validate_condition_exposure(BenchmarkCondition::C3, &harness, &events, &decisions,)
                .valid
        );
        assert!(
            validate_condition_exposure(BenchmarkCondition::C4, &harness, &events, &decisions,)
                .valid,
            "MCP not requested is neutral in C4"
        );
    }

    fn lifecycle(
        project_id: ProjectId,
        channel: LifecycleChannel,
        stage: LifecycleStage,
    ) -> LifecycleEvent {
        LifecycleEvent {
            event_id: uuid::Uuid::now_v7(),
            project_id,
            harness: Harness::Codex,
            session: SessionAttribution::Attributed("session".to_owned()),
            correlation_id: Some("correlation".to_owned()),
            channel,
            stage,
            occurred_at: time::OffsetDateTime::now_utc(),
            detail: serde_json::json!({}),
        }
    }

    fn decision(project_id: ProjectId, channel: LifecycleChannel) -> RetrievalDecision {
        RetrievalDecision {
            decision_id: uuid::Uuid::now_v7(),
            project_id,
            harness: Harness::Codex,
            session: SessionAttribution::Attributed("session".to_owned()),
            correlation_id: Some("correlation".to_owned()),
            channel,
            outcome: RetrievalOutcome::HealthySilence,
            reason_code: RetrievalReasonCode::NoRelevantCandidate,
            candidate_count: 0,
            selected_count: 0,
            dropped_count: 0,
            token_count: 0,
            latency_ms: 1,
            query_sha256: "a".repeat(64),
            selected_evidence_ids: Vec::new(),
            occurred_at: time::OffsetDateTime::now_utc(),
        }
    }
}
