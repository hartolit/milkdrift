//! Generation behavior grouped by scheduler lifecycle invariant.

#[path = "generation/admission_and_capacity.rs"]
mod admission_and_capacity;
#[path = "generation/backend_contracts.rs"]
mod backend_contracts;
#[path = "generation/backpressure.rs"]
mod backpressure;
#[path = "generation/cancellation_and_drain.rs"]
mod cancellation_and_drain;
#[path = "generation/cleanup_interaction.rs"]
mod cleanup_interaction;
#[path = "generation/normal_output.rs"]
mod normal_output;
#[path = "generation/stop_and_sampling.rs"]
mod stop_and_sampling;
#[path = "generation/support/mod.rs"]
mod support;
