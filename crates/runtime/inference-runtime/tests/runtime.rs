//! Runtime lifecycle coverage grouped by ownership boundary.

#[path = "runtime/command_events.rs"]
mod command_events;
#[path = "runtime/memory_accounting.rs"]
mod memory_accounting;
#[path = "runtime/model_lifecycle.rs"]
mod model_lifecycle;
#[path = "runtime/sequence_lifecycle.rs"]
mod sequence_lifecycle;
#[path = "runtime/shutdown.rs"]
mod shutdown;
#[path = "runtime/support.rs"]
mod support;
