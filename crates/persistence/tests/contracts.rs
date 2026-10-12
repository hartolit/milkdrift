//! Contracts retained by task persistence and independent host boundaries.
use milkdrift_persistence::{
    ArtifactPublicationId, ArtifactStore, BoundedDetail, NativePathClaimStore, PageSize,
    WorkflowStore,
};
#[test]
fn identity_text_and_page_bounds_are_enforced() {
    assert!(ArtifactPublicationId::new("").is_err());
    assert!(ArtifactPublicationId::new("unsafe identity").is_err());
    assert!(ArtifactPublicationId::new("x".repeat(193)).is_err());
    assert!(BoundedDetail::new("contains\ncontrol").is_err());
    assert!(BoundedDetail::new("é".repeat(2_048)).is_ok());
    assert!(BoundedDetail::new("é".repeat(2_049)).is_err());
    assert!(PageSize::new(0).is_err());
    assert!(PageSize::new(1_001).is_err());
}
#[test]
fn persistence_ports_are_object_safe() {
    fn ports(_: &dyn WorkflowStore, _: &dyn ArtifactStore, _: &dyn NativePathClaimStore) {}
    let _ = ports;
}
