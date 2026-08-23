use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use brain_cli::{
    McpCommandOutput, McpCommandRunner, install_claude_mcp_with, uninstall_claude_mcp_with,
};

struct FakeRunner {
    outputs: Mutex<VecDeque<McpCommandOutput>>,
    calls: Mutex<Vec<Vec<String>>>,
}

impl FakeRunner {
    fn new(outputs: Vec<McpCommandOutput>) -> Self {
        Self {
            outputs: Mutex::new(outputs.into()),
            calls: Mutex::new(Vec::new()),
        }
    }
}

impl McpCommandRunner for FakeRunner {
    fn run(&self, _program: &Path, args: &[String]) -> anyhow::Result<McpCommandOutput> {
        self.calls.lock().unwrap().push(args.to_vec());
        Ok(self.outputs.lock().unwrap().pop_front().unwrap())
    }
}

fn output(success: bool, stdout: impl Into<String>) -> McpCommandOutput {
    McpCommandOutput {
        success,
        stdout: stdout.into(),
        stderr: String::new(),
    }
}

#[test]
fn install_uses_claude_supported_user_scope_and_is_idempotently_verified() {
    let temp = tempfile::tempdir().unwrap();
    let binary = temp.path().join("bin/brain-mcp.exe");
    std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
    std::fs::write(&binary, b"mcp").unwrap();
    let connected = format!(
        "brain:\n  Scope: User config (available in all your projects)\n  Status: √ Connected\n  Type: stdio\n  Command: {}\n  Args:\n",
        binary.canonicalize().unwrap().display()
    );
    let runner = FakeRunner::new(vec![
        output(false, "No MCP server named brain"),
        output(true, "added"),
        output(true, connected),
    ]);
    let report = install_claude_mcp_with(temp.path(), Path::new("claude.exe"), &runner).unwrap();
    assert!(report.configured);
    assert!(report.connected);
    assert!(report.changed);
    let calls = runner.calls.lock().unwrap();
    assert_eq!(
        calls[1][..7],
        [
            "mcp",
            "add",
            "--scope",
            "user",
            "--transport",
            "stdio",
            "brain"
        ]
    );
    assert_eq!(calls[1][7], "--");
    assert_eq!(PathBuf::from(&calls[1][8]), binary.canonicalize().unwrap());
}

#[test]
fn target_release_and_foreign_brain_server_are_never_overwritten_or_removed() {
    let temp = tempfile::tempdir().unwrap();
    let stale = temp.path().join("target/release/brain-mcp.exe");
    std::fs::create_dir_all(stale.parent().unwrap()).unwrap();
    std::fs::write(&stale, b"stale").unwrap();
    assert!(brain_cli::validate_installed_mcp_path(&stale).is_err());

    let binary = temp.path().join("bin/brain-mcp.exe");
    std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
    std::fs::write(&binary, b"mcp").unwrap();
    let runner = FakeRunner::new(vec![output(
        true,
        "brain:\n  Scope: User config\n  Status: √ Connected\n  Command: C:\\other\\brain-mcp.exe\n",
    )]);
    assert!(uninstall_claude_mcp_with(temp.path(), Path::new("claude.exe"), &runner).is_err());
    assert_eq!(runner.calls.lock().unwrap().len(), 1);
}
