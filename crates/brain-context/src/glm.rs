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
        Add no fields beyond those seven. Never propose preferences or secrets."
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
    // Carry a slice of the offending output into the error. "does not match the schema" alone
    // costs a reproduction run against the live provider to learn which field was wrong, and
    // the failure is recorded on the job where nobody can re-ask the model what it said.
    serde_json::from_str(content).with_context(|| {
        format!(
            "GLM message content does not match the proposed-memory schema; content was: {}",
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
