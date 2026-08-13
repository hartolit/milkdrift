//! Translation from lower cleanup evidence into the stable application contract.

use inference_runtime::{
    CleanupFailureReport, CleanupResource, CleanupRetryState, ConservativeFootprint,
    RetainedOwnership,
};

use crate::{
    ApplicationConservativeFootprint, ApplicationFailure, ApplicationFailureKind,
    ApplicationModelCleanupDisposition, ApplicationRetainedModelResource,
    ApplicationRetainedOwnership,
};

pub(super) const fn application_cleanup_resource(
    resource: CleanupResource,
) -> ApplicationRetainedModelResource {
    match resource {
        CleanupResource::Model { handle } => {
            ApplicationRetainedModelResource::LoadedModel { handle }
        }
        CleanupResource::IncompatibleModel { handle } => {
            ApplicationRetainedModelResource::IncompatibleModel { handle }
        }
        CleanupResource::FailedLoad { handle } => {
            ApplicationRetainedModelResource::FailedLoad { handle }
        }
        CleanupResource::Sequence { .. } => ApplicationRetainedModelResource::UnconfirmedModel,
    }
}

pub(super) const fn application_cleanup_disposition(
    cleanup: CleanupRetryState,
) -> ApplicationModelCleanupDisposition {
    if cleanup.exhausted() {
        ApplicationModelCleanupDisposition::LowerExhausted {
            attempts: cleanup.attempts,
            maximum_attempts: cleanup.maximum_attempts,
        }
    } else {
        ApplicationModelCleanupDisposition::LowerRetryable {
            attempts: cleanup.attempts,
            maximum_attempts: cleanup.maximum_attempts,
        }
    }
}

pub(super) fn application_retained_ownership(
    ownership: RetainedOwnership,
) -> ApplicationRetainedOwnership {
    match ownership {
        RetainedOwnership::Released => ApplicationRetainedOwnership::Unknown,
        RetainedOwnership::Exact(footprint) => ApplicationRetainedOwnership::Exact(footprint),
        RetainedOwnership::Unverified {
            accepted_footprint,
            reported_footprint,
            conservative_footprint,
        } => ApplicationRetainedOwnership::Unverified {
            accepted_loading_peak: accepted_footprint,
            reported_footprint,
            conservative_footprint: match conservative_footprint {
                ConservativeFootprint::Known(footprint) => {
                    ApplicationConservativeFootprint::Known(footprint)
                }
                ConservativeFootprint::Overflow => ApplicationConservativeFootprint::Overflow,
            },
        },
    }
}

pub(super) fn application_primary_failure(report: CleanupFailureReport) -> ApplicationFailure {
    let kind = match report.primary_failure {
        inference_runtime::FailureClass::Load => ApplicationFailureKind::ModelLoad,
        inference_runtime::FailureClass::Capacity => ApplicationFailureKind::MemoryAdmission,
        inference_runtime::FailureClass::BackendContract => {
            ApplicationFailureKind::IncompatibleReceipt
        }
        _ => ApplicationFailureKind::Inference,
    };
    ApplicationFailure::from_debug(
        kind,
        "model operation failed before retained cleanup",
        (report.primary_operation, report.primary_detail),
    )
}

pub(super) fn application_cleanup_failure(report: CleanupFailureReport) -> ApplicationFailure {
    ApplicationFailure::from_debug(
        ApplicationFailureKind::RetainedCleanup,
        "explicit lower cleanup failed",
        (report.cleanup_operation, report.cleanup_detail),
    )
}
