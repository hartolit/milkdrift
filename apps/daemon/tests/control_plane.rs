//! Loopback-only integration tests of ordinary tasks and retained independent serving.

#[path = "control_plane/binary.rs"]
mod binary;
#[path = "control_plane/diagnostics.rs"]
mod diagnostics;
#[path = "control_plane/direct.rs"]
mod direct;
#[path = "control_plane/direct_preparation.rs"]
mod direct_preparation;
#[path = "control_plane/independent_client.rs"]
mod independent_client;
#[path = "control_plane/model.rs"]
mod model;
#[path = "control_plane/operations.rs"]
mod operations;
#[path = "support/process.rs"]
mod process;
#[path = "control_plane/process_cleanup.rs"]
mod process_cleanup;
#[path = "control_plane/required_inputs.rs"]
mod required_inputs;
#[path = "control_plane/resources.rs"]
mod resources;
#[path = "control_plane/roles.rs"]
mod roles;
#[path = "control_plane/streams.rs"]
mod streams;
#[path = "control_plane/support.rs"]
mod support;
#[path = "control_plane/tasks.rs"]
mod tasks;
