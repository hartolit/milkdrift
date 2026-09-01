//! Loopback-only integration coverage for the durable daemon control plane.

#[path = "control_plane/control_workflows.rs"]
mod control_workflows;
#[path = "control_plane/durability.rs"]
mod durability;
#[path = "control_plane/operations.rs"]
mod operations;
#[path = "control_plane/support.rs"]
mod support;
