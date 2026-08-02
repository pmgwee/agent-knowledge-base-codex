use std::time::Duration;

use anyhow::{Result, bail};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaptureServiceConfig {
    pub reconciliation_interval: Duration,
    pub watcher_debounce: Duration,
}

impl CaptureServiceConfig {
    pub fn validate(&self) -> Result<()> {
        if self.reconciliation_interval.is_zero() {
            bail!("reconciliation interval must be greater than zero");
        }
        if self.watcher_debounce.is_zero() {
            bail!("watcher debounce must be greater than zero");
        }
        Ok(())
    }
}

impl Default for CaptureServiceConfig {
    fn default() -> Self {
        Self {
            reconciliation_interval: Duration::from_secs(2),
            watcher_debounce: Duration::from_millis(50),
        }
    }
}
