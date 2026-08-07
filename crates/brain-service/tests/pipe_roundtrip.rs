use std::time::Duration;

use brain_domain::{HOOK_PROTOCOL_VERSION, Harness, HookEnvelope, HookReply};
use brain_service::{HookOutcome, HookPipeServer};

#[tokio::test]
async fn length_prefixed_hook_request_round_trips_over_a_local_named_pipe() {
    let pipe_name = format!(r"\\.\pipe\agent-brain-test-{}", uuid::Uuid::now_v7());
    let server_name = pipe_name.clone();
    let server = tokio::spawn(async move {
        HookPipeServer::new(server_name)
            .serve_once(|envelope| async move {
                assert_eq!(envelope.event_name, "SessionStart");
                Ok(HookOutcome::bare(HookReply {
                    additional_context: Some("bounded fixture context".to_owned()),
                    diagnostics_id: Some(envelope.nonce.to_string()),
                }))
            })
            .await
    });
    let envelope = HookEnvelope {
        protocol: HOOK_PROTOCOL_VERSION,
        harness: Harness::ClaudeCode,
        event_name: "SessionStart".to_owned(),
        received_at: time::OffsetDateTime::now_utc(),
        nonce: uuid::Uuid::now_v7(),
        payload: serde_json::json!({"session_id": "pipe-fixture"}),
    };

    let reply = brain_hook::protocol::request(&pipe_name, &envelope, Duration::from_secs(2))
        .await
        .expect("pipe request succeeds");

    assert_eq!(
        reply.additional_context.as_deref(),
        Some("bounded fixture context")
    );
    let expected_diagnostics = envelope.nonce.to_string();
    assert_eq!(
        reply.diagnostics_id.as_deref(),
        Some(expected_diagnostics.as_str())
    );
    server.await.expect("join server").expect("server succeeds");
}

#[tokio::test]
async fn persistent_pipe_accepts_multiple_requests_and_shuts_down_cleanly() {
    let pipe_name = format!(
        r"\\.\pipe\agent-brain-persistent-test-{}",
        uuid::Uuid::now_v7()
    );
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let server_name = pipe_name.clone();
    let server = tokio::spawn(async move {
        HookPipeServer::new(server_name)
            .run(shutdown_rx, |envelope| async move {
                Ok(HookOutcome::bare(HookReply {
                    additional_context: Some(envelope.event_name),
                    diagnostics_id: None,
                }))
            })
            .await
    });

    for event_name in ["SessionStart", "PostToolUse"] {
        let envelope = HookEnvelope {
            protocol: HOOK_PROTOCOL_VERSION,
            harness: Harness::ClaudeCode,
            event_name: event_name.to_owned(),
            received_at: time::OffsetDateTime::now_utc(),
            nonce: uuid::Uuid::now_v7(),
            payload: serde_json::json!({}),
        };
        let reply = brain_hook::protocol::request(&pipe_name, &envelope, Duration::from_secs(2))
            .await
            .expect("persistent pipe request succeeds");
        assert_eq!(reply.additional_context.as_deref(), Some(event_name));
    }

    shutdown_tx.send(true).expect("request pipe shutdown");
    server
        .await
        .expect("join persistent pipe")
        .expect("persistent pipe stops cleanly");
}
