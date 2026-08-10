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

/// A slow request must not spend the budget of the one behind it.
///
/// This is the 10 August failure, reduced: a cold-cache `SessionStart` took 6,571 ms, and the two
/// hooks that arrived while it ran were never read until it finished — by which time their clients
/// had given up at `HOOK_HARD_TIMEOUT` and the replies were written to closed pipes. One slow
/// compile, three sessions without an orientation.
///
/// The accept loop used to `await` the handler, so "a second instance exists to connect to" and "a
/// second request is being served" were different things, and only the first was true. The timings
/// below are chosen so the serial arrangement cannot pass: the fast request is given less time than
/// the slow one still has left to run.
#[tokio::test]
async fn a_slow_request_does_not_starve_the_one_behind_it() {
    let pipe_name = format!(
        r"\\.\pipe\agent-brain-concurrent-test-{}",
        uuid::Uuid::now_v7()
    );
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let server_name = pipe_name.clone();
    let server = tokio::spawn(async move {
        HookPipeServer::new(server_name)
            .run(shutdown_rx, |envelope: HookEnvelope| async move {
                if envelope.event_name == "Slow" {
                    tokio::time::sleep(Duration::from_millis(1_500)).await;
                }
                Ok(HookOutcome::bare(HookReply {
                    additional_context: Some(envelope.event_name),
                    diagnostics_id: None,
                }))
            })
            .await
    });

    let envelope = |event_name: &str| HookEnvelope {
        protocol: HOOK_PROTOCOL_VERSION,
        harness: Harness::ClaudeCode,
        event_name: event_name.to_owned(),
        received_at: time::OffsetDateTime::now_utc(),
        nonce: uuid::Uuid::now_v7(),
        payload: serde_json::json!({}),
    };

    let slow_name = pipe_name.clone();
    let slow = tokio::spawn(async move {
        brain_hook::protocol::request(&slow_name, &envelope("Slow"), Duration::from_secs(10)).await
    });
    // Long enough that the slow request is definitely being served, short enough that it still has
    // ~1.4 s to run — so a serial server could not answer this one inside its 700 ms budget.
    tokio::time::sleep(Duration::from_millis(100)).await;

    let fast =
        brain_hook::protocol::request(&pipe_name, &envelope("Fast"), Duration::from_millis(700))
            .await
            .expect("a fast request is served while a slow one is still running");
    assert_eq!(fast.additional_context.as_deref(), Some("Fast"));

    // And the slow one still finishes: serving concurrently must not mean dropping it.
    let slow = slow
        .await
        .expect("join slow request")
        .expect("the slow request completes");
    assert_eq!(slow.additional_context.as_deref(), Some("Slow"));

    shutdown_tx.send(true).expect("request pipe shutdown");
    server
        .await
        .expect("join concurrent pipe")
        .expect("concurrent pipe stops cleanly");
}
