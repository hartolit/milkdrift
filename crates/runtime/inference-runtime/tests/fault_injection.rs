//! Transaction rollback tests using a deliberately nonconforming backend.

#![expect(
    clippy::result_large_err,
    reason = "test helpers return the runtime's bounded allocation-free structured error directly"
)]

#[path = "fault_injection/cleanup_fairness.rs"]
mod cleanup_fairness;
#[path = "fault_injection/complete_model_contract.rs"]
mod complete_model_contract;
#[path = "fault_injection/failed_materialization.rs"]
mod failed_materialization;
#[path = "fault_injection/load_contract.rs"]
mod load_contract;
#[path = "fault_injection/sequence_contract.rs"]
mod sequence_contract;
#[path = "fault_injection/shutdown_retention.rs"]
mod shutdown_retention;
#[path = "fault_injection/support/mod.rs"]
mod support;
#[path = "fault_injection/unload.rs"]
mod unload;
