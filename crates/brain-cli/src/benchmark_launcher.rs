use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{BenchmarkCondition, BenchmarkHarness};

const BENCHMARK_ENV_PREFIX: &str = "BENCHMARK_";
const BOUNDED_PROMPT: &str = "Benchmark contract: work only inside the provided checkout. Do not read user-level configuration or any path outside the checkout and sample workspace. Use at most {max_turns} assistant turns and {max_tool_calls} tool calls. Solve the task, run the smallest relevant verification, and report concise evidence.\n\nTask:\n{prompt}";

#[derive(Clone, Debug, Parser)]
#[command(
    version,
    about = "Isolated native-harness launcher for the second-brain benchmark"
)]
pub struct BenchmarkLauncherArgs {
    #[arg(long)]
    pub condition_config: PathBuf,
    #[arg(long)]
    pub checkout: PathBuf,
    #[arg(long)]
    pub sample_root: PathBuf,
    #[arg(long)]
    pub harness_home: PathBuf,
    #[arg(long)]
    pub codegraph_home: PathBuf,
    #[arg(long)]
    pub max_turns: u32,
    #[arg(long)]
    pub max_tool_calls: u32,
    #[arg(long)]
    pub prompt: String,
    /// Materialize and validate the isolated configuration without launching a paid agent.
    #[arg(long)]
    pub prepare_only: bool,
}

#[derive(Clone, Debug, Deserialize)]
struct ConditionProfile {
    schema_version: u32,
    harness: BenchmarkHarness,
    condition: BenchmarkCondition,
    tools: Value,
    hooks: Value,
    mcp: Value,
}

#[derive(Clone, Debug)]
struct LauncherPaths {
    native_harness: PathBuf,
    codegraph_node: Option<PathBuf>,
    codegraph_script: Option<PathBuf>,
    brain_hook: Option<PathBuf>,
    brain_mcp: Option<PathBuf>,
    credential: Option<PathBuf>,
    claude_state: Option<PathBuf>,
    brain_home: Option<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LauncherPreparation {
    pub schema_version: u32,
    pub harness: BenchmarkHarness,
    pub condition: BenchmarkCondition,
    pub harness_home: PathBuf,
    pub settings_path: PathBuf,
    pub mcp_config_path: Option<PathBuf>,
    pub codegraph_enabled: bool,
    pub session_start_enabled: bool,
    pub session_end_enabled: bool,
    pub prompt_push_enabled: bool,
    pub brain_mcp_enabled: bool,
    pub credential_copied: bool,
}

pub fn run_benchmark_launcher() -> Result<()> {
    let args = BenchmarkLauncherArgs::parse();
    let profile: ConditionProfile = serde_json::from_slice(
        &fs::read(&args.condition_config)
            .with_context(|| format!("read {}", args.condition_config.display()))?,
    )?;
    validate_args(&args, &profile)?;
    let paths = LauncherPaths::from_environment(profile.harness, args.prepare_only)?;
    let preparation = prepare(&args, &profile, &paths, !args.prepare_only)?;
    if args.prepare_only {
        println!("{}", serde_json::to_string_pretty(&preparation)?);
        return Ok(());
    }

    if preparation.codegraph_enabled {
        initialize_codegraph(&args.checkout, &args.codegraph_home, &paths)?;
    }
    let _credential = CredentialGuard::new(profile.harness, &args.harness_home);
    let status = launch_native(&args, &profile, &paths, &preparation)?;
    ensure!(status.success(), "native harness exited with {status}");
    Ok(())
}

impl LauncherPaths {
    fn from_environment(harness: BenchmarkHarness, prepare_only: bool) -> Result<Self> {
        let harness_key = match harness {
            BenchmarkHarness::ClaudeCode => "BENCHMARK_CLAUDE_EXECUTABLE",
            BenchmarkHarness::Codex => "BENCHMARK_CODEX_EXECUTABLE",
        };
        let credential_key = match harness {
            BenchmarkHarness::ClaudeCode => "BENCHMARK_CLAUDE_CREDENTIALS",
            BenchmarkHarness::Codex => "BENCHMARK_CODEX_AUTH",
        };
        let required = |key: &str| -> Result<PathBuf> {
            let path = std::env::var_os(key)
                .map(PathBuf::from)
                .with_context(|| format!("{key} is required"))?;
            ensure!(
                path.is_file(),
                "{key} does not name a file: {}",
                path.display()
            );
            path.canonicalize()
                .with_context(|| format!("resolve {key}"))
        };
        let optional = |key: &str| -> Result<Option<PathBuf>> {
            std::env::var_os(key)
                .map(PathBuf::from)
                .map(|path| {
                    ensure!(
                        path.is_file(),
                        "{key} does not name a file: {}",
                        path.display()
                    );
                    path.canonicalize()
                        .with_context(|| format!("resolve {key}"))
                })
                .transpose()
        };
        Ok(Self {
            native_harness: required(harness_key)?,
            codegraph_node: optional("BENCHMARK_CODEGRAPH_NODE")?,
            codegraph_script: optional("BENCHMARK_CODEGRAPH_SCRIPT")?,
            brain_hook: optional("BENCHMARK_BRAIN_HOOK")?,
            brain_mcp: optional("BENCHMARK_BRAIN_MCP")?,
            credential: if prepare_only {
                optional(credential_key)?
            } else {
                Some(required(credential_key)?)
            },
            claude_state: if harness == BenchmarkHarness::ClaudeCode {
                if prepare_only {
                    optional("BENCHMARK_CLAUDE_STATE")?
                } else {
                    Some(required("BENCHMARK_CLAUDE_STATE")?)
                }
            } else {
                None
            },
            brain_home: std::env::var_os("BRAIN_HOME").map(PathBuf::from),
        })
    }
}

fn validate_args(args: &BenchmarkLauncherArgs, profile: &ConditionProfile) -> Result<()> {
    ensure!(
        profile.schema_version == 2,
        "unsupported condition-profile schema"
    );
    ensure!(args.checkout.is_dir(), "benchmark checkout is missing");
    ensure!(args.sample_root.is_dir(), "sample root is missing");
    ensure!(args.max_turns > 0, "max-turns must be positive");
    ensure!(args.max_tool_calls > 0, "max-tool-calls must be positive");
    ensure!(!args.prompt.trim().is_empty(), "prompt is required");
    ensure!(
        args.harness_home.starts_with(&args.sample_root),
        "harness home must be inside sample root"
    );
    ensure!(
        args.codegraph_home.starts_with(&args.sample_root),
        "CodeGraph home must be inside sample root"
    );
    validate_profile_capabilities(profile)
}

fn validate_profile_capabilities(profile: &ConditionProfile) -> Result<()> {
    let codegraph = profile
        .tools
        .pointer("/codegraph/enabled")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let events = profile
        .hooks
        .pointer("/agent_brain/events")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let event_names = events.iter().filter_map(Value::as_str).collect::<Vec<_>>();
    let brain_mcp = profile.mcp.pointer("/agent_brain").is_some();
    let expected_codegraph = profile.condition != BenchmarkCondition::C0;
    let expected_hooks: &[&str] = match profile.condition {
        BenchmarkCondition::C0 | BenchmarkCondition::C1 => &[],
        BenchmarkCondition::C2 => &["SessionStart", "SessionEnd"],
        BenchmarkCondition::C3 | BenchmarkCondition::C4 => {
            &["SessionStart", "SessionEnd", "UserPromptSubmit"]
        }
    };
    ensure!(
        codegraph == expected_codegraph,
        "profile CodeGraph capability is inconsistent"
    );
    ensure!(
        event_names == expected_hooks,
        "profile hook capability is inconsistent"
    );
    ensure!(
        brain_mcp == (profile.condition == BenchmarkCondition::C4),
        "profile Brain MCP capability is inconsistent"
    );
    Ok(())
}

fn prepare(
    args: &BenchmarkLauncherArgs,
    profile: &ConditionProfile,
    paths: &LauncherPaths,
    copy_credential: bool,
) -> Result<LauncherPreparation> {
    fs::create_dir_all(&args.harness_home)?;
    fs::create_dir_all(&args.codegraph_home)?;
    let codegraph_enabled = profile.condition != BenchmarkCondition::C0;
    if codegraph_enabled {
        require_path(&paths.codegraph_node, "BENCHMARK_CODEGRAPH_NODE")?;
        require_path(&paths.codegraph_script, "BENCHMARK_CODEGRAPH_SCRIPT")?;
    }
    let session_start_enabled = profile.condition.requires_brain_service();
    let session_end_enabled = session_start_enabled;
    let prompt_push_enabled = matches!(
        profile.condition,
        BenchmarkCondition::C3 | BenchmarkCondition::C4
    );
    let brain_mcp_enabled = profile.condition == BenchmarkCondition::C4;
    let brain_hook = if session_start_enabled {
        Some(copy_brain_binary(
            args,
            paths.brain_home.as_deref(),
            paths.brain_hook.as_deref(),
            "brain-hook.exe",
        )?)
    } else {
        None
    };
    let brain_mcp = if brain_mcp_enabled {
        Some(copy_brain_binary(
            args,
            paths.brain_home.as_deref(),
            paths.brain_mcp.as_deref(),
            "brain-mcp.exe",
        )?)
    } else {
        None
    };
    let (settings_path, mcp_config_path) = match profile.harness {
        BenchmarkHarness::ClaudeCode => prepare_claude(
            args,
            paths,
            brain_hook.as_deref(),
            brain_mcp.as_deref(),
            prompt_push_enabled,
            codegraph_enabled,
        )?,
        BenchmarkHarness::Codex => prepare_codex(
            args,
            paths,
            brain_hook.as_deref(),
            brain_mcp.as_deref(),
            prompt_push_enabled,
            codegraph_enabled,
        )?,
    };
    let credential_copied = if copy_credential {
        let source = paths
            .credential
            .as_deref()
            .context("credential source is required")?;
        let target = credential_path(profile.harness, &args.harness_home);
        fs::copy(source, &target)
            .with_context(|| format!("copy credential into {}", target.display()))?;
        if profile.harness == BenchmarkHarness::ClaudeCode {
            copy_claude_auth_state(
                paths
                    .claude_state
                    .as_deref()
                    .context("BENCHMARK_CLAUDE_STATE is required")?,
                &args.harness_home.join(".claude.json"),
            )?;
        }
        true
    } else {
        false
    };
    Ok(LauncherPreparation {
        schema_version: 1,
        harness: profile.harness,
        condition: profile.condition,
        harness_home: args.harness_home.clone(),
        settings_path,
        mcp_config_path,
        codegraph_enabled,
        session_start_enabled,
        session_end_enabled,
        prompt_push_enabled,
        brain_mcp_enabled,
        credential_copied,
    })
}

fn copy_brain_binary(
    args: &BenchmarkLauncherArgs,
    brain_home: Option<&Path>,
    source: Option<&Path>,
    name: &str,
) -> Result<PathBuf> {
    let source = source.with_context(|| format!("source for {name} is required"))?;
    let brain_home = brain_home.context("BRAIN_HOME is required for this condition")?;
    ensure!(
        brain_home.starts_with(&args.sample_root),
        "BRAIN_HOME must be inside sample root"
    );
    let target = brain_home.join("bin").join(name);
    fs::create_dir_all(target.parent().expect("binary parent"))?;
    fs::copy(source, &target).with_context(|| format!("copy {name}"))?;
    Ok(target)
}

fn prepare_claude(
    args: &BenchmarkLauncherArgs,
    paths: &LauncherPaths,
    brain_hook: Option<&Path>,
    brain_mcp: Option<&Path>,
    prompt_push: bool,
    codegraph: bool,
) -> Result<(PathBuf, Option<PathBuf>)> {
    let settings_path = args.harness_home.join("benchmark-settings.json");
    let hooks = hook_document(BenchmarkHarness::ClaudeCode, brain_hook, prompt_push)?;
    fs::write(
        &settings_path,
        serde_json::to_vec_pretty(&json!({ "hooks": hooks }))?,
    )?;
    let mcp_path = args.harness_home.join("benchmark-mcp.json");
    let mut servers = serde_json::Map::new();
    if codegraph {
        servers.insert("codegraph".to_owned(), codegraph_mcp(paths)?);
    }
    if let Some(brain_mcp) = brain_mcp {
        servers.insert(
            "brain".to_owned(),
            json!({ "command": brain_mcp, "args": [] }),
        );
    }
    fs::write(
        &mcp_path,
        serde_json::to_vec_pretty(&json!({ "mcpServers": servers }))?,
    )?;
    Ok((settings_path, Some(mcp_path)))
}

fn prepare_codex(
    args: &BenchmarkLauncherArgs,
    paths: &LauncherPaths,
    brain_hook: Option<&Path>,
    brain_mcp: Option<&Path>,
    prompt_push: bool,
    codegraph: bool,
) -> Result<(PathBuf, Option<PathBuf>)> {
    let hooks_path = args.harness_home.join("hooks.json");
    fs::write(
        &hooks_path,
        serde_json::to_vec_pretty(&json!({
            "hooks": hook_document(BenchmarkHarness::Codex, brain_hook, prompt_push)?
        }))?,
    )?;
    let config_path = args.harness_home.join("config.toml");
    let mut config = String::new();
    if codegraph {
        push_toml_mcp(
            &mut config,
            "codegraph",
            require_path(&paths.codegraph_node, "node")?,
            &[
                path_text(require_path(&paths.codegraph_script, "CodeGraph script")?),
                "serve".to_owned(),
                "--mcp".to_owned(),
            ],
        )?;
    }
    if let Some(brain_mcp) = brain_mcp {
        push_toml_mcp(&mut config, "brain", brain_mcp, &[])?;
    }
    fs::write(&config_path, config)?;
    Ok((config_path, None))
}

fn hook_document(
    harness: BenchmarkHarness,
    hook: Option<&Path>,
    prompt_push: bool,
) -> Result<Value> {
    let Some(hook) = hook else {
        return Ok(json!({}));
    };
    let (harness_arg, start_matcher, start_timeout, end_timeout) = match harness {
        BenchmarkHarness::ClaudeCode => {
            ("claude-code", "startup|resume|clear|compact|fork", 10, 10)
        }
        BenchmarkHarness::Codex => ("codex", "^(startup|resume|clear|compact)$", 15, 3),
    };
    if harness == BenchmarkHarness::Codex {
        ensure!(
            !hook.to_string_lossy().contains(' '),
            "Codex benchmark hook path contains a space"
        );
    }
    let command = format!("\"{}\" --harness {harness_arg}", hook.display());
    let command_windows = format!("{} --harness {harness_arg}", hook.display());
    let entry = |timeout: u64, status: bool| {
        let mut value = json!({
            "type": "command",
            "timeout": timeout
        });
        let object = value.as_object_mut().expect("object");
        match harness {
            BenchmarkHarness::ClaudeCode => {
                object.insert("command".to_owned(), json!(hook));
                object.insert("args".to_owned(), json!(["--harness", harness_arg]));
            }
            BenchmarkHarness::Codex => {
                object.insert("command".to_owned(), json!(command));
                object.insert("commandWindows".to_owned(), json!(command_windows));
                if status {
                    object.insert("statusMessage".to_owned(), json!("Loading project memory"));
                    object.insert("additionalContextLimit".to_owned(), json!(1500));
                }
            }
        }
        value
    };
    let mut hooks = serde_json::Map::from_iter([
        (
            "SessionStart".to_owned(),
            json!([{ "matcher": start_matcher, "hooks": [entry(start_timeout, true)] }]),
        ),
        (
            "SessionEnd".to_owned(),
            json!([{ "matcher": ".*", "hooks": [entry(end_timeout, false)] }]),
        ),
    ]);
    if prompt_push {
        let group = match harness {
            BenchmarkHarness::ClaudeCode => json!([{ "hooks": [entry(start_timeout, false)] }]),
            BenchmarkHarness::Codex => {
                json!([{ "matcher": ".*", "hooks": [entry(start_timeout, false)] }])
            }
        };
        hooks.insert("UserPromptSubmit".to_owned(), group);
    }
    Ok(Value::Object(hooks))
}

fn codegraph_mcp(paths: &LauncherPaths) -> Result<Value> {
    Ok(json!({
        "command": require_path(&paths.codegraph_node, "BENCHMARK_CODEGRAPH_NODE")?,
        "args": [
            require_path(&paths.codegraph_script, "BENCHMARK_CODEGRAPH_SCRIPT")?,
            "serve",
            "--mcp"
        ]
    }))
}

fn push_toml_mcp(output: &mut String, name: &str, command: &Path, args: &[String]) -> Result<()> {
    output.push_str(&format!(
        "[mcp_servers.{name}]\ncommand = {}\nargs = [",
        toml_string(&path_text(command))?
    ));
    for (index, argument) in args.iter().enumerate() {
        if index > 0 {
            output.push_str(", ");
        }
        output.push_str(&toml_string(argument)?);
    }
    output.push_str("]\n\n");
    Ok(())
}

fn toml_string(value: &str) -> Result<String> {
    serde_json::to_string(value).context("encode TOML-compatible basic string")
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

fn require_path<'a>(path: &'a Option<PathBuf>, label: &str) -> Result<&'a Path> {
    path.as_deref()
        .with_context(|| format!("{label} is required"))
}

fn initialize_codegraph(
    checkout: &Path,
    codegraph_home: &Path,
    paths: &LauncherPaths,
) -> Result<()> {
    let status = Command::new(require_path(&paths.codegraph_node, "CodeGraph node")?)
        .arg(require_path(&paths.codegraph_script, "CodeGraph script")?)
        .arg("init")
        .arg(codegraph_cli_path(checkout))
        .current_dir(checkout)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .status()
        .context("initialize sample CodeGraph index")?;
    ensure!(
        status.success(),
        "CodeGraph initialization failed with {status}"
    );
    ensure!(
        checkout.join(".codegraph").is_dir(),
        "CodeGraph did not create an index"
    );
    fs::write(
        codegraph_home.join("state.json"),
        serde_json::to_vec_pretty(&json!({
            "schema_version": 1,
            "index": checkout.join(".codegraph"),
            "checkout": checkout
        }))?,
    )?;
    Ok(())
}

fn codegraph_cli_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    if let Some(plain) = path.to_string_lossy().strip_prefix(r"\\?\") {
        return PathBuf::from(plain);
    }
    path.to_path_buf()
}

fn copy_claude_auth_state(source: &Path, target: &Path) -> Result<()> {
    let state: Value = serde_json::from_slice(
        &fs::read(source).with_context(|| format!("read {}", source.display()))?,
    )?;
    let mut isolated = serde_json::Map::new();
    for key in ["oauthAccount", "primaryApiKey"] {
        if let Some(value) = state.get(key) {
            isolated.insert(key.to_owned(), value.clone());
        }
    }
    ensure!(
        !isolated.is_empty(),
        "Claude state contains no supported login material"
    );
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(target, serde_json::to_vec_pretty(&Value::Object(isolated))?)?;
    Ok(())
}

fn launch_native(
    args: &BenchmarkLauncherArgs,
    profile: &ConditionProfile,
    paths: &LauncherPaths,
    preparation: &LauncherPreparation,
) -> Result<std::process::ExitStatus> {
    let bounded = BOUNDED_PROMPT
        .replace("{max_turns}", &args.max_turns.to_string())
        .replace("{max_tool_calls}", &args.max_tool_calls.to_string())
        .replace("{prompt}", &args.prompt);
    let mut command = Command::new(&paths.native_harness);
    command.current_dir(&args.checkout);
    match profile.harness {
        BenchmarkHarness::ClaudeCode => {
            command.args([
                "--print",
                "--verbose",
                "--output-format",
                "stream-json",
                "--include-hook-events",
                "--permission-mode",
                "acceptEdits",
                "--setting-sources",
                "project",
                "--settings",
            ]);
            command.arg(&preparation.settings_path);
            command.args(["--strict-mcp-config", "--mcp-config"]);
            command.arg(
                preparation
                    .mcp_config_path
                    .as_ref()
                    .context("Claude MCP config")?,
            );
            command.args(["--no-session-persistence", "--session-id"]);
            command.arg(Uuid::now_v7().to_string());
            command.arg(bounded);
            command.env("CLAUDE_CONFIG_DIR", &args.harness_home);
        }
        BenchmarkHarness::Codex => {
            command.args([
                "exec",
                "--json",
                "--dangerously-bypass-hook-trust",
                "--strict-config",
                "--ignore-rules",
                "--sandbox",
                "workspace-write",
                "--cd",
            ]);
            command.arg(&args.checkout);
            command.arg("--skip-git-repo-check");
            command.arg(bounded);
            command.env("CODEX_HOME", &args.harness_home);
        }
    }
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with(BENCHMARK_ENV_PREFIX) {
            command.env_remove(key);
        }
    }
    // Both conditions receive the same fresh user home. This blocks accidental native session,
    // memory, MCP, hook, or instruction discovery from the operator's real profile while keeping
    // authentication available only through the copied, short-lived credential file.
    command.env("USERPROFILE", &args.harness_home);
    command.env("HOME", &args.harness_home);
    if !profile.condition.requires_brain_service() {
        command.env_remove("BRAIN_HOME");
        command.env_remove("BRAIN_PIPE_NAME");
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .with_context(|| format!("launch {}", paths.native_harness.display()))
}

fn credential_path(harness: BenchmarkHarness, home: &Path) -> PathBuf {
    match harness {
        BenchmarkHarness::ClaudeCode => home.join(".credentials.json"),
        BenchmarkHarness::Codex => home.join("auth.json"),
    }
}

struct CredentialGuard(Vec<PathBuf>);

impl CredentialGuard {
    fn new(harness: BenchmarkHarness, home: &Path) -> Self {
        let mut paths = vec![credential_path(harness, home)];
        if harness == BenchmarkHarness::ClaudeCode {
            paths.push(home.join(".claude.json"));
        }
        Self(paths)
    }
}

impl Drop for CredentialGuard {
    fn drop(&mut self) {
        for path in &self.0 {
            let _ = fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(harness: BenchmarkHarness, condition: BenchmarkCondition) -> ConditionProfile {
        let events = match condition {
            BenchmarkCondition::C0 | BenchmarkCondition::C1 => Vec::new(),
            BenchmarkCondition::C2 => vec!["SessionStart", "SessionEnd"],
            BenchmarkCondition::C3 | BenchmarkCondition::C4 => {
                vec!["SessionStart", "SessionEnd", "UserPromptSubmit"]
            }
        };
        ConditionProfile {
            schema_version: 2,
            harness,
            condition,
            tools: json!({ "codegraph": { "enabled": condition != BenchmarkCondition::C0 } }),
            hooks: if events.is_empty() {
                json!({})
            } else {
                json!({ "agent_brain": { "events": events } })
            },
            mcp: if condition == BenchmarkCondition::C4 {
                json!({ "agent_brain": {} })
            } else {
                json!({})
            },
        }
    }

    #[test]
    fn capability_profiles_are_exactly_cumulative() {
        for harness in BenchmarkHarness::ALL {
            for condition in BenchmarkCondition::ALL {
                validate_profile_capabilities(&profile(harness, condition)).expect("valid profile");
            }
        }
    }

    #[test]
    fn claude_auth_state_copy_keeps_only_login_material() {
        let temp = tempfile::tempdir().expect("temp");
        let source = temp.path().join("operator-claude.json");
        let target = temp.path().join("isolated/.claude.json");
        fs::write(
            &source,
            serde_json::to_vec(&json!({
                "oauthAccount": { "accountUuid": "fixture" },
                "primaryApiKey": "managed-fixture",
                "mcpServers": { "forbidden": { "command": "outside.exe" } },
                "projects": { "C:/private": { "memory": "forbidden" } }
            }))
            .unwrap(),
        )
        .expect("source state");

        copy_claude_auth_state(&source, &target).expect("copy auth state");

        let copied: Value = serde_json::from_slice(&fs::read(target).unwrap()).unwrap();
        assert!(copied.get("oauthAccount").is_some());
        assert!(copied.get("primaryApiKey").is_some());
        assert!(copied.get("mcpServers").is_none());
        assert!(copied.get("projects").is_none());
    }

    #[cfg(windows)]
    #[test]
    fn codegraph_cli_path_strips_the_windows_verbatim_prefix() {
        let path = Path::new(r"\\?\C:\benchmark\checkout");
        assert_eq!(
            codegraph_cli_path(path),
            PathBuf::from(r"C:\benchmark\checkout")
        );
    }

    #[test]
    fn claude_c2_has_boundaries_but_no_prompt_push_or_brain_mcp() {
        let temp = tempfile::tempdir().expect("temp");
        let hook = temp.path().join("brain-hook.exe");
        fs::write(&hook, b"hook").expect("hook");
        let hooks = hook_document(BenchmarkHarness::ClaudeCode, Some(&hook), false).expect("hooks");
        assert!(hooks.get("SessionStart").is_some());
        assert!(hooks.get("SessionEnd").is_some());
        assert!(hooks.get("UserPromptSubmit").is_none());
    }

    #[test]
    fn codex_toml_contains_only_explicit_mcp_servers() {
        let mut output = String::new();
        push_toml_mcp(
            &mut output,
            "brain",
            Path::new(r"C:\AgentBrain\bin\brain-mcp.exe"),
            &[],
        )
        .expect("TOML");
        assert!(output.contains("[mcp_servers.brain]"));
        assert!(!output.contains("codegraph"));
    }

    #[test]
    fn every_condition_materializes_only_its_assigned_capabilities() {
        let temp = tempfile::tempdir().expect("temp");
        let checkout = temp.path().join("checkout");
        fs::create_dir_all(&checkout).expect("checkout");
        let file = |name: &str| {
            let path = temp.path().join(name);
            fs::write(&path, name.as_bytes()).expect("fixture file");
            path
        };
        let common = LauncherPaths {
            native_harness: file("native.exe"),
            codegraph_node: Some(file("node.exe")),
            codegraph_script: Some(file("codegraph.js")),
            brain_hook: Some(file("brain-hook.exe")),
            brain_mcp: Some(file("brain-mcp.exe")),
            credential: None,
            claude_state: None,
            brain_home: None,
        };
        for condition in BenchmarkCondition::ALL {
            let sample_root = temp.path().join(condition.as_str());
            let harness_home = sample_root.join("harness");
            let codegraph_home = sample_root.join("codegraph");
            fs::create_dir_all(&sample_root).expect("sample");
            let mut paths = common.clone();
            if condition.requires_brain_service() {
                let brain_home = sample_root.join("brain");
                fs::create_dir_all(&brain_home).expect("brain");
                paths.brain_home = Some(brain_home);
            }
            let args = BenchmarkLauncherArgs {
                condition_config: temp.path().join("unused.json"),
                checkout: checkout.clone(),
                sample_root,
                harness_home,
                codegraph_home,
                max_turns: 12,
                max_tool_calls: 35,
                prompt: "bounded fixture prompt".to_owned(),
                prepare_only: true,
            };
            let prepared = prepare(
                &args,
                &profile(BenchmarkHarness::Codex, condition),
                &paths,
                false,
            )
            .expect("prepare condition");
            assert_eq!(
                prepared.codegraph_enabled,
                condition != BenchmarkCondition::C0
            );
            assert_eq!(
                prepared.session_start_enabled,
                condition.requires_brain_service()
            );
            assert_eq!(
                prepared.prompt_push_enabled,
                matches!(condition, BenchmarkCondition::C3 | BenchmarkCondition::C4)
            );
            assert_eq!(
                prepared.brain_mcp_enabled,
                condition == BenchmarkCondition::C4
            );
        }
    }
}
