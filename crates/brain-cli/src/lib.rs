#![forbid(unsafe_code)]

mod install_hooks;
mod register;
mod status;

pub use install_hooks::{
    HookInstallResult, install_claude_hooks, install_codex_hooks, uninstall_claude_hooks,
    uninstall_codex_hooks,
};
pub use register::{RegisterOptions, RegistrationResult, register_project};
pub use status::{BrainStatus, HermesStatus, read_hermes_status, read_status};
