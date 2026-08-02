use std::time::Duration;

use anyhow::{Context, Result};
use brain_domain::{HookEnvelope, HookReply, decode_hook_frame_length, decode_hook_frame_payload};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::windows::named_pipe::ClientOptions;

pub use brain_domain::encode_hook_frame as encode_frame;

pub async fn request(
    pipe_name: &str,
    envelope: &HookEnvelope,
    timeout: Duration,
) -> Result<HookReply> {
    let frame = encode_frame(envelope)?;
    tokio::time::timeout(timeout, async {
        let mut client = loop {
            match ClientOptions::new().open(pipe_name) {
                Ok(client) => break client,
                Err(_) => tokio::time::sleep(Duration::from_millis(2)).await,
            }
        };
        client.write_all(&frame).await.context("write hook frame")?;
        client.flush().await.context("flush hook frame")?;
        read_reply(&mut client).await
    })
    .await
    .context("hook request exceeded hard timeout")?
}

async fn read_reply(
    client: &mut tokio::net::windows::named_pipe::NamedPipeClient,
) -> Result<HookReply> {
    let mut prefix = [0_u8; 4];
    client
        .read_exact(&mut prefix)
        .await
        .context("read hook reply length")?;
    let length = decode_hook_frame_length(prefix)?;
    let mut payload = vec![0_u8; length];
    client
        .read_exact(&mut payload)
        .await
        .context("read hook reply payload")?;
    decode_hook_frame_payload(&payload)
}
