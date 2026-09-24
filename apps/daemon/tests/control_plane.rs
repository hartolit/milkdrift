//! Loopback-only integration coverage for the durable daemon control plane.

#[path = "control_plane/control_workflows.rs"]
mod control_workflows;
#[path = "control_plane/direct.rs"]
mod direct;
#[path = "control_plane/durability.rs"]
mod durability;
#[path = "control_plane/operations.rs"]
mod operations;
#[path = "support/process.rs"]
mod process;
#[path = "control_plane/process_cleanup.rs"]
mod process_cleanup;
#[path = "control_plane/resources.rs"]
mod resources;
#[path = "control_plane/roles.rs"]
mod roles;
#[path = "control_plane/support.rs"]
mod support;

#[path = "control_plane/published.rs"]
mod published;
