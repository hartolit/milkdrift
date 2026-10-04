//! Loopback-only integration coverage for the durable daemon control plane.

#[path = "control_plane/authoring.rs"]
mod authoring;
#[path = "control_plane/authoring_cli.rs"]
mod authoring_cli;
#[path = "control_plane/binary.rs"]
mod binary;
#[path = "control_plane/control_workflows.rs"]
mod control_workflows;
#[path = "control_plane/direct.rs"]
mod direct;
#[path = "control_plane/direct_preparation.rs"]
mod direct_preparation;
#[path = "control_plane/durability.rs"]
mod durability;
#[path = "control_plane/independent_client.rs"]
mod independent_client;
#[path = "control_plane/inputs.rs"]
mod inputs;
#[path = "control_plane/inputs_cli.rs"]
mod inputs_cli;
#[path = "control_plane/learning.rs"]
mod learning;
#[path = "control_plane/operations.rs"]
mod operations;
#[path = "support/process.rs"]
mod process;
#[path = "control_plane/process_cleanup.rs"]
mod process_cleanup;
#[path = "control_plane/repair.rs"]
mod repair;
#[path = "control_plane/resources.rs"]
mod resources;
#[path = "control_plane/results.rs"]
mod results;
#[path = "control_plane/reuse.rs"]
mod reuse;
#[path = "control_plane/roles.rs"]
mod roles;
#[path = "control_plane/support.rs"]
mod support;

#[path = "control_plane/published.rs"]
mod published;
#[path = "control_plane/published_inputs.rs"]
mod published_inputs;
