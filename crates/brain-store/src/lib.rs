#![forbid(unsafe_code)]

mod cursor;
mod ledger;
mod memory;
mod migrations;

pub use ledger::{AppendResult, EventLedger, StoredEvent};
pub use memory::GlobalPreferenceStore;
