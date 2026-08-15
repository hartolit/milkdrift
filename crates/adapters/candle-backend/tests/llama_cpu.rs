//! Candle CPU compatibility coverage grouped by source and lifecycle invariant.

#[path = "llama_cpu/configuration_declarations.rs"]
mod configuration_declarations;
#[path = "llama_cpu/footprints_and_reservations.rs"]
mod footprints_and_reservations;
#[path = "llama_cpu/metadata_limits.rs"]
mod metadata_limits;
#[path = "llama_cpu/prepared_lifecycle.rs"]
mod prepared_lifecycle;
#[path = "llama_cpu/scalar_layouts.rs"]
mod scalar_layouts;
#[path = "llama_cpu/selective_materialization.rs"]
mod selective_materialization;
#[path = "llama_cpu/source_identity.rs"]
mod source_identity;
#[path = "llama_cpu/support/mod.rs"]
mod support;
