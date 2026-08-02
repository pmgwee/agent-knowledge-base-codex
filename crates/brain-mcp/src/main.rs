use std::io::{BufRead, Write};
use std::path::PathBuf;

use anyhow::{Context, Result};
use brain_domain::BrainConfig;
use brain_mcp::{BrainTools, McpServer};
use brain_service::BrainQueryService;
use clap::Parser;

#[derive(Parser)]
#[command(name = "brain-mcp", about = "Agent Brain stdio MCP server")]
struct Args {
    #[arg(long)]
    brain_home: Option<PathBuf>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let brain_home = match args.brain_home {
        Some(path) => path,
        None => BrainConfig::brain_home().context("resolve BRAIN_HOME")?,
    };
    let tools = BrainTools::new(BrainQueryService::open(brain_home)?);
    let mut server = McpServer::new(tools);
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        if let Some(response) = server.handle_line(&line?) {
            writeln!(stdout, "{response}")?;
            stdout.flush()?;
        }
    }
    Ok(())
}
