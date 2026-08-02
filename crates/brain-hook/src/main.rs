use std::io::Read;
use std::path::PathBuf;

use brain_domain::{BrainConfig, HOOK_MAX_FRAME_BYTES, Harness};
use brain_hook::{DEFAULT_PIPE_NAME, HOOK_HARD_TIMEOUT, HookOptions};
use clap::Parser;

#[derive(clap::Parser)]
struct Args {
    #[arg(long, default_value = "claude-code")]
    harness: String,
    #[arg(long)]
    event_name: Option<String>,
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args = match Args::try_parse() {
        Ok(args) => args,
        Err(error) => {
            eprintln!("brain-hook argument error: {error}");
            print_empty_reply();
            return;
        }
    };
    let brain_home = BrainConfig::brain_home().unwrap_or_else(|error| {
        eprintln!("brain-hook configuration error: {error:#}");
        PathBuf::from("AgentBrain")
    });
    let pipe_name =
        std::env::var("BRAIN_PIPE_NAME").unwrap_or_else(|_| DEFAULT_PIPE_NAME.to_owned());
    let harness = parse_harness(&args.harness);
    let input = read_stdin();
    let output = brain_hook::invoke(
        HookOptions {
            brain_home,
            pipe_name,
            harness,
            event_name: args.event_name,
            timeout: HOOK_HARD_TIMEOUT,
        },
        &input,
    )
    .await;
    match serde_json::to_string(&output) {
        Ok(output) => println!("{output}"),
        Err(error) => {
            eprintln!("brain-hook output error: {error:#}");
            print_empty_reply();
        }
    }
}

fn read_stdin() -> Vec<u8> {
    let limit = u64::try_from(HOOK_MAX_FRAME_BYTES).expect("frame limit fits in u64") + 1;
    let mut input = Vec::new();
    if let Err(error) = std::io::stdin().take(limit).read_to_end(&mut input) {
        eprintln!("brain-hook stdin error: {error:#}");
    }
    if input.len() > HOOK_MAX_FRAME_BYTES {
        input.truncate(HOOK_MAX_FRAME_BYTES);
        eprintln!("brain-hook stdin exceeded {HOOK_MAX_FRAME_BYTES} bytes and was truncated");
    }
    input
}

fn parse_harness(value: &str) -> Harness {
    match value {
        "claude" | "claude-code" => Harness::ClaudeCode,
        "codex" => Harness::Codex,
        "hermes" | "hermes-agent" => Harness::Hermes,
        other => Harness::Other(other.to_owned()),
    }
}

fn print_empty_reply() {
    println!("{{}}");
}
