use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct CommandPlan {
    pub sample_id: String,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub current_dir: PathBuf,
    pub environment: BTreeMap<String, String>,
    pub timeout_seconds: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessOutput {
    pub exit_code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub timed_out: bool,
    pub elapsed_ms: Option<u64>,
}

pub trait ProcessRunner {
    fn run(&self, plan: &CommandPlan) -> Result<ProcessOutput>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemProcessRunner;

impl ProcessRunner for SystemProcessRunner {
    fn run(&self, plan: &CommandPlan) -> Result<ProcessOutput> {
        ensure!(plan.timeout_seconds > 0, "process timeout must be positive");
        let started = Instant::now();
        let mut command = Command::new(&plan.program);
        command
            .args(&plan.args)
            .current_dir(&plan.current_dir)
            .envs(&plan.environment)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .with_context(|| format!("launch {}", plan.program.display()))?;
        let deadline = started + Duration::from_secs(plan.timeout_seconds);
        let mut timed_out = false;
        loop {
            if child.try_wait()?.is_some() {
                break;
            }
            if Instant::now() >= deadline {
                child.kill()?;
                timed_out = true;
                break;
            }
            thread::sleep(Duration::from_millis(25));
        }
        let output = child.wait_with_output()?;
        let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        Ok(ProcessOutput {
            exit_code: output.status.code(),
            stdout: output.stdout,
            stderr: output.stderr,
            timed_out,
            elapsed_ms: Some(elapsed_ms),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RunOutcome {
    DryRun(CommandPlan),
    Executed(ProcessOutput),
}

pub fn execute_command_plan(
    plan: &CommandPlan,
    execute: bool,
    runner: &impl ProcessRunner,
) -> Result<RunOutcome> {
    validate_plan(plan)?;
    if !execute {
        return Ok(RunOutcome::DryRun(plan.clone()));
    }
    Ok(RunOutcome::Executed(runner.run(plan)?))
}

fn validate_plan(plan: &CommandPlan) -> Result<()> {
    ensure!(!plan.sample_id.trim().is_empty(), "sample id is required");
    ensure!(plan.program.is_file(), "harness executable does not exist");
    ensure!(
        plan.current_dir.is_dir(),
        "benchmark checkout does not exist"
    );
    ensure!(plan.timeout_seconds > 0, "timeout must be positive");
    if plan.environment.contains_key("HOME") || plan.environment.contains_key("CODEX_HOME") {
        bail!("generic HOME/CODEX_HOME overrides are forbidden; use a benchmark-scoped launcher");
    }
    Ok(())
}
