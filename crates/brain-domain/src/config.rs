use std::path::PathBuf;

use anyhow::{Result, bail};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BrainConfig {
    pub brain_home: PathBuf,
}

impl BrainConfig {
    pub fn load() -> Result<Self> {
        Self::from_paths(
            std::env::var_os("BRAIN_HOME").map(PathBuf::from),
            std::env::var_os("USERPROFILE").map(PathBuf::from),
        )
    }

    pub fn brain_home() -> Result<PathBuf> {
        Ok(Self::load()?.brain_home)
    }

    pub fn from_paths(brain_home: Option<PathBuf>, user_profile: Option<PathBuf>) -> Result<Self> {
        if let Some(brain_home) = brain_home {
            return Ok(Self { brain_home });
        }
        let Some(user_profile) = user_profile else {
            bail!("BRAIN_HOME is unset and USERPROFILE is unavailable");
        };

        Ok(Self {
            brain_home: user_profile.join("AgentBrain"),
        })
    }
}
