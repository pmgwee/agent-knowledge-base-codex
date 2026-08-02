use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use brain_domain::HookEnvelope;

pub(crate) fn write(brain_home: &Path, envelope: &HookEnvelope) -> Result<()> {
    let spool_dir = brain_home.join("runtime").join("spool");
    std::fs::create_dir_all(&spool_dir)
        .with_context(|| format!("create spool directory {}", spool_dir.display()))?;
    let stem = envelope.nonce.to_string();
    let temporary = spool_dir.join(format!("{stem}.jsonl.tmp"));
    let destination = spool_dir.join(format!("{stem}.jsonl"));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .with_context(|| format!("create spool file {}", temporary.display()))?;
    serde_json::to_writer(&mut file, envelope).context("serialize spooled hook envelope")?;
    file.write_all(b"\n")
        .context("terminate spooled envelope")?;
    file.sync_all().context("flush spooled envelope")?;
    drop(file);
    std::fs::rename(&temporary, &destination)
        .with_context(|| format!("atomically publish spool file {}", destination.display()))?;
    Ok(())
}
