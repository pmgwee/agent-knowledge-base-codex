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
    assert_eq!(group["hooks"][0]["timeout"], 1);

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
