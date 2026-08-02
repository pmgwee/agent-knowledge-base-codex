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
                    "content": "Return only strict JSON matching {memories:[{kind,title,content,valid_from,confidence,evidence_ids,supersedes}]}. Cite only supplied evidence IDs. Never propose preferences or secrets."
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
    serde_json::from_str(&response.choices[0].message.content)
        .context("GLM message content does not match the proposed-memory schema")
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
