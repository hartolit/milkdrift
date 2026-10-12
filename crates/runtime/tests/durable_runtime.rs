//! Production redb task lifecycle, authority, replay and fault evidence.
use milkdrift_authority::{GrantDigest, GrantId};
use milkdrift_runtime::CommandAuthorityClaim;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn claim() -> TestResult<CommandAuthorityClaim> {
    Ok(CommandAuthorityClaim::new(
        GrantId::new("grant:durable-runtime")?,
        1,
        GrantDigest::new(format!("b3_{}", "0".repeat(64)))?,
        0,
    )?)
}

#[path = "durable_runtime/tasks.rs"]
mod tasks;
