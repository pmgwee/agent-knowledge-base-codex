//! What the brain is wired to do — and whether each wire actually reaches anything.
//!
//! Every delivery defect this project has hit had the same shape: the configuration looked right
//! and the capability was absent. A hook registered twice, so every session start compiled two
//! orientations against a pipe that serves one client. A hook still pointing at `target/release`
//! after the binaries moved. A provider named in the config whose environment variable was never
//! set in the service's session. In each case nothing on the dashboard was wrong, because nothing
//! on the dashboard showed it.
//!
//! So this panel never reports a setting without the fact that decides whether it does anything:
//! a path with whether that file exists, an event with which harness fires it, a provider with
//! whether its variable is readable *here*.
//!
//! **It reads no secrets.** A provider's credential is named by environment variable and its
//! presence is reported as a boolean. The value is never read, never stored, and never rendered —
//! a panel that leaks a key to make a status green is worse than a panel that says "unknown".

use std::path::{Path, PathBuf};

use anyhow::Result;
use brain_service::ServiceLaunchConfig;

/// The wiring, as one document.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ConfigDashboard {
    pub harnesses: Vec<HarnessWiring>,
    pub schedule: Vec<ScheduledJob>,
    pub credentials: Vec<CredentialBinding>,
    /// The budget contract, stated so a drifting orientation is checkable against a number rather
    /// than against a memory of one.
    pub budget: BudgetContract,
}

/// One harness's push and pull paths.
#[derive(Clone, Debug, serde::Serialize)]
pub struct HarnessWiring {
    pub harness: String,
    /// Where the harness reads its hook registrations.
    pub settings_path: PathBuf,
    pub settings_present: bool,
    /// Hook events carrying a brain entry, in the order the harness fires them.
    pub events: Vec<String>,
    /// The executable those entries invoke. `None` when nothing is registered.
    pub hook_executable: Option<PathBuf>,
    /// Whether that executable is on disk. A registration pointing at a moved binary looks
    /// installed and delivers nothing.
    pub hook_executable_present: bool,
    /// More than one brain entry across the registered events. Each one fires; two orientations
    /// get compiled per session start and the pipe serves them one at a time.
    pub duplicate_registrations: usize,
    /// Whether this harness can also *pull*, via the MCP server.
    pub mcp: Option<McpWiring>,
    pub detail: String,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct McpWiring {
    pub config_path: PathBuf,
    pub server_name: String,
    pub command: PathBuf,
    pub command_present: bool,
    pub configured: bool,
    pub approved_or_connected: Option<bool>,
    pub status: String,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct ScheduledJob {
    pub task_name: String,
    pub cadence: String,
    pub does: String,
    pub installed: bool,
}

/// A credential the brain expects to find in the environment. Never its value.
#[derive(Clone, Debug, serde::Serialize)]
pub struct CredentialBinding {
    pub provider: String,
    pub variable: String,
    /// Whether the variable is readable in *this* process. The service runs in a different
    /// session, so a false here is a question rather than a verdict — which the detail says.
    pub present_in_this_session: bool,
    pub detail: String,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct BudgetContract {
    pub normal_tokens: [usize; 2],
    pub hard_max_tokens: usize,
    pub mid_session_tokens: usize,
    pub mid_session_memories: usize,
    pub max_pushes_per_session: usize,
    pub detail: String,
}

pub fn read_config_panel(
    config: &ServiceLaunchConfig,
    tasks: &[(String, bool)],
) -> Result<ConfigDashboard> {
    let home = dirs_home();
    let claude_settings = home.join(".claude").join("settings.json");
    let codex_hooks = home.join(".codex").join("hooks.json");
    let codex_config = home.join(".codex").join("config.toml");
    let claude_mcp = crate::install_mcp::inspect_claude_mcp(Path::new("claude"))
        .ok()
        .flatten()
        .map(|(command, connected, raw)| McpWiring {
            config_path: home.join(".claude.json"),
            server_name: "brain".to_owned(),
            command_present: command.is_file(),
            command,
            configured: true,
            approved_or_connected: Some(connected),
            status: if connected {
                "connected".to_owned()
            } else if raw.to_lowercase().contains("pending approval") {
                "pending_approval".to_owned()
            } else {
                "configured_not_connected".to_owned()
            },
        });

    let mut harnesses = vec![harness_wiring(
        "claude-code",
        &claude_settings,
        claude_mcp,
        "Push. The harness invokes the hook and injects the reply as developer context — nothing \
         is visible in the UI, which is why a silent failure here went unnoticed for days.",
    )?];
    harnesses.push(harness_wiring(
        "codex",
        &codex_hooks,
        codex_mcp_wiring(&codex_config)?,
        "Push and pull. Codex documents SessionStart; the MCP server carries everything the hook \
         cannot — search, timeline, evidence, claims, leases.",
    )?);

    let schedule = [
        (
            "AgentBrain.Service",
            "at logon, restarts 999×",
            "Capture, consolidation, rediscovery, and the hook pipe.",
        ),
        (
            "AgentBrain.Backup",
            "hourly",
            "GFS snapshots to a separate drive.",
        ),
        (
            "AgentBrain.RestoreDrill",
            "monthly",
            "Restores the latest backup into a scratch root — a backup nobody has restored is a \
             hypothesis.",
        ),
        (
            "AgentBrain.Digest",
            "daily",
            "The derived health reading, appended to each project's vault log. No provider call, \
             so it survives the outage that makes it most useful.",
        ),
    ]
    .into_iter()
    .map(|(name, cadence, does)| ScheduledJob {
        task_name: name.to_owned(),
        cadence: cadence.to_owned(),
        does: does.to_owned(),
        installed: tasks
            .iter()
            .any(|(task, installed)| task == name && *installed),
    })
    .collect();

    let credentials = credential_bindings(config);

    Ok(ConfigDashboard {
        harnesses,
        schedule,
        credentials,
        budget: BudgetContract {
            normal_tokens: [1_000, 1_500],
            hard_max_tokens: 3_000,
            mid_session_tokens: 400,
            mid_session_memories: 4,
            max_pushes_per_session: 20,
            detail: "A contract, not a target: adding a field to an orientation means removing \
                     one. The mid-session numbers are deliberately a fraction of the session-start \
                     ones — a push that interrupts is worse than one that never fires."
                .to_owned(),
        },
    })
}

fn harness_wiring(
    harness: &str,
    settings_path: &Path,
    mcp: Option<McpWiring>,
    detail: &str,
) -> Result<HarnessWiring> {
    let document: serde_json::Value = if settings_path.is_file() {
        serde_json::from_slice(&std::fs::read(settings_path)?).unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };
    let mut events = Vec::new();
    let mut commands: Vec<String> = Vec::new();
    for event in ["SessionStart", "SessionEnd", "UserPromptSubmit"] {
        let found: Vec<String> = document
            .pointer(&format!("/hooks/{event}"))
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|group| group.get("hooks").and_then(serde_json::Value::as_array))
            .flatten()
            .filter_map(|entry| {
                ["commandWindows", "command"]
                    .into_iter()
                    .find_map(|key| entry.get(key).and_then(serde_json::Value::as_str))
            })
            .filter(|command| command.to_lowercase().contains("brain-hook"))
            .map(str::to_owned)
            .collect();
        if !found.is_empty() {
            events.push(event.to_owned());
        }
        commands.extend(found);
    }
    // One registration per event is correct; a second entry on the *same* event is the duplicate
    // that double-fires. Counting raw commands would call a healthy three-event install a triple.
    let duplicates = commands.len().saturating_sub(events.len());
    let executable = commands.first().map(|command| executable_of(command));
    let present = executable
        .as_ref()
        .map(|path| path.is_file())
        .unwrap_or(false);
    Ok(HarnessWiring {
        harness: harness.to_owned(),
        settings_path: settings_path.to_path_buf(),
        settings_present: settings_path.is_file(),
        events,
        hook_executable: executable,
        hook_executable_present: present,
        duplicate_registrations: duplicates,
        mcp,
        detail: detail.to_owned(),
    })
}

/// Pull the first path-looking token out of a hook command line.
fn executable_of(command: &str) -> PathBuf {
    let trimmed = command.trim();
    if let Some(rest) = trimmed.strip_prefix('"')
        && let Some(end) = rest.find('"')
    {
        return PathBuf::from(&rest[..end]);
    }
    PathBuf::from(
        trimmed
            .split_whitespace()
            .find(|token| token.to_lowercase().contains("brain-hook"))
            .unwrap_or(trimmed),
    )
}

fn codex_mcp_wiring(config_path: &Path) -> Result<Option<McpWiring>> {
    if !config_path.is_file() {
        return Ok(None);
    }
    // Deliberately a line scan rather than a TOML dependency: this reads one key out of a file the
    // brain does not own, and a parse failure on an unrelated section must not blank the panel.
    let text = std::fs::read_to_string(config_path)?;
    let mut in_brain = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_brain = line.starts_with("[mcp_servers.brain]");
            continue;
        }
        if in_brain && let Some(value) = line.strip_prefix("command") {
            // Both quote styles. TOML literal strings use single quotes and do not escape
            // backslashes — which is exactly how a Windows path is usually written here — so
            // trimming only `"` leaves the quotes in the path and reports a present binary as
            // missing. That is the panel's own failure mode: a status that is wrong in the
            // reassuring direction is a bug; wrong in the alarming direction is noise nobody acts
            // on twice.
            let raw = value.trim_start_matches([' ', '=']).trim();
            let command = match raw.strip_prefix('\'') {
                Some(rest) => rest.trim_end_matches('\'').to_owned(),
                None => raw.trim_matches('"').replace("\\\\", "\\"),
            };
            let path = PathBuf::from(&command);
            return Ok(Some(McpWiring {
                config_path: config_path.to_path_buf(),
                server_name: "brain".to_owned(),
                command_present: path.is_file(),
                command: path,
                configured: true,
                approved_or_connected: None,
                status: "configured_connection_not_observable".to_owned(),
            }));
        }
    }
    Ok(None)
}

fn credential_bindings(config: &ServiceLaunchConfig) -> Vec<CredentialBinding> {
    let Some(brain_service::ConsolidationProviderConfig::Glm {
        model, api_key_env, ..
    }) = &config.consolidation
    else {
        return Vec::new();
    };
    // `is_ok`, never the value. The panel's job is to say whether a name resolves, not to move a
    // secret one step closer to a rendered page.
    let present = std::env::var(api_key_env).is_ok();
    vec![CredentialBinding {
        provider: format!("glm · {model}"),
        variable: api_key_env.clone(),
        present_in_this_session: present,
        detail: if present {
            "Readable here. The service runs in its own session and may still differ — a deferral \
             logged as HTTP 401 is the signal that it does, and a 429 means quota rather than \
             configuration."
                .to_owned()
        } else {
            "Not readable in this process. The service runs under Task Scheduler with its own \
             environment, so this is a question, not a verdict: check the deferral reason before \
             treating it as unset."
                .to_owned()
        },
    }]
}

fn dirs_home() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_duplicate_registration_is_counted_per_event_not_per_command() {
        // The defect this exists to surface, and the arithmetic that makes it readable: three
        // events with one entry each is a healthy install, not a triple registration.
        let directory = tempfile::tempdir().expect("tempdir");
        let settings = directory.path().join("settings.json");
        std::fs::write(
            &settings,
            serde_json::to_vec(&serde_json::json!({
                "hooks": {
                    "SessionStart": [{"hooks": [{"command": "C:\\bin\\brain-hook.exe --harness claude-code"}]}],
                    "SessionEnd": [{"hooks": [{"command": "C:\\bin\\brain-hook.exe --harness claude-code"}]}],
                    "UserPromptSubmit": [{"hooks": [{"command": "C:\\bin\\brain-hook.exe --harness claude-code"}]}]
                }
            }))
            .expect("json"),
        )
        .expect("write");
        let wiring = harness_wiring("claude-code", &settings, None, "").expect("wiring");
        assert_eq!(wiring.events.len(), 3);
        assert_eq!(wiring.duplicate_registrations, 0);
    }

    #[test]
    fn two_entries_on_one_event_are_reported_as_a_duplicate() {
        let directory = tempfile::tempdir().expect("tempdir");
        let settings = directory.path().join("settings.json");
        std::fs::write(
            &settings,
            serde_json::to_vec(&serde_json::json!({
                "hooks": {
                    "SessionStart": [
                        {"hooks": [{"command": "C:\\old\\brain-hook.exe"}]},
                        {"hooks": [{"command": "C:\\bin\\brain-hook.exe"}]}
                    ]
                }
            }))
            .expect("json"),
        )
        .expect("write");
        let wiring = harness_wiring("claude-code", &settings, None, "").expect("wiring");
        assert_eq!(wiring.duplicate_registrations, 1);
    }

    #[test]
    fn another_tools_hook_is_not_counted_as_ours() {
        // CodeGraph sits on UserPromptSubmit in this very config. A panel that claimed its entry
        // was a brain duplicate would send someone to delete a working hook.
        let directory = tempfile::tempdir().expect("tempdir");
        let settings = directory.path().join("settings.json");
        std::fs::write(
            &settings,
            serde_json::to_vec(&serde_json::json!({
                "hooks": {
                    "UserPromptSubmit": [
                        {"hooks": [{"command": "codegraph-hook.exe"}]},
                        {"hooks": [{"command": "C:\\bin\\brain-hook.exe"}]}
                    ]
                }
            }))
            .expect("json"),
        )
        .expect("write");
        let wiring = harness_wiring("claude-code", &settings, None, "").expect("wiring");
        assert_eq!(wiring.events, vec!["UserPromptSubmit".to_owned()]);
        assert_eq!(wiring.duplicate_registrations, 0);
    }

    #[test]
    fn a_missing_settings_file_reports_absence_rather_than_failing() {
        // Optional wiring may only add. A harness that was never installed is a fact to render,
        // not an error that blanks the whole panel.
        let wiring =
            harness_wiring("codex", Path::new("nowhere/settings.json"), None, "").expect("wiring");
        assert!(!wiring.settings_present);
        assert!(wiring.events.is_empty());
        assert!(wiring.hook_executable.is_none());
        assert!(!wiring.hook_executable_present);
    }

    #[test]
    fn the_mcp_command_is_read_without_a_toml_parser() {
        let directory = tempfile::tempdir().expect("tempdir");
        let config = directory.path().join("config.toml");
        std::fs::write(
            &config,
            "[mcp_servers.other]\ncommand = \"other.exe\"\n\n\
             [mcp_servers.brain]\ncommand = \"C:\\\\bin\\\\brain-mcp.exe\"\nargs = []\n",
        )
        .expect("write");
        let wiring = codex_mcp_wiring(&config).expect("wiring").expect("present");
        assert_eq!(wiring.server_name, "brain");
        assert_eq!(wiring.command, PathBuf::from(r"C:\bin\brain-mcp.exe"));
    }

    #[test]
    fn a_toml_literal_string_keeps_its_backslashes_and_loses_its_quotes() {
        // The live config is written this way, and the first version of this parser reported the
        // installed MCP binary as missing because the quotes stayed in the path.
        let directory = tempfile::tempdir().expect("tempdir");
        let config = directory.path().join("config.toml");
        std::fs::write(
            &config,
            "[mcp_servers.brain]\ncommand = 'C:\\Users\\me\\AgentBrain\\bin\\brain-mcp.exe'\n",
        )
        .expect("write");
        let wiring = codex_mcp_wiring(&config).expect("wiring").expect("present");
        assert_eq!(
            wiring.command,
            PathBuf::from(r"C:\Users\me\AgentBrain\bin\brain-mcp.exe")
        );
    }
}
