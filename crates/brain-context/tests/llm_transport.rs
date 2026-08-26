//! What actually goes out on the wire, asserted against a stub provider on a real socket.
//!
//! The contract tests cover the envelope the provider sends back. These cover the half that no
//! amount of parsing can check: the URL the request is addressed to, the model named in it, the
//! header carrying the key, and what happens when the provider says no or says nothing. Building
//! any of those wrong fails at runtime with a message — 404, 401, a hang — that reads like a
//! provider problem rather than ours.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use brain_context::{
    ConsolidationLlm, EvidencePacket, LlmAvailabilityError, LlmClient, LlmConfig, MergeProvider,
    RedactedEvidence,
};
use brain_domain::{EventType, ProjectId};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// The whole file is one test on purpose.
///
/// The key is read from the environment by name, so exercising the happy path means setting a
/// variable — and `std::env::set_var` is unsound while any other thread may be reading the
/// environment, which in a multi-test binary is exactly what the other tests are doing (reqwest
/// itself reads proxy variables when it builds a client). One test function means one test
/// thread, and the variable is set before anything else starts.
#[test]
fn the_wire_contract_holds() {
    // Not a secret: a literal consumed by a stub server on loopback that never checks it beyond
    // echoing which header it arrived in.
    const KEY_VARIABLE: &str = "BRAIN_TEST_LLM_KEY";
    const KEY_VALUE: &str = "stub-provider-token";

    // SAFETY: single-threaded, before any runtime, client or socket exists.
    unsafe { std::env::set_var(KEY_VARIABLE, KEY_VALUE) };

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime");

    runtime.block_on(async {
        a_generation_is_addressed_to_the_documented_path_with_the_configured_model(
            KEY_VARIABLE,
            KEY_VALUE,
        )
        .await;
        a_merge_uses_the_same_responses_transport(KEY_VARIABLE).await;
        a_rate_limit_is_retried_rather_than_failed(KEY_VARIABLE).await;
        an_exhausted_server_failure_remains_an_availability_error(KEY_VARIABLE).await;
        a_success_status_whose_body_stalls_is_retried(KEY_VARIABLE).await;
        an_authentication_failure_is_reported_and_never_retried(KEY_VARIABLE).await;
        a_provider_that_never_answers_gives_up_at_the_configured_timeout(KEY_VARIABLE).await;
        a_missing_key_variable_fails_before_any_request_is_made().await;
    });
}

async fn a_merge_uses_the_same_responses_transport(key_variable: &str) {
    let merged = r#"{"merged":"claim","evidence_ids":[]}"#;
    let stub = stub(vec![(200, text_envelope(merged))]).await;
    let client = client(&stub.base_url, key_variable, 0, Duration::from_secs(10));

    let output = client
        .merge("merge these claims")
        .await
        .expect("merge response text");
    assert_eq!(output, merged);
    let recorded = stub.recorded();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].target, "/zen/go/v1/responses");
    assert_eq!(recorded[0].body["store"].as_bool(), Some(false));
    assert!(recorded[0].body.get("temperature").is_none());
}

async fn a_generation_is_addressed_to_the_documented_path_with_the_configured_model(
    key_variable: &str,
    key_value: &str,
) {
    let stub = stub(vec![(200, memory_envelope())]).await;
    let client = client(&stub.base_url, key_variable, 2, Duration::from_secs(10));
    let packet = packet();

    let batch = client
        .propose(&packet)
        .await
        .expect("the stub provider answers a well-formed batch");
    assert_eq!(
        batch.memories.len(),
        1,
        "the proposal survives the round trip"
    );

    let recorded = stub.recorded();
    assert_eq!(recorded.len(), 1, "one request for one proposal");
    let request = &recorded[0];

    // The documented endpoint, appended to the configured root exactly once.
    assert_eq!(
        request.target, "/zen/go/v1/responses",
        "the request must reach the Responses path, got {}",
        request.target
    );
    assert_eq!(
        request.authorization.as_deref(),
        Some(format!("Bearer {key_value}").as_str()),
        "the key travels as a bearer token and nowhere else"
    );
    assert_eq!(
        request.body["model"].as_str(),
        Some("gpt-5.6-luna"),
        "the configured model must be the one asked"
    );

    // Responses API field names. The old Chat Completions ones are not errors the provider
    // reports — it ignores them, and the JSON constraint quietly stops being applied.
    assert!(
        request.body["instructions"].is_string(),
        "the system rules travel as `instructions`"
    );
    assert!(
        request.body["input"].is_string(),
        "the evidence packet travels as `input`"
    );
    assert_eq!(
        request.body["text"]["format"]["type"].as_str(),
        Some("json_object"),
        "JSON output is constrained through `text.format`"
    );
    assert!(
        request.body.get("temperature").is_none(),
        "GPT-5.6 Luna rejects `temperature`; the provider must choose its supported default"
    );
    assert!(
        request.body.get("messages").is_none(),
        "`messages` is the Chat Completions shape and must not be sent"
    );
    assert!(
        request.body.get("response_format").is_none(),
        "`response_format` is the Chat Completions shape and must not be sent"
    );
    assert_eq!(
        request.body["store"].as_bool(),
        Some(false),
        "private transcript evidence must not be retained by the provider"
    );
}

async fn a_rate_limit_is_retried_rather_than_failed(key_variable: &str) {
    // A rate limit is an outage, not a bad job — the packet is fine and asking again is the whole
    // remedy. Losing the retry would dead-letter work that was never in question.
    let stub = stub(vec![
        (429, r#"{"error":{"message":"slow down"}}"#.to_owned()),
        (200, memory_envelope()),
    ])
    .await;
    let client = client(&stub.base_url, key_variable, 2, Duration::from_secs(10));

    let batch = client
        .propose(&packet())
        .await
        .expect("the retry succeeds where the first attempt was throttled");
    assert_eq!(batch.memories.len(), 1);
    assert_eq!(stub.recorded().len(), 2, "the throttled attempt is retried");
}

async fn an_exhausted_server_failure_remains_an_availability_error(key_variable: &str) {
    let failure = r#"{"error":{"message":"try later"}}"#.to_owned();
    let stub = stub(vec![(500, failure.clone()), (500, failure)]).await;
    let client = client(&stub.base_url, key_variable, 1, Duration::from_secs(10));

    let error = client
        .propose(&packet())
        .await
        .expect_err("an exhausted 500 remains a provider outage");
    assert!(
        error.downcast_ref::<LlmAvailabilityError>().is_some(),
        "the worker must be able to distinguish provider availability from bad evidence: {error:#}"
    );
    assert_eq!(stub.recorded().len(), 2, "the configured retry is used");
}

async fn a_success_status_whose_body_stalls_is_retried(key_variable: &str) {
    // `1` scripts a complete successful response header followed by a body that never arrives.
    // This is distinct from a send timeout: the provider accepted the request, then its response
    // stream failed. The packet is still valid and must not be charged for that transport fault.
    let stub = stub(vec![(1, memory_envelope()), (200, memory_envelope())]).await;
    let client = client(&stub.base_url, key_variable, 1, Duration::from_millis(250));

    let batch = client
        .propose(&packet())
        .await
        .expect("a stalled successful body is retried");
    assert_eq!(batch.memories.len(), 1);
    assert_eq!(stub.recorded().len(), 2, "the stalled body is retried");
}

async fn an_authentication_failure_is_reported_and_never_retried(key_variable: &str) {
    // A rejected key does not become acceptable on the second ask. Retrying it would spend the
    // retry budget on a certainty and delay the error that names the problem.
    let stub = stub(vec![(401, r#"{"error":{"message":"no"}}"#.to_owned())]).await;
    let client = client(&stub.base_url, key_variable, 2, Duration::from_secs(10));

    let error = client
        .propose(&packet())
        .await
        .expect_err("a 401 is a failure");
    let text = format!("{error:#}");
    assert!(
        text.contains("401"),
        "the error must name the status, got: {text}"
    );
    assert_eq!(
        stub.recorded().len(),
        1,
        "an authentication failure is not retried"
    );
}

async fn a_provider_that_never_answers_gives_up_at_the_configured_timeout(key_variable: &str) {
    // `0` scripts a connection that is accepted and then left hanging.
    let stub = stub(vec![(0, String::new())]).await;
    let client = client(&stub.base_url, key_variable, 0, Duration::from_millis(250));

    let error = client
        .propose(&packet())
        .await
        .expect_err("a provider that never answers must not hang the worker");
    let text = format!("{error:#}").to_lowercase();
    assert!(
        text.contains("timed out") || text.contains("timeout"),
        "the error must be recognisable as a timeout, because that is what tells consolidation \
         this is an outage rather than a bad job, got: {text}"
    );
}

async fn a_missing_key_variable_fails_before_any_request_is_made() {
    let stub = stub(Vec::new()).await;
    let client = client(
        &stub.base_url,
        "BRAIN_TEST_LLM_KEY_THAT_IS_NEVER_SET",
        2,
        Duration::from_secs(10),
    );

    let error = client
        .propose(&packet())
        .await
        .expect_err("an unset key variable is a configuration failure");
    let text = format!("{error:#}");
    assert!(
        text.contains("BRAIN_TEST_LLM_KEY_THAT_IS_NEVER_SET"),
        "the error must name the variable to set, got: {text}"
    );
    assert!(
        stub.recorded().is_empty(),
        "nothing should reach the provider without a key"
    );
}

// ---------------------------------------------------------------------------
// The stub provider
// ---------------------------------------------------------------------------

fn client(base_url: &str, key_variable: &str, max_retries: u32, timeout: Duration) -> LlmClient {
    LlmClient::new(LlmConfig {
        base_url: base_url.to_owned(),
        model: "gpt-5.6-luna".to_owned(),
        api_key_env: key_variable.to_owned(),
        timeout,
        max_retries,
    })
    .expect("the stub configuration is sound")
}

#[derive(Clone, Debug)]
struct Recorded {
    target: String,
    authorization: Option<String>,
    body: serde_json::Value,
}

struct Stub {
    base_url: String,
    recorded: Arc<Mutex<Vec<Recorded>>>,
}

impl Stub {
    fn recorded(&self) -> Vec<Recorded> {
        self.recorded.lock().expect("stub records").clone()
    }
}

/// Answer a scripted sequence of responses, recording what each request carried.
///
/// A status of `0` means "accept the connection and never answer", which is how a hung provider
/// is scripted.
async fn stub(script: Vec<(u16, String)>) -> Stub {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind the stub provider");
    let address: SocketAddr = listener.local_addr().expect("stub provider address");
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&recorded);

    tokio::spawn(async move {
        for (status, body) in script {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let sink = Arc::clone(&sink);
            tokio::spawn(async move {
                if let Some(request) = read_request(&mut socket).await {
                    sink.lock().expect("stub records").push(request);
                }
                if status == 0 {
                    // Held open rather than dropped: a closed connection is a connect error, and
                    // the case under test is a provider that answers nothing at all.
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    return;
                }
                if status == 1 {
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                         Content-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    return;
                }
                let response = format!(
                    "HTTP/1.1 {status} STATUS\r\nContent-Type: application/json\r\n\
                     Content-Length: {length}\r\nConnection: close\r\n\r\n{body}",
                    length = body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            });
        }
    });

    Stub {
        // Deliberately more than one path segment, so a route built by string-joining rather than
        // by replacing the path is caught.
        base_url: format!("http://{address}/zen/go/v1"),
        recorded,
    }
}

async fn read_request(socket: &mut TcpStream) -> Option<Recorded> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        let read = socket.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
        let Some(head_end) = position(&buffer, b"\r\n\r\n") else {
            continue;
        };
        let head = String::from_utf8_lossy(&buffer[..head_end]).into_owned();
        let length = header(&head, "content-length")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        if buffer.len() < head_end + 4 + length {
            continue;
        }
        let mut request_line = head.lines().next()?.split_whitespace();
        let target = request_line.nth(1)?.to_owned();
        return Some(Recorded {
            target,
            authorization: header(&head, "authorization"),
            body: serde_json::from_slice(&buffer[head_end + 4..head_end + 4 + length]).ok()?,
        });
    }
}

fn position(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn header(head: &str, name: &str) -> Option<String> {
    head.lines().skip(1).find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.trim()
            .eq_ignore_ascii_case(name)
            .then(|| value.trim().to_owned())
    })
}

fn memory_envelope() -> String {
    serde_json::json!({
        "status": "completed",
        "output": [{
            "type": "message",
            "role": "assistant",
            "content": [{"type": "output_text", "text": serde_json::json!({
                "memories": [{
                    "kind": "fact",
                    "title": "a grounded fact",
                    "content": "the stub provider answered",
                    "valid_from": "2026-08-02T01:00:00Z",
                    "confidence": 0.9,
                    "evidence_ids": ["00000000-0000-0000-0000-000000000001"],
                    "supersedes": []
                }]
            }).to_string()}]
        }]
    })
    .to_string()
}

fn text_envelope(text: &str) -> String {
    serde_json::json!({
        "status": "completed",
        "output": [{
            "type": "message",
            "role": "assistant",
            "content": [{"type": "output_text", "text": text}]
        }]
    })
    .to_string()
}

fn packet() -> EvidencePacket {
    EvidencePacket {
        job_id: uuid::Uuid::now_v7(),
        project_id: ProjectId(uuid::Uuid::now_v7()),
        trigger: None,
        events: vec![RedactedEvidence {
            event_id: uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001")
                .expect("fixture event ID"),
            event_type: EventType::AgentResponded,
            occurred_at: time::OffsetDateTime::parse(
                "2026-08-02T01:00:00Z",
                &time::format_description::well_known::Rfc3339,
            )
            .expect("fixture time"),
            payload: serde_json::json!({"content": "OAuth callback tests pass"}),
            raw: serde_json::json!({}),
        }],
        redactions: Vec::new(),
        allowed_supersession_ids: Vec::new(),
    }
}
