//! Durable peer service behavior grouped by owned protocol boundary.

#[path = "peer_service/artifact_transfer.rs"]
mod artifact_transfer;
#[path = "peer_service/lifecycle.rs"]
mod lifecycle;
#[path = "peer_service/storage.rs"]
mod storage;
#[path = "peer_service/support.rs"]
mod support;
