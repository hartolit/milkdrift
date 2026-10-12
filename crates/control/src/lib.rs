//! Admit ordinary workflow requests through authenticated, scoped authority.
//!
//! [`TaskControlService`] shares one command path across browser, CLI, service and model callers.
//! The runtime atomically validates future definitions and records exact recoverable receipts.

mod command;
mod error;
mod preset;
mod tasks;

pub use command::ActorAuthorityContext;
pub use error::ControlError;
pub use preset::{AuthorityPreset, GrantTemplate};
pub use tasks::TaskControlService;
