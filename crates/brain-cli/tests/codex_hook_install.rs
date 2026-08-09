use brain_cli::{install_codex_hooks, uninstall_codex_hooks};

#[test]
fn codex_hook_install_is_additive_idempotent_and_leaves_notify_untouched() {
    let temp = tempfile::tempdir().expect("create Codex hook fixture");
    let codex_home = temp.path().join(".codex");
    std::fs::create_dir_all(&codex_home).expect("create Codex home");
    let hooks_path = codex_home.join("hooks.json");
    let config_path = codex_home.join("config.toml");
    let hook_exe = temp.path().join("brain-hook.exe");
    std::fs::write(&hook_exe, b"fixture").expect("create hook executable");
    let config = b"notify = [\"existing-notifier.exe\", \"turn-ended\"]\n";
    std::fs::write(&config_path, config).expect("write existing Codex config");
    std::fs::write(
        &hooks_path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "description": "existing hooks",
            "hooks": {
                "PreToolUse": [{
                    "matcher": "Bash",
                    "hooks": [{"type": "command", "command": "existing-policy.exe"}]
                }]
            }
        }))
        .expect("serialize existing hooks"),
    )
    .expect("write existing hooks");

    let installed = install_codex_hooks(&hooks_path, &hook_exe).expect("install Codex hook");
    assert!(installed.changed);
    assert!(
        installed
            .backup_path
            .as_ref()
            .is_some_and(|path| path.is_file())
    );
    assert_eq!(
        std::fs::read(&config_path).expect("read untouched config"),
        config
    );
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&hooks_path).expect("read installed hooks"))
            .expect("parse installed hooks");
    assert_eq!(value["description"], "existing hooks");
    assert_eq!(
        value["hooks"]["PreToolUse"][0]["hooks"][0]["command"],
        "existing-policy.exe"
    );
    let group = &value["hooks"]["SessionStart"][0];
    assert_eq!(group["matcher"], "^(startup|resume|clear|compact)$");
    assert_eq!(group["hooks"][0]["type"], "command");
    assert!(
        group["hooks"][0]["commandWindows"]
            .as_str()
            .is_some_and(|command| command.contains("--harness codex"))
    );
    assert_eq!(group["hooks"][0]["additionalContextLimit"], 1_500);
    // Outer budget must comfortably exceed the hook's internal fail-open deadline; see
    // CODEX_HOOK_TIMEOUT_SECONDS in install_hooks.rs.
    assert_eq!(group["hooks"][0]["timeout"], 15);

    let repeated = install_codex_hooks(&hooks_path, &hook_exe).expect("repeat Codex install");
    assert!(!repeated.changed);
    assert!(repeated.backup_path.is_none());
    let repeated_value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&hooks_path).expect("read repeated hooks"))
            .expect("parse repeated hooks");
    assert_eq!(
        repeated_value["hooks"]["SessionStart"]
            .as_array()
            .expect("SessionStart groups")
            .len(),
        1
    );

    let removed = uninstall_codex_hooks(&hooks_path, &hook_exe).expect("uninstall Codex hook");
    assert!(removed.changed);
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&hooks_path).expect("read uninstalled hooks"))
            .expect("parse uninstalled hooks");
    assert!(value["hooks"].get("SessionStart").is_none());
    assert_eq!(
        value["hooks"]["PreToolUse"][0]["hooks"][0]["command"],
        "existing-policy.exe"
    );
    assert_eq!(
        std::fs::read(&config_path).expect("read final config"),
        config
    );
}

/// The bug that made Codex hooks look unimplemented for five days.
///
/// Codex on Windows does not strip quotes from `commandWindows`, so
/// `"C:\…\brain-hook.exe" --harness codex` fails with `hook exited with code 1` while the unquoted
/// form runs. The installer emitted the quoted string for *both* fields, so every Codex install this
/// tool produced carried a hook that could not launch.
///
/// It was misread twice — first as "Codex Desktop does not implement hooks", then as an upstream
/// regression with a matching build number. Both conclusions came from zero deliveries *and* zero
/// spool entries, on the reasoning that a hook which fired and failed would still spool. It would —
/// but a hook that cannot *launch* never reaches our binary, so it neither delivers nor spools, and
/// that is indistinguishable from never being invoked.
#[test]
fn the_windows_command_is_unquoted_and_the_posix_one_is_not() {
    let directory = tempfile::tempdir().expect("tempdir");
    let executable = directory.path().join("brain-hook.exe");
    std::fs::write(&executable, b"binary").expect("write");
    let hooks = directory.path().join("hooks.json");

    brain_cli::install_codex_hooks(&hooks, &executable).expect("install");
    let document: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&hooks).expect("read")).expect("json");

    let mut checked = 0;
    for event in ["SessionStart", "SessionEnd", "UserPromptSubmit"] {
        let entry = &document["hooks"][event][0]["hooks"][0];
        let windows = entry["commandWindows"].as_str().expect("commandWindows");
        let posix = entry["command"].as_str().expect("command");
        assert!(
            !windows.starts_with('"'),
            "{event}: commandWindows must not quote the executable — Codex does not strip it"
        );
        assert!(
            posix.starts_with('"'),
            "{event}: the POSIX command still needs its quotes"
        );
        assert!(windows.ends_with("--harness codex"));
        checked += 1;
    }
    assert_eq!(checked, 3, "all three hooks are registered for Codex");
}

/// An unquoted command cannot survive a space, so the install refuses the path instead of writing a
/// hook that fails silently — the exact shape of defect this whole episode was.
#[test]
fn a_path_with_a_space_is_refused_rather_than_written_broken() {
    let directory = tempfile::tempdir().expect("tempdir");
    let spaced = directory.path().join("Agent Brain");
    std::fs::create_dir_all(&spaced).expect("mkdir");
    let executable = spaced.join("brain-hook.exe");
    std::fs::write(&executable, b"binary").expect("write");

    let error = brain_cli::install_codex_hooks(directory.path().join("hooks.json"), &executable)
        .expect_err("a space must be refused");
    assert!(
        error.to_string().contains("contains a space"),
        "the refusal must name the reason: {error}"
    );
}
