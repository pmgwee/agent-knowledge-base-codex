use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use async_trait::async_trait;

use crate::{ConsolidationLlm, EvidencePacket, ProposedMemoryBatch};

#[derive(Clone, Debug)]
pub struct GlmConfig {
    pub endpoint: String,
    pub model: String,
    pub api_key_env: String,
    pub timeout: Duration,
    pub max_retries: u32,
}

pub struct GlmClient {
    config: GlmConfig,
    client: reqwest::Client,
}

impl GlmClient {
    pub fn new(config: GlmConfig) -> Result<Self> {
        let endpoint = reqwest::Url::parse(&config.endpoint).context("invalid GLM endpoint")?;
        ensure!(
            matches!(endpoint.scheme(), "http" | "https"),
            "GLM endpoint must use HTTP or HTTPS"
        );
        ensure!(!config.model.trim().is_empty(), "GLM model is empty");
        ensure!(
            !config.api_key_env.trim().is_empty(),
            "GLM key environment name is empty"
        );
        ensure!(!config.timeout.is_zero(), "GLM timeout must be positive");
        let client = reqwest::Client::builder().timeout(config.timeout).build()?;
        Ok(Self { config, client })
    }
}

/// The same endpoint, asked to merge two claims rather than distil a batch.
///
/// Deliberately thin: the caller supplies the whole instruction, because the rules a merge must
/// satisfy live beside the validator that enforces them (`brain_context::merge`) rather than being
/// split across a client that cannot check them.
#[async_trait]
impl crate::MergeProvider for GlmClient {
    async fn merge(&self, instruction: &str) -> Result<String> {
        let key = std::env::var(&self.config.api_key_env).with_context(|| {
            format!(
                "GLM API key environment variable {} is unavailable",
                self.config.api_key_env
            )
        })?;
        ensure!(!key.trim().is_empty(), "GLM API key is empty");
        let request = serde_json::json!({
            "model": self.config.model,
            "temperature": 0,
            "response_format": {"type": "json_object"},
            "messages": [{"role": "user", "content": instruction}]
        });
        let response = self
            .client
            .post(&self.config.endpoint)
            .bearer_auth(&key)
            .json(&request)
            .send()
            .await
            .context("send GLM merge request")?;
        ensure!(
            response.status().is_success(),
            "GLM merge request failed with HTTP status {}",
            response.status()
        );
        let text = response.text().await.context("read GLM merge response")?;
        // The chat envelope is the same; only the payload inside differs, so the existing extractor
        // would work — but it parses straight into a memory batch. Pull the content out and let the
        // merge module interpret it.
        let value: serde_json::Value = serde_json::from_str(&text).context("parse GLM envelope")?;
        Ok(value
            .pointer("/choices/0/message/content")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(&text)
            .to_owned())
    }
}

#[async_trait]
impl ConsolidationLlm for GlmClient {
    async fn propose(&self, packet: &EvidencePacket) -> Result<ProposedMemoryBatch> {
        let key = std::env::var(&self.config.api_key_env).with_context(|| {
            format!(
                "GLM API key environment variable {} is unavailable",
                self.config.api_key_env
            )
        })?;
        ensure!(!key.trim().is_empty(), "GLM API key is empty");
        let request = serde_json::json!({
            "model": self.config.model,
            "temperature": 0,
            "response_format": {"type": "json_object"},
            "messages": [
                {
                    "role": "system",
                    // The allowed `kind` values are spelled out because the schema is rejected
                    // whole when one is wrong, and a model given only field names invents
                    // plausible ones — "project" was the first thing GLM produced against real
                    // evidence, and every memory in the batch was discarded for it.
                    "content": "Return only strict JSON matching {\"memories\":[{\"kind\":..., \"title\":..., \"content\":..., \"valid_from\":..., \"confidence\":..., \"evidence_ids\":[...], \"supersedes\":[...]}]}.\n\
        kind MUST be exactly one of: checkpoint, decision, fact, investigation, procedure, deployment, timeline, task. Use no other value; pick the closest one.\n\
        valid_from MUST be RFC 3339, e.g. 2026-08-06T10:00:00Z.\n\
        confidence MUST be a number between 0 and 1.\n\
        evidence_ids MUST be a non-empty array of event_id values copied verbatim from the supplied evidence. Never invent one.\n\
        supersedes MUST be an array, [] when nothing is superseded. Never null.\n\
        Add no fields beyond those seven. Never propose preferences or secrets.
        When trigger is \"session_stopped\" the evidence is one complete working session: propose one memory of kind checkpoint summarising what was attempted, what was decided and where it was left, in addition to any specific facts or decisions worth keeping on their own. For any other trigger the span is an arbitrary cut through ongoing work, so propose only the specific claims the evidence supports and do not summarise it as though it were finished."
                },
                {"role": "user", "content": packet.serialized()}
            ]
        });
        let mut attempt = 0;
        loop {
            let response = self
                .client
                .post(&self.config.endpoint)
                .bearer_auth(&key)
                .json(&request)
                .send()
                .await;
            match response {
                Ok(response) if response.status().is_success() => {
                    let text = response.text().await.context("read GLM response")?;
                    return parse_glm_chat_response(&text);
                }
                Ok(response)
                    if (response.status().as_u16() == 429
                        || response.status().is_server_error())
                        && attempt < self.config.max_retries =>
                {
                    attempt += 1;
                    tokio::time::sleep(Duration::from_millis(200 * u64::from(attempt))).await;
                }
                Ok(response) => {
                    bail!("GLM request failed with HTTP status {}", response.status())
                }
                Err(error)
                    if (error.is_timeout() || error.is_connect())
                        && attempt < self.config.max_retries =>
                {
                    attempt += 1;
                    tokio::time::sleep(Duration::from_millis(200 * u64::from(attempt))).await;
                }
                Err(error) => return Err(error).context("GLM request failed"),
            }
        }
    }
}

pub fn parse_glm_chat_response(value: &str) -> Result<ProposedMemoryBatch> {
    let response: ChatCompletionResponse =
        serde_json::from_str(value).context("GLM response is not valid JSON")?;
    ensure!(
        response.choices.len() == 1,
        "GLM response must contain one choice"
    );
    let content = &response.choices[0].message.content;
    // Carry both halves of the diagnosis. A slice of the offending output says what the model
    // wrote; serde's own message says which field was wrong, and without it the reader has the
    // evidence but not the verdict — three dead-lettered jobs recorded 400 characters of
    // plausible-looking JSON and no reason at all. The two are put in one string rather than
    // left to the error chain because the chain is what the caller kept losing.
    serde_json::from_str(content).map_err(|error| {
        anyhow::anyhow!(
            "GLM message content does not match the proposed-memory schema ({error}); \
             content was: {}",
            crate::truncate_for_error(content, 400)
        )
    })
}

#[derive(serde::Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<ChatChoice>,
}

#[derive(serde::Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[derive(serde::Deserialize)]
struct ChatMessage {
    content: String,
}
