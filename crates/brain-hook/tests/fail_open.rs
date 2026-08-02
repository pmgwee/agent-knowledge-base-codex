use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn missing_service_exits_zero_emits_valid_empty_output_and_spools() {
    let temp = tempfile::tempdir().expect("create hook fixture");
    let pipe_name = format!(r"\\.\pipe\agent-brain-absent-{}", uuid::Uuid::now_v7());
    let started = Instant::now();
    let mut child = Command::new(env!("CARGO_BIN_EXE_brain-hook"))
        .arg("--harness")
        .arg("claude-code")
        .env("BRAIN_HOME", temp.path())
        .env("BRAIN_PIPE_NAME", pipe_name)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start hook shim");
    child
        .stdin
        .as_mut()
        .expect("hook stdin")
        .write_all(br#"{"hook_event_name":"SessionStart","session_id":"fixture"}"#)
        .expect("write hook payload");

    let output = child.wait_with_output().expect("wait for hook shim");

    assert_eq!(output.status.code(), Some(0));
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "missing service must fail open within the 250 ms hook timeout plus process overhead"
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output.stdout).expect("valid hook JSON"),
        serde_json::json!({})
    );
    let spool = temp.path().join("runtime").join("spool");
    let entries = std::fs::read_dir(&spool)
        .expect("spool directory")
        .collect::<Result<Vec<_>, _>>()
        .expect("read spool entries");
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0]
            .path()
            .extension()
            .and_then(|value| value.to_str()),
        Some("jsonl")
    );
    let spooled = std::fs::read_to_string(entries[0].path()).expect("read spooled envelope");
    let envelope: brain_domain::HookEnvelope =
        serde_json::from_str(spooled.trim()).expect("valid spooled envelope");
    assert_eq!(envelope.protocol, brain_domain::HOOK_PROTOCOL_VERSION);
    assert_eq!(envelope.event_name, "SessionStart");
    assert_eq!(envelope.payload["session_id"], "fixture");
}
