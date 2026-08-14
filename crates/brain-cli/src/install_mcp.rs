use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail, ensure};

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct McpInstallReport {
    pub harness: String,
    pub scope: String,
    pub server_name: String,
    pub command: PathBuf,
    pub binary_present: bool,
    pub configured: bool,
    pub connected: bool,
    pub changed: bool,
    pub detail: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct McpCommandOutput {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

pub trait McpCommandRunner {
    fn run(&self, program: &Path, args: &[String]) -> Result<McpCommandOutput>;
}

pub struct SystemMcpCommandRunner;

impl McpCommandRunner for SystemMcpCommandRunner {
    fn run(&self, program: &Path, args: &[String]) -> Result<McpCommandOutput> {
        let output = Command::new(program)
            .args(args)
            .output()
            .with_context(|| format!("run {} {}", program.display(), args.join(" ")))?;
        Ok(McpCommandOutput {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

pub fn install_claude_mcp(brain_home: &Path, claude_executable: &Path) -> Result<McpInstallReport> {
    install_claude_mcp_with(brain_home, claude_executable, &SystemMcpCommandRunner)
}

pub fn uninstall_claude_mcp(
    brain_home: &Path,
    claude_executable: &Path,
) -> Result<McpInstallReport> {
    uninstall_claude_mcp_with(brain_home, claude_executable, &SystemMcpCommandRunner)
}

pub fn install_claude_mcp_with(
    brain_home: &Path,
    claude_executable: &Path,
    runner: &impl McpCommandRunner,
) -> Result<McpInstallReport> {
    let binary = brain_home.join("bin").join("brain-mcp.exe");
    validate_installed_mcp_path(&binary)?;
    let binary = binary.canonicalize()?;
    validate_deployment_if_configured(brain_home)?;
    let existing = get(runner, claude_executable)?;
    let mut changed = false;
    if existing.success {
        let command = parse_command(&existing.stdout).context("Claude returned no MCP command")?;
        ensure_same_command(&command, &binary)?;
    } else {
        let arguments = vec![
            "mcp".to_owned(),
            "add".to_owned(),
            "--scope".to_owned(),
            "user".to_owned(),
            "--transport".to_owned(),
            "stdio".to_owned(),
            "brain".to_owned(),
            "--".to_owned(),
            binary.to_string_lossy().into_owned(),
        ];
        let added = runner.run(claude_executable, &arguments)?;
        ensure!(
            added.success,
            "Claude MCP add failed: {}",
            nonempty(&added.stderr, &added.stdout)
        );
        changed = true;
    }
    let verified = get(runner, claude_executable)?;
    ensure!(
        verified.success,
        "Claude MCP server was not visible after installation: {}",
        nonempty(&verified.stderr, &verified.stdout)
    );
    let command = parse_command(&verified.stdout).context("Claude returned no MCP command")?;
    ensure_same_command(&command, &binary)?;
    let connected = verified.stdout.to_lowercase().contains("connected");
    Ok(McpInstallReport {
        harness: "claude-code".to_owned(),
        scope: "user".to_owned(),
        server_name: "brain".to_owned(),
        command: binary,
        binary_present: true,
        configured: true,
        connected,
        changed,
        detail: if connected {
            "Claude reports the user-scoped Brain MCP server connected.".to_owned()
        } else {
            "Claude reports the server configured but not connected; inspect `claude mcp get brain`."
                .to_owned()
        },
    })
}

pub fn uninstall_claude_mcp_with(
    brain_home: &Path,
    claude_executable: &Path,
    runner: &impl McpCommandRunner,
) -> Result<McpInstallReport> {
    let binary = brain_home.join("bin").join("brain-mcp.exe");
    validate_installed_mcp_path(&binary)?;
    let binary = binary.canonicalize()?;
    let existing = get(runner, claude_executable)?;
    if !existing.success {
        return Ok(McpInstallReport {
            harness: "claude-code".to_owned(),
            scope: "user".to_owned(),
            server_name: "brain".to_owned(),
            command: binary,
            binary_present: true,
            configured: false,
            connected: false,
            changed: false,
            detail: "Claude has no MCP server named brain.".to_owned(),
        });
    }
    let command = parse_command(&existing.stdout).context("Claude returned no MCP command")?;
    ensure_same_command(&command, &binary)?;
    let removed = runner.run(
        claude_executable,
        &[
            "mcp".to_owned(),
            "remove".to_owned(),
            "brain".to_owned(),
            "--scope".to_owned(),
            "user".to_owned(),
        ],
    )?;
    ensure!(
        removed.success,
        "Claude MCP remove failed: {}",
        nonempty(&removed.stderr, &removed.stdout)
    );
    Ok(McpInstallReport {
        harness: "claude-code".to_owned(),
        scope: "user".to_owned(),
        server_name: "brain".to_owned(),
        command: binary,
        binary_present: true,
        configured: false,
        connected: false,
        changed: true,
        detail: "Removed only the user-scoped Claude MCP server named brain.".to_owned(),
    })
}

pub fn validate_installed_mcp_path(path: &Path) -> Result<()> {
    ensure!(
        path.is_file(),
        "installed brain-mcp binary is missing: {}",
        path.display()
    );
    let normalized = path.to_string_lossy().replace('\\', "/").to_lowercase();
    ensure!(
        !normalized.contains("/target/release/")
            && !normalized.ends_with("/target/release/brain-mcp.exe"),
        "target/release is a build artifact, not an installed MCP binary"
    );
    ensure!(
        path.file_name().and_then(|name| name.to_str()) == Some("brain-mcp.exe"),
        "MCP command is not brain-mcp.exe"
    );
    Ok(())
}

pub(crate) fn inspect_claude_mcp(
    claude_executable: &Path,
) -> Result<Option<(PathBuf, bool, String)>> {
    let output = get(&SystemMcpCommandRunner, claude_executable)?;
    if !output.success {
        return Ok(None);
    }
    let Some(command) = parse_command(&output.stdout) else {
        return Ok(None);
    };
    let connected = output.stdout.to_lowercase().contains("connected");
    Ok(Some((command, connected, output.stdout)))
}

fn get(runner: &impl McpCommandRunner, claude_executable: &Path) -> Result<McpCommandOutput> {
    runner.run(
        claude_executable,
        &["mcp".to_owned(), "get".to_owned(), "brain".to_owned()],
    )
}

fn parse_command(output: &str) -> Option<PathBuf> {
    output.lines().find_map(|line| {
        line.trim()
            .strip_prefix("Command:")
            .map(str::trim)
            .filter(|command| !command.is_empty())
            .map(|command| PathBuf::from(command.trim_matches('"')))
    })
}

fn ensure_same_command(configured: &Path, expected: &Path) -> Result<()> {
    let configured = configured
        .canonicalize()
        .unwrap_or_else(|_| configured.to_path_buf());
    let expected = expected
        .canonicalize()
        .unwrap_or_else(|_| expected.to_path_buf());
    if configured != expected {
        bail!(
            "refusing to replace or remove Claude MCP server `brain`: command {} does not match Agent Brain {}",
            configured.display(),
            expected.display()
        );
    }
    Ok(())
}

fn validate_deployment_if_configured(brain_home: &Path) -> Result<()> {
    let deployment = crate::read_deployment(brain_home);
    if deployment.configured {
        ensure!(
            deployment.up_to_date
                && !deployment
                    .drifted_binaries
                    .iter()
                    .any(|binary| binary == "brain-mcp.exe"),
            "installed brain-mcp is stale or drifted; deploy and verify before wiring Claude"
        );
    }
    Ok(())
}

fn nonempty<'a>(preferred: &'a str, fallback: &'a str) -> &'a str {
    if preferred.trim().is_empty() {
        fallback.trim()
    } else {
        preferred.trim()
    }
}
