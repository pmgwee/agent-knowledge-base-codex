use anyhow::{Context, Result, bail};
use serde::de::DeserializeOwned;

use crate::Harness;

pub const HOOK_PROTOCOL_VERSION: u16 = 1;
pub const HOOK_MAX_FRAME_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct HookEnvelope {
    pub protocol: u16,
    pub harness: Harness,
    pub event_name: String,
    pub received_at: time::OffsetDateTime,
    pub nonce: uuid::Uuid,
    pub payload: serde_json::Value,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct HookReply {
    pub additional_context: Option<String>,
    pub diagnostics_id: Option<String>,
}

pub fn encode_hook_frame<T: serde::Serialize>(value: &T) -> Result<Vec<u8>> {
    let payload = serde_json::to_vec(value).context("serialize hook frame")?;
    if payload.len() > HOOK_MAX_FRAME_BYTES {
        bail!(
            "hook frame size {} exceeds {} bytes",
            payload.len(),
            HOOK_MAX_FRAME_BYTES
        );
    }
    let length = u32::try_from(payload.len()).context("hook frame length exceeds u32")?;
    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(&length.to_le_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

pub fn decode_hook_frame_length(prefix: [u8; 4]) -> Result<usize> {
    let length = usize::try_from(u32::from_le_bytes(prefix)).context("invalid frame length")?;
    if length > HOOK_MAX_FRAME_BYTES {
        bail!("hook frame size {length} exceeds {HOOK_MAX_FRAME_BYTES} bytes");
    }
    Ok(length)
}

pub fn decode_hook_frame_payload<T: DeserializeOwned>(payload: &[u8]) -> Result<T> {
    if payload.len() > HOOK_MAX_FRAME_BYTES {
        bail!(
            "hook frame size {} exceeds {} bytes",
            payload.len(),
            HOOK_MAX_FRAME_BYTES
        );
    }
    serde_json::from_slice(payload).context("decode hook frame JSON")
}
