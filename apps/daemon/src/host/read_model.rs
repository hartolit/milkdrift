use super::PublicFailure;
use milkdrift_authority::AuthorityDecisionSnapshot;
use milkdrift_control::ControlError;
use milkdrift_control_protocol::{ArtifactMetadataRead, CapabilityOperationRead, ErrorCode};
use milkdrift_persistence::PersistenceError;

pub(super) fn public_operation_contract(
    operation: &milkdrift_capability::OperationId,
    contract: &milkdrift_capability::OperationContract,
) -> CapabilityOperationRead {
    CapabilityOperationRead {
        operation: operation.as_str().to_owned(),
        side_effect: snake_debug(&contract.side_effect()),
        idempotency: snake_debug(&contract.idempotency()),
        cancellation: snake_debug(&contract.cancellation()),
        streaming: contract.streaming().iter().map(snake_debug).collect(),
    }
}

pub(super) fn public_artifact_metadata(
    value: &milkdrift_workspace::ArtifactMetadata,
) -> ArtifactMetadataRead {
    ArtifactMetadataRead {
        artifact_id: value.reference().artifact().as_str().to_owned(),
        digest: value.reference().digest().to_hex(),
        size: value.reference().size_bytes(),
        content_type: value.reference().media_type().as_str().to_owned(),
        disposition_name: None,
        sensitivity: snake_debug(&value.sensitivity()),
    }
}

pub(super) fn clock_unavailable() -> PublicFailure {
    PublicFailure::new(ErrorCode::Unavailable, "daemon clock unavailable", true)
}

pub(super) fn snake_debug(value: &impl std::fmt::Debug) -> String {
    let source = format!("{value:?}");
    let mut result = String::with_capacity(source.len() + 4);
    for (index, character) in source.chars().enumerate() {
        if character.is_ascii_uppercase() && index != 0 {
            result.push('_');
        }
        result.extend(character.to_lowercase());
    }
    result
}

pub(super) fn bounded(value: &str) -> String {
    milkdrift_contracts::truncate_utf8(value, 4_096).to_owned()
}

pub(super) fn invalid(message: &str) -> PublicFailure {
    PublicFailure::new(ErrorCode::InvalidInput, bounded(message), false)
}

pub(super) fn conflict(message: &str) -> PublicFailure {
    PublicFailure::new(ErrorCode::Conflict, message, false)
}

pub(super) fn unauthorized() -> PublicFailure {
    PublicFailure::new(
        ErrorCode::Unauthorized,
        "authority denied the operation",
        false,
    )
}

pub(super) fn unauthorized_decision(decision: &AuthorityDecisionSnapshot) -> PublicFailure {
    let mut failure = unauthorized();
    failure
        .details
        .insert("decision_digest".to_owned(), decision.digest().to_owned());
    failure.details.insert(
        "reason_codes".to_owned(),
        decision
            .reason_codes()
            .iter()
            .map(snake_debug)
            .collect::<Vec<_>>()
            .join(","),
    );
    failure
}

pub(super) fn not_found() -> PublicFailure {
    PublicFailure::new(
        ErrorCode::NotFound,
        "requested resource was not found",
        false,
    )
}

pub(super) fn internal() -> PublicFailure {
    PublicFailure::new(
        ErrorCode::Internal,
        "internal control operation failed",
        false,
    )
}

pub(super) fn public_control(error: ControlError) -> PublicFailure {
    match error {
        ControlError::AuthorizationDenied {
            reasons,
            decision_digest,
        } => {
            let mut failure = unauthorized();
            if let Some(digest) = decision_digest {
                failure.details.insert("decision_digest".to_owned(), digest);
            }
            failure.details.insert(
                "reason_codes".to_owned(),
                reasons
                    .iter()
                    .map(snake_debug)
                    .collect::<Vec<_>>()
                    .join(","),
            );
            failure
        }
        ControlError::Persistence(error) => public_persistence(error),
        ControlError::Runtime(milkdrift_runtime::RuntimeError::Persistence(error)) => {
            public_persistence(error)
        }
        ControlError::Runtime(milkdrift_runtime::RuntimeError::AuthorizationDenied { .. }) => {
            unauthorized()
        }
        ControlError::Runtime(error @ milkdrift_runtime::RuntimeError::InvalidTransition(_)) => {
            conflict(&bounded(&error.to_string()))
        }
        _ => invalid(&bounded(&error.to_string())),
    }
}

pub(super) fn public_persistence(error: PersistenceError) -> PublicFailure {
    match error {
        PersistenceError::NativePathContention => {
            PublicFailure::new(ErrorCode::Unavailable, "Project is in use", true)
        }
        PersistenceError::WorkflowGenerationConflict {
            expected, actual, ..
        } => {
            let mut failure = conflict("workflow generation guard is stale");
            if let Some(expected) = expected {
                failure
                    .details
                    .insert("expected_generation".into(), expected.to_string());
            }
            if let Some(actual) = actual {
                failure
                    .details
                    .insert("actual_generation".into(), actual.to_string());
            }
            failure
        }
        PersistenceError::DaemonGenerationConflict { .. }
        | PersistenceError::ProjectGenerationConflict { .. }
        | PersistenceError::ProjectIdempotencyConflict { .. }
        | PersistenceError::DaemonIdempotencyConflict { .. }
        | PersistenceError::WorkflowIdempotencyConflict { .. }
        | PersistenceError::ImmutableConflict { .. }
        | PersistenceError::WorkspaceUsageConflict { .. } => conflict(&bounded(&error.to_string())),
        PersistenceError::NotFound { .. } => not_found(),
        PersistenceError::ArtifactAccessDenied(_) => unauthorized(),
        PersistenceError::Corruption(_) => PublicFailure::new(
            ErrorCode::Corruption,
            "durable integrity verification failed",
            false,
        ),
        PersistenceError::UnsupportedVersion { .. }
        | PersistenceError::MigrationRequired { .. } => PublicFailure::new(
            ErrorCode::UnsupportedVersion,
            bounded(&error.to_string()),
            false,
        ),
        PersistenceError::Storage { class, .. } => {
            let code = match class {
                milkdrift_persistence::StorageFailureClass::Corruption => ErrorCode::Corruption,
                milkdrift_persistence::StorageFailureClass::ResourceExhausted => {
                    ErrorCode::Overload
                }
                milkdrift_persistence::StorageFailureClass::Unavailable
                | milkdrift_persistence::StorageFailureClass::OwnerBusy => ErrorCode::Unavailable,
                milkdrift_persistence::StorageFailureClass::Migration => {
                    ErrorCode::UnsupportedVersion
                }
                milkdrift_persistence::StorageFailureClass::Internal => ErrorCode::Internal,
            };
            PublicFailure::new(
                code,
                "durable storage operation failed",
                matches!(code, ErrorCode::Unavailable | ErrorCode::Overload),
            )
        }
        _ => invalid(&bounded(&error.to_string())),
    }
}
