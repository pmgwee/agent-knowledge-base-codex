use std::cell::Cell;
use std::collections::BTreeMap;

use brain_cli::{
    CommandPlan, ProcessOutput, ProcessRunner, RunOutcome, SystemProcessRunner,
    execute_command_plan,
};

struct FakeRunner {
    calls: Cell<usize>,
}

impl ProcessRunner for FakeRunner {
    fn run(&self, _plan: &CommandPlan) -> anyhow::Result<ProcessOutput> {
        self.calls.set(self.calls.get() + 1);
        Ok(ProcessOutput {
            exit_code: Some(0),
            stdout: b"result".to_vec(),
            stderr: Vec::new(),
            timed_out: false,
            elapsed_ms: Some(25),
        })
    }
}

#[test]
fn dry_run_is_the_default_execution_boundary() {
    let temp = tempfile::tempdir().unwrap();
    let executable = temp.path().join("harness.exe");
    std::fs::write(&executable, b"fixture").unwrap();
    let plan = CommandPlan {
        sample_id: "sample-1".to_owned(),
        program: executable,
        args: vec!["--json".to_owned()],
        current_dir: temp.path().to_path_buf(),
        environment: BTreeMap::new(),
        timeout_seconds: 30,
    };
    let runner = FakeRunner {
        calls: Cell::new(0),
    };
    assert!(matches!(
        execute_command_plan(&plan, false, &runner).unwrap(),
        RunOutcome::DryRun(_)
    ));
    assert_eq!(runner.calls.get(), 0);
    assert!(matches!(
        execute_command_plan(&plan, true, &runner).unwrap(),
        RunOutcome::Executed(_)
    ));
    assert_eq!(runner.calls.get(), 1);
}

#[test]
fn generic_home_overrides_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let executable = temp.path().join("harness.exe");
    std::fs::write(&executable, b"fixture").unwrap();
    let mut environment = BTreeMap::new();
    environment.insert("CODEX_HOME".to_owned(), "unsafe".to_owned());
    let plan = CommandPlan {
        sample_id: "sample-1".to_owned(),
        program: executable,
        args: Vec::new(),
        current_dir: temp.path().to_path_buf(),
        environment,
        timeout_seconds: 30,
    };
    let runner = FakeRunner {
        calls: Cell::new(0),
    };
    assert!(execute_command_plan(&plan, true, &runner).is_err());
    assert_eq!(runner.calls.get(), 0);
}

#[test]
fn system_runner_measures_through_timeout_cleanup_with_one_monotonic_clock() {
    let temp = tempfile::tempdir().unwrap();
    let powershell = std::path::PathBuf::from(std::env::var("SystemRoot").unwrap())
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let plan = CommandPlan {
        sample_id: "timed-timeout".to_owned(),
        program: powershell,
        args: vec![
            "-NoProfile".to_owned(),
            "-Command".to_owned(),
            "Start-Sleep -Seconds 5".to_owned(),
        ],
        current_dir: temp.path().to_path_buf(),
        environment: BTreeMap::new(),
        timeout_seconds: 1,
    };
    let output = SystemProcessRunner.run(&plan).expect("timed process");
    assert!(output.timed_out);
    assert!(output.elapsed_ms.expect("monotonic elapsed time") >= 900);
}
