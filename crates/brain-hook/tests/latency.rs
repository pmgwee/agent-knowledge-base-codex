use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const COLD_PROCESS_SAMPLES: usize = 20;
const MEASURED_SAMPLES: usize = 200;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "hardware release gate; run explicitly with --release"]
async fn warm_compiled_hook_p95_is_under_fifty_milliseconds() {
    let temp = tempfile::tempdir().expect("create latency fixture");
    let pipe_name = format!(r"\\.\pipe\agent-brain-latency-{}", uuid::Uuid::now_v7());
    let server_name = pipe_name.clone();
    let server = tokio::spawn(async move {
        brain_service_for_benchmark::serve_many(
            &server_name,
            COLD_PROCESS_SAMPLES + MEASURED_SAMPLES,
        )
        .await
        .expect("serve benchmark requests");
    });

    let mut cold_samples = Vec::with_capacity(COLD_PROCESS_SAMPLES);
    let mut warm_samples = Vec::with_capacity(MEASURED_SAMPLES);
    for sequence in 0..(COLD_PROCESS_SAMPLES + MEASURED_SAMPLES) {
        let started = Instant::now();
        let mut child = Command::new(env!("CARGO_BIN_EXE_brain-hook"))
            .arg("--harness")
            .arg("claude-code")
            .env("BRAIN_HOME", temp.path())
            .env("BRAIN_PIPE_NAME", &pipe_name)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start benchmark hook");
        writeln!(
            child.stdin.as_mut().expect("benchmark stdin"),
            "{{\"hook_event_name\":\"PostToolUse\",\"sequence\":{sequence}}}"
        )
        .expect("write benchmark payload");
        assert!(child.wait().expect("wait for benchmark hook").success());
        if sequence < COLD_PROCESS_SAMPLES {
            cold_samples.push(started.elapsed());
        } else {
            warm_samples.push(started.elapsed());
        }
    }
    server.await.expect("join benchmark server");

    cold_samples.sort_unstable();
    warm_samples.sort_unstable();
    let cold_p50 = percentile(&cold_samples, 50);
    let cold_p95 = percentile(&cold_samples, 95);
    let cold_p99 = percentile(&cold_samples, 99);
    let warm_p50 = percentile(&warm_samples, 50);
    let warm_p95 = percentile(&warm_samples, 95);
    let warm_p99 = percentile(&warm_samples, 99);
    eprintln!(
        "compiled hook latency: cold p50={cold_p50:?} p95={cold_p95:?} p99={cold_p99:?}; warm p50={warm_p50:?} p95={warm_p95:?} p99={warm_p99:?}"
    );
    assert!(
        cold_p95 <= Duration::from_millis(100),
        "cold compiled hook p95 {cold_p95:?} exceeds 100 ms"
    );
    assert!(
        warm_p95 <= Duration::from_millis(50),
        "warm compiled hook p95 {warm_p95:?} exceeds 50 ms"
    );
}

fn percentile(samples: &[Duration], percentile: usize) -> Duration {
    let rank = (samples.len() * percentile).div_ceil(100);
    samples[rank.saturating_sub(1)]
}

mod brain_service_for_benchmark {
    use anyhow::{Context, Result};
    use brain_domain::{
        HookEnvelope, HookReply, decode_hook_frame_length, decode_hook_frame_payload,
        encode_hook_frame,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::windows::named_pipe::ServerOptions;

    pub async fn serve_many(pipe_name: &str, count: usize) -> Result<()> {
        let mut options = ServerOptions::new();
        options
            .first_pipe_instance(true)
            .reject_remote_clients(true)
            .max_instances(2);
        let mut server = options
            .create(pipe_name)
            .context("create first benchmark pipe")?;
        options.first_pipe_instance(false);
        for sequence in 0..count {
            server.connect().await.context("connect benchmark client")?;
            let next = if sequence + 1 < count {
                Some(
                    options
                        .create(pipe_name)
                        .context("create next benchmark pipe")?,
                )
            } else {
                None
            };
            serve_connected(&mut server).await?;
            if let Some(next) = next {
                server = next;
            }
        }
        Ok(())
    }

    async fn serve_connected(
        server: &mut tokio::net::windows::named_pipe::NamedPipeServer,
    ) -> Result<()> {
        let mut prefix = [0_u8; 4];
        server.read_exact(&mut prefix).await?;
        let length = decode_hook_frame_length(prefix)?;
        let mut payload = vec![0_u8; length];
        server.read_exact(&mut payload).await?;
        let _: HookEnvelope = decode_hook_frame_payload(&payload)?;
        let frame = encode_hook_frame(&HookReply::default())?;
        server.write_all(&frame).await?;
        server.flush().await?;
        Ok(())
    }
}
