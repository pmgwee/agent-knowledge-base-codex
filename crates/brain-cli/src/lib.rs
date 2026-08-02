#![forbid(unsafe_code)]

mod install_hooks;
mod register;
mod status;

pub use install_hooks::{HookInstallResult, install_claude_hooks, uninstall_claude_hooks};
pub use register::{RegisterOptions, RegistrationResult, register_project};
pub use status::{BrainStatus, read_status};
