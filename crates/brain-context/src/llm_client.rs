use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use async_trait::async_trait;

use crate::{ConsolidationLlm, EvidencePacket, ProposedMemoryBatch};

/// The rules a consolidation proposal must satisfy, sent as the request's `instructions`.
///
/// The allowed `kind` values are spelled out because the schema is rejected whole when one is
/// wrong, and a model given only field names invents plausible ones — "project" was the first
/// thing a provider produced against real evidence, and every memory in the batch was discarded
/// for it.
const CONSOLIDATION_INSTRUCTIONS: &str = "Return only strict JSON matching {\"memories\":[{\"kind\":..., \"title\":..., \"content\":..., \"valid_from\":..., \"confidence\":..., \"evidence_ids\":[...], \"supersedes\":[...]}]}.\n\
        kind MUST be exactly one of: checkpoint, decision, fact, investigation, procedure, deployment, timeline, task. Use no other value; pick the closest one.\n\
        valid_from MUST be RFC 3339, e.g. 2026-08-06T10:00:00Z.\n\
        confidence MUST be a number between 0 and 1.\n\
        evidence_ids MUST be a non-empty array of event_id values copied verbatim from the supplied evidence. Never invent one.\n\
        supersedes MUST be an array, [] when nothing is superseded. Never null.\n\
        Add no fields beyond those seven. Never propose preferences or secrets.
        When trigger is \"session_stopped\" the evidence is one complete working session: propose one memory of kind checkpoint summarising what was attempted, what was decided and where it was left, in addition to any specific facts or decisions worth keeping on their own. For any other trigger the span is an arbitrary cut through ongoing work, so propose only the specific claims the evidence supports and do not summarise it as though it were finished.";

/// Where the provider lives, which model answers, and which environment variable holds the key.
///
/// Provider-neutral by construction: nothing here names a vendor. Swapping providers is a change
/// to configuration — `base_url` and `model` — rather than a change to this crate.
#[derive(Clone, Debug)]
pub struct LlmConfig {
    /// The API root (preferred), or a complete endpoint ending in `/responses` exactly once.
    pub base_url: String,
    pub model: String,
    /// The *name* of the environment variable holding the key. Never the key itself — this
    /// struct is cloned into logs' reach and the key must not travel with it.
    pub api_key_env: String,
    pub timeout: Duration,
    pub max_retries: u32,
}

pub struct LlmClient {
    config: LlmConfig,
    responses_url: String,
    client: reqwest::Client,
}

/// A provider or credential failure that says nothing about the evidence packet's validity.
///
/// Consolidation downcasts this type before deciding whether a leased job should consume an
/// attempt. Keeping the distinction typed avoids coupling evidence retention to provider error
/// prose, while the service still has a conservative text fallback for third-party providers.
#[derive(Debug, thiserror::Error)]
pub enum LlmAvailabilityError {
    #[error("LLM API key environment variable {variable} is unavailable")]
    CredentialUnavailable { variable: String },
    #[error("LLM request failed with HTTP status {status}")]
    HttpStatus { status: reqwest::StatusCode },
    #[error("LLM response transport failed: {message}")]
    Transport { message: String },
}

impl LlmClient {
    pub fn new(config: LlmConfig) -> Result<Self> {
        let responses_url = responses_url(&config.base_url)?;
        ensure!(!config.model.trim().is_empty(), "LLM model is empty");
        ensure!(
            !config.api_key_env.trim().is_empty(),
            "LLM key environment name is empty"
        );
        ensure!(!config.timeout.is_zero(), "LLM timeout must be positive");
        let client = reqwest::Client::builder().timeout(config.timeout).build()?;
        Ok(Self {
            config,
            responses_url,
            client,
        })
    }

    /// The URL requests actually go to. Exposed so a test can assert the route rather than infer
    /// it, since building it wrong is silent — the provider answers a 404 that reads like an outage.
    pub fn responses_url(&self) -> &str {
        &self.responses_url
    }

    pub fn model(&self) -> &str {
        &self.config.model
    }

    /// Read the key by the configured variable name.
    ///
    /// The value never enters an error, a log line or a returned string: the caller learns which
    /// variable was consulted and whether it resolved, which is everything needed to fix it.
    fn api_key(&self) -> Result<String> {
        let unavailable = || LlmAvailabilityError::CredentialUnavailable {
            variable: self.config.api_key_env.clone(),
        };
        let key = std::env::var(&self.config.api_key_env).map_err(|_| unavailable())?;
        if key.trim().is_empty() {
            return Err(unavailable().into());
        }
        Ok(key)
    }

    /// The Responses API request body.
    ///
    /// `instructions` carries what was a `system` message and `input` what was a `user` message;
    /// `text.format` is where the Responses API puts what Chat Completions called
    /// `response_format`. Sending the old field names is not an error the provider reports — it
    /// ignores them, and the JSON constraint quietly stops being applied.
    fn request(&self, instructions: Option<&str>, input: &str) -> serde_json::Value {
        let mut request = serde_json::json!({
            "model": self.config.model,
            "input": input,
            "text": {"format": {"type": "json_object"}},
            // Evidence packets are drawn from private transcripts. Nothing about this work needs
            // the provider to retain them between calls, so it is asked not to.
            "store": false,
        });
        if let Some(instructions) = instructions {
            request["instructions"] = serde_json::Value::String(instructions.to_owned());
        }
        request
    }

    async fn send_request(&self, key: &str, request: &serde_json::Value) -> Result<String> {
        let mut attempt = 0;
        loop {
            let response = self
                .client
                .post(&self.responses_url)
                .bearer_auth(key)
                .json(request)
                .send()
                .await;
            match response {
                Ok(response) if response.status().is_success() => match response.text().await {
                    Ok(text) => return Ok(text),
                    Err(_error) if attempt < self.config.max_retries => {
                        attempt += 1;
                        tokio::time::sleep(Duration::from_millis(200 * u64::from(attempt))).await;
                    }
                    Err(error) => {
                        return Err(LlmAvailabilityError::Transport {
                            message: transport_message(&error),
                        }
                        .into());
                    }
                },
                Ok(response)
                    if retryable_status(response.status()) && attempt < self.config.max_retries =>
                {
                    attempt += 1;
                    tokio::time::sleep(Duration::from_millis(200 * u64::from(attempt))).await;
                }
                Ok(response)
                    if retryable_status(response.status())
                        || matches!(response.status().as_u16(), 401 | 403) =>
                {
                    return Err(LlmAvailabilityError::HttpStatus {
                        status: response.status(),
                    }
                    .into());
                }
                Ok(response) => {
                    bail!("LLM request failed with HTTP status {}", response.status())
                }
                Err(error)
                    if (error.is_timeout() || error.is_connect())
                        && attempt < self.config.max_retries =>
                {
                    attempt += 1;
                    tokio::time::sleep(Duration::from_millis(200 * u64::from(attempt))).await;
                }
                Err(error) => {
                    return Err(LlmAvailabilityError::Transport {
                        message: transport_message(&error),
                    }
                    .into());
                }
            }
        }
    }
}

fn retryable_status(status: reqwest::StatusCode) -> bool {
    status.as_u16() == 429 || status.is_server_error() || matches!(status.as_u16(), 408 | 425)
}

fn transport_message(error: &reqwest::Error) -> String {
    let kind = if error.is_timeout() {
        "request timed out"
    } else if error.is_connect() {
        "connection failed"
    } else if error.is_body() {
        "response body failed"
    } else {
        "request transport failed"
    };
    format!("{kind}: {error}")
}

/// Join the configured API root to the documented `/responses` path.
///
/// A complete `/responses` endpoint is accepted exactly once for paste-friendly configuration. A
/// base still ending in the old Chat Completions path is refused rather than silently mangled.
pub fn responses_url(base_url: &str) -> Result<String> {
    let trimmed = base_url.trim();
    ensure!(!trimmed.is_empty(), "LLM base URL is empty");
    let mut parsed = reqwest::Url::parse(trimmed).context("invalid LLM base URL")?;
    ensure!(
        parsed.query().is_none(),
        "LLM base URL must not contain a query"
    );
    ensure!(
        parsed.fragment().is_none(),
        "LLM base URL must not contain a fragment"
    );
    ensure!(
        parsed.username().is_empty() && parsed.password().is_none(),
        "LLM base URL must not contain credentials"
    );
    ensure!(
        matches!(parsed.scheme(), "http" | "https"),
        "LLM base URL must use HTTP or HTTPS"
    );
    let path = parsed.path().trim_end_matches('/').to_owned();
    ensure!(
        !path.ends_with("/chat/completions"),
        "LLM base URL must be the API root, not a Chat Completions endpoint; \
         drop the trailing /chat/completions"
    );
    if !path.ends_with("/responses") {
        parsed.set_path(&format!("{path}/responses"));
    } else {
        parsed.set_path(&path);
    }
    Ok(parsed.to_string())
}

/// The same endpoint, asked to merge two claims rather than distil a batch.
///
/// Deliberately thin: the caller supplies the whole instruction, because the rules a merge must
/// satisfy live beside the validator that enforces them (`brain_context::merge`) rather than being
/// split across a client that cannot check them.
#[async_trait]
impl crate::MergeProvider for LlmClient {
    async fn merge(&self, instruction: &str) -> Result<String> {
        let key = self.api_key()?;
        let text = self
            .send_request(&key, &self.request(None, instruction))
            .await
            .context("LLM merge request failed")?;
        // The envelope is the same as consolidation's; only the payload inside differs, so the
        // existing extractor would work — but it parses straight into a memory batch. Pull the
        // content out and let the merge module interpret it.
        output_text(&text).context("parse LLM merge response")
    }
}

#[async_trait]
impl ConsolidationLlm for LlmClient {
    async fn propose(&self, packet: &EvidencePacket) -> Result<ProposedMemoryBatch> {
        let key = self.api_key()?;
        let request = self.request(Some(CONSOLIDATION_INSTRUCTIONS), &packet.serialized());
        let text = self.send_request(&key, &request).await?;
        parse_llm_response(&text)
    }
}

pub fn parse_llm_response(value: &str) -> Result<ProposedMemoryBatch> {
    let content = output_text(value)?;
    // Carry both halves of the diagnosis. A slice of the offending output says what the model
    // wrote; serde's own message says which field was wrong, and without it the reader has the
    // evidence but not the verdict — three dead-lettered jobs recorded 400 characters of
    // plausible-looking JSON and no reason at all. The two are put in one string rather than
    // left to the error chain because the chain is what the caller kept losing.
    serde_json::from_str(&content).map_err(|error| {
        anyhow::anyhow!(
            "LLM message content does not match the proposed-memory schema ({error}); \
             content was: {}",
            crate::truncate_for_error(&content, 400)
        )
    })
}

/// Pull the assistant's text out of a Responses API envelope.
///
/// `output` is a **list of items**, and the message is not reliably its first element — a
/// reasoning item precedes it whenever the model emits one. So the message is searched for rather
/// than indexed, and an envelope whose shape is assumed instead of read is exactly how a working
/// integration returns "no assistant text" on the day the model starts reasoning.
fn output_text(value: &str) -> Result<String> {
    let value: serde_json::Value =
        serde_json::from_str(value).context("LLM response is not valid JSON")?;

    if let Some(error) = value.get("error").filter(|error| !error.is_null()) {
        let message = error
            .get("message")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("no message given");
        bail!(
            "LLM returned an error: {}",
            crate::truncate_for_error(message, 300)
        );
    }

    // A response that stopped early carries a *partial* body, which would otherwise be reported as
    // "does not match the schema" — true, but naming the wrong cause, and the retry that follows
    // is pointless where a truncation needs a smaller packet.
    match value.get("status").and_then(serde_json::Value::as_str) {
        Some(status) if status != "completed" => {
            let reason = value
                .pointer("/incomplete_details/reason")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("no reason given");
            bail!("LLM response is {status} rather than completed ({reason})");
        }
        _ => {}
    }

    let mut text = String::new();
    for item in value
        .get("output")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
    {
        if item.get("type").and_then(serde_json::Value::as_str) != Some("message") {
            continue;
        }
        for part in item
            .get("content")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
        {
            if part.get("type").and_then(serde_json::Value::as_str) == Some("output_text") {
                if let Some(chunk) = part.get("text").and_then(serde_json::Value::as_str) {
                    text.push_str(chunk);
                }
            }
        }
    }

    // The aggregated convenience field, when the gateway supplies one and the walk above found
    // nothing. Checked second: `output` is the normative shape.
    if text.is_empty() {
        if let Some(flat) = value.get("output_text").and_then(serde_json::Value::as_str) {
            text.push_str(flat);
        }
    }

    ensure!(
        !text.trim().is_empty(),
        "LLM response carried no assistant text"
    );
    Ok(text)
}
