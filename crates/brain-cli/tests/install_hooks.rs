use brain_cli::{install_claude_hooks, uninstall_claude_hooks};

#[test]
fn install_is_backed_up_idempotent_and_preserves_unrelated_hooks() {
    let temp = tempfile::tempdir().expect("create hook fixture");
    let settings = temp.path().join("settings.json");
    let hook_exe = temp.path().join("brain-hook.exe");
    std::fs::write(&hook_exe, b"fixture").expect("create hook executable fixture");
    std::fs::write(
        &settings,
        serde_json::to_vec_pretty(&serde_json::json!({
            "theme": "dark",
            "hooks": {
                "PreToolUse": [{
                    "matcher": "Bash",
                    "hooks": [{"type": "command", "command": "existing.exe"}]
                }]
            }
        }))
        .expect("serialize settings fixture"),
    )
    .expect("write settings fixture");

    let installed = install_claude_hooks(&settings, &hook_exe).expect("install Claude hook");
    assert!(installed.changed);
    assert!(
        installed
            .backup_path
            .as_ref()
            .is_some_and(|path| path.is_file())
    );

    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&settings).expect("read installed settings"))
            .expect("parse installed settings");
    assert_eq!(value["theme"], "dark");
    assert_eq!(
        value["hooks"]["PreToolUse"][0]["hooks"][0]["command"],
        "existing.exe"
    );
    let session_start = value["hooks"]["SessionStart"]
        .as_array()
        .expect("SessionStart groups");
    assert_eq!(session_start.len(), 1);
    assert_eq!(
        session_start[0]["hooks"][0]["command"],
        hook_exe.to_string_lossy().as_ref()
    );
    assert_eq!(
        session_start[0]["hooks"][0]["args"],
        serde_json::json!(["--harness", "claude-code"])
    );

    let repeated = install_claude_hooks(&settings, &hook_exe).expect("repeat hook install");
    assert!(!repeated.changed);
    assert!(repeated.backup_path.is_none());
    let repeated_value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&settings).expect("read repeated settings"))
            .expect("parse repeated settings");
    assert_eq!(
        repeated_value["hooks"]["SessionStart"]
            .as_array()
            .expect("SessionStart groups")
            .len(),
        1
    );

    let removed = uninstall_claude_hooks(&settings, &hook_exe).expect("uninstall Claude hook");
    assert!(removed.changed);
    assert!(
        removed
            .backup_path
            .as_ref()
            .is_some_and(|path| path.is_file())
    );
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&settings).expect("read uninstalled settings"))
            .expect("parse uninstalled settings");
    assert!(value["hooks"].get("SessionStart").is_none());
    assert_eq!(
        value["hooks"]["PreToolUse"][0]["hooks"][0]["command"],
        "existing.exe"
    );
    assert_eq!(value["theme"], "dark");
}

#[test]
fn invalid_settings_are_rejected_without_replacement() {
    let temp = tempfile::tempdir().expect("create invalid hook fixture");
    let settings = temp.path().join("settings.json");
    let hook_exe = temp.path().join("brain-hook.exe");
    std::fs::write(&settings, b"{not-json").expect("write invalid settings");
    std::fs::write(&hook_exe, b"fixture").expect("create hook executable fixture");

    assert!(install_claude_hooks(&settings, &hook_exe).is_err());
    assert_eq!(
        std::fs::read(&settings).expect("read unchanged settings"),
        b"{not-json"
    );
}

#[test]
fn install_registers_session_end_as_well_as_session_start() {
    // Without this the brain cannot observe a session boundary at all. Transcripts are append-only
    // JSONL — a session ending writes no line, the file just stops growing — so `session.ended` had
    // never been emitted once across 139,192 captured events, and the consolidation reason keyed to
    // it was unreachable.
    let temp = tempfile::tempdir().expect("fixture");
    let settings = temp.path().join("settings.json");
    let hook_exe = temp.path().join("brain-hook.exe");
    std::fs::write(&hook_exe, b"fixture").expect("hook executable");
    std::fs::write(&settings, b"{}").expect("settings");

    assert!(
        install_claude_hooks(&settings, &hook_exe)
            .expect("install")
            .changed
    );
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&settings).expect("read")).expect("parse");

    for event in ["SessionStart", "SessionEnd"] {
        let groups = value["hooks"][event]
            .as_array()
            .unwrap_or_else(|| panic!("{event} must be registered"));
        assert_eq!(groups.len(), 1, "{event} should have exactly one group");
        assert_eq!(
            groups[0]["hooks"][0]["command"],
            hook_exe.to_string_lossy().as_ref()
        );
        assert_eq!(
            groups[0]["hooks"][0]["args"],
            serde_json::json!(["--harness", "claude-code"])
        );
    }
}

#[test]
fn a_config_holding_only_the_old_session_start_entry_is_upgraded() {
    // The upgrade path that would otherwise fail silently: the presence check keys off the binary,
    // and an existing `SessionStart` entry pointing at the right executable used to read as "already
    // current". The install would report no change and leave `SessionEnd` unregistered forever.
    let temp = tempfile::tempdir().expect("fixture");
    let settings = temp.path().join("settings.json");
    let hook_exe = temp.path().join("brain-hook.exe");
    std::fs::write(&hook_exe, b"fixture").expect("hook executable");
    std::fs::write(
        &settings,
        serde_json::to_vec_pretty(&serde_json::json!({
            "hooks": {
                "SessionStart": [{
                    "matcher": "startup|resume|clear|compact|fork",
                    "hooks": [{
                        "type": "command",
                        "command": hook_exe.to_string_lossy(),
                        "args": ["--harness", "claude-code"],
                        "timeout": 10
                    }]
                }]
            }
        }))
        .expect("serialize"),
    )
    .expect("write");

    let installed = install_claude_hooks(&settings, &hook_exe).expect("install");
    assert!(
        installed.changed,
        "a config missing SessionEnd is not current, however right its SessionStart entry looks"
    );

    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&settings).expect("read")).expect("parse");
    assert!(value["hooks"]["SessionEnd"].is_array());
    assert_eq!(
        value["hooks"]["SessionStart"]
            .as_array()
            .expect("SessionStart")
            .len(),
        1,
        "the old entry must be replaced, not appended to — a duplicate double-fires every session"
    );

    // And now it really is idempotent.
    assert!(
        !install_claude_hooks(&settings, &hook_exe)
            .expect("repeat")
            .changed
    );
}

#[test]
fn uninstall_removes_both_events() {
    let temp = tempfile::tempdir().expect("fixture");
    let settings = temp.path().join("settings.json");
    let hook_exe = temp.path().join("brain-hook.exe");
    std::fs::write(&hook_exe, b"fixture").expect("hook executable");
    std::fs::write(&settings, b"{\"theme\":\"dark\"}").expect("settings");

    install_claude_hooks(&settings, &hook_exe).expect("install");
    assert!(
        uninstall_claude_hooks(&settings, &hook_exe)
            .expect("uninstall")
            .changed
    );

    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&settings).expect("read")).expect("parse");
    assert!(
        value.get("hooks").is_none(),
        "both events gone means the whole hooks object goes, leaving no empty scaffolding"
    );
    assert_eq!(value["theme"], "dark", "unrelated settings must survive");
}
