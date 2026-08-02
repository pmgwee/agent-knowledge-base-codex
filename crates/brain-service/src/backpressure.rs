use std::path::Path;

use anyhow::Result;

#[derive(Clone, Copy, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct DiskSample {
    pub available_bytes: u64,
    pub total_bytes: u64,
}

impl DiskSample {
    pub fn available_percent(self) -> f64 {
        if self.total_bytes == 0 {
            0.0
        } else {
            (self.available_bytes as f64 / self.total_bytes as f64) * 100.0
        }
    }
}

pub trait DiskProbe: Send + Sync {
    fn sample(&self, path: &Path) -> Result<DiskSample>;
}

pub struct FilesystemDiskProbe;

impl DiskProbe for FilesystemDiskProbe {
    fn sample(&self, path: &Path) -> Result<DiskSample> {
        Ok(DiskSample {
            available_bytes: fs2::available_space(path)?,
            total_bytes: fs2::total_space(path)?,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct PressurePolicy {
    pub warning_free_bytes: u64,
    pub critical_free_bytes: u64,
    pub emergency_free_bytes: u64,
    pub recovery_free_bytes: u64,
    pub warning_free_percent: f64,
    pub critical_free_percent: f64,
    pub recovery_free_percent: f64,
    pub recovery_checks: u8,
}

impl Default for PressurePolicy {
    fn default() -> Self {
        Self {
            warning_free_bytes: 5 * 1024 * 1024 * 1024,
            critical_free_bytes: 2 * 1024 * 1024 * 1024,
            emergency_free_bytes: 256 * 1024 * 1024,
            recovery_free_bytes: 6 * 1024 * 1024 * 1024,
            warning_free_percent: 10.0,
            critical_free_percent: 5.0,
            recovery_free_percent: 12.0,
            recovery_checks: 2,
        }
    }
}

impl PressurePolicy {
    pub fn validate(self) -> Result<()> {
        anyhow::ensure!(
            self.emergency_free_bytes <= self.critical_free_bytes
                && self.critical_free_bytes <= self.warning_free_bytes
                && self.warning_free_bytes <= self.recovery_free_bytes,
            "disk byte thresholds must be ordered emergency <= critical <= warning <= recovery"
        );
        anyhow::ensure!(
            self.critical_free_percent <= self.warning_free_percent
                && self.warning_free_percent <= self.recovery_free_percent,
            "disk percent thresholds must be ordered critical <= warning <= recovery"
        );
        anyhow::ensure!(self.recovery_checks > 0, "recovery checks must be positive");
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct DegradationState {
    pub provider_refresh_paused: bool,
    pub basic_memory_paused: bool,
    pub markdown_projection_paused: bool,
    pub consolidation_paused: bool,
    pub cold_cache_paused: bool,
    pub capture_blocked: bool,
}

pub struct PressureController {
    policy: PressurePolicy,
    state: DegradationState,
    healthy_recovery_checks: u8,
}

impl PressureController {
    pub fn new(policy: PressurePolicy) -> Result<Self> {
        policy.validate()?;
        Ok(Self {
            policy,
            state: DegradationState::default(),
            healthy_recovery_checks: 0,
        })
    }

    pub fn evaluate(&mut self, sample: DiskSample) -> DegradationState {
        let percent = sample.available_percent();
        let emergency = sample.available_bytes <= self.policy.emergency_free_bytes;
        let critical = sample.available_bytes <= self.policy.critical_free_bytes
            || percent <= self.policy.critical_free_percent;
        let warning = sample.available_bytes <= self.policy.warning_free_bytes
            || percent <= self.policy.warning_free_percent;
        if emergency {
            self.healthy_recovery_checks = 0;
            self.state = DegradationState {
                provider_refresh_paused: true,
                basic_memory_paused: true,
                markdown_projection_paused: true,
                consolidation_paused: true,
                cold_cache_paused: true,
                capture_blocked: true,
            };
        } else if critical {
            self.healthy_recovery_checks = 0;
            self.state = DegradationState {
                provider_refresh_paused: true,
                basic_memory_paused: true,
                markdown_projection_paused: true,
                consolidation_paused: true,
                cold_cache_paused: true,
                capture_blocked: false,
            };
        } else if warning {
            self.healthy_recovery_checks = 0;
            self.state.provider_refresh_paused = true;
            self.state.basic_memory_paused = true;
            self.state.capture_blocked = false;
        } else if sample.available_bytes >= self.policy.recovery_free_bytes
            && percent >= self.policy.recovery_free_percent
        {
            self.healthy_recovery_checks = self.healthy_recovery_checks.saturating_add(1);
            if self.healthy_recovery_checks >= self.policy.recovery_checks {
                self.state = DegradationState::default();
                self.healthy_recovery_checks = 0;
            }
        } else {
            self.healthy_recovery_checks = 0;
        }
        self.state
    }

    pub const fn state(&self) -> DegradationState {
        self.state
    }
}
