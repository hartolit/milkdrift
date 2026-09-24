//! Publication commands use ordinary authentication, receipts and capability authority.
use super::super::{
    Owner, PublicFailure, internal, invalid, not_found, public_control, public_persistence,
};
use crate::auth::ActorSession;
use milkdrift_authority::{AuthorityOperation, RequestedResourceFacts};
use milkdrift_capability::{CapabilityId, OperationId};
use milkdrift_control_protocol::{Command, CommandAccepted, CommandRequest};
use milkdrift_persistence::{
    PageSize,
    published::{PublishedMethod, PublishedMethodStore},
};

pub(super) fn execute(
    owner: &Owner,
    session: &ActorSession,
    request: &CommandRequest,
) -> Result<CommandAccepted, PublicFailure> {
    let publications =
        owner.workflow()?.publications.as_ref().ok_or_else(|| {
            invalid("publication services are unavailable during recovery controls")
        })?;
    let (capability, operation) = match &request.command {
        Command::PublishMethod { document, .. } => {
            let method: PublishedMethod = serde_json::from_value(document.clone())
                .map_err(|error| invalid(&error.to_string()))?;
            (Some(method.descriptor.identity().clone()), "method.publish")
        }
        Command::RetireMethod { capability, .. } => (
            Some(CapabilityId::new(capability).map_err(|error| invalid(&error.to_string()))?),
            "method.retire",
        ),
        Command::InspectMethod { capability, .. } => (
            Some(CapabilityId::new(capability).map_err(|error| invalid(&error.to_string()))?),
            "method.inspect",
        ),
        Command::ListMethods { .. } => (None, "method.inspect"),
        _ => return Err(internal()),
    };
    let mut resources = RequestedResourceFacts::empty();
    resources.capability = capability.clone();
    resources.capability_operation =
        Some(OperationId::new(operation).map_err(|error| invalid(&error.to_string()))?);
    let fingerprint = super::super::receipts::command_fingerprint(session, request)?;
    let decision = owner.authorize(
        session,
        AuthorityOperation::AdministerCapabilities,
        resources,
        operation,
    )?;
    owner.record_security_decision(&decision)?;
    let value = match &request.command {
        Command::PublishMethod {
            document,
            expected_previous_version,
        } => {
            let method = serde_json::from_value(document.clone())
                .map_err(|error| invalid(&error.to_string()))?;
            serde_json::to_value(
                publications
                    .publish(method, *expected_previous_version, &decision, &fingerprint)
                    .map_err(public_control)?,
            )
            .map_err(|_| internal())?
        }
        Command::RetireMethod {
            generation,
            expected_version,
            ..
        } => serde_json::to_value(
            publications
                .retire(
                    capability.as_ref().ok_or_else(internal)?,
                    *generation,
                    *expected_version,
                    &decision,
                    &fingerprint,
                )
                .map_err(public_control)?,
        )
        .map_err(|_| internal())?,
        Command::InspectMethod { generation, .. } => serde_json::to_value(
            owner
                .store
                .published_method(capability.as_ref().ok_or_else(internal)?, *generation)
                .map_err(public_persistence)?
                .ok_or_else(not_found)?,
        )
        .map_err(|_| internal())?,
        Command::ListMethods {
            after_capability,
            after_generation,
            limit,
        } => {
            if after_capability.is_some() != after_generation.is_some()
                || *limit == 0
                || *limit > 128
            {
                return Err(invalid(
                    "publication page requires a complete after identity and limit 1..=128",
                ));
            }
            let after = after_capability
                .as_ref()
                .map(CapabilityId::new)
                .transpose()
                .map_err(|error| invalid(&error.to_string()))?;
            let page = owner
                .store
                .published_methods(
                    after.as_ref().zip(*after_generation),
                    PageSize::new(*limit).map_err(public_persistence)?,
                )
                .map_err(public_persistence)?;
            let mut visible = Vec::new();
            for record in page {
                let mut resources = RequestedResourceFacts::empty();
                resources.capability = Some(record.method.descriptor.identity().clone());
                resources.capability_operation = Some(
                    OperationId::new("method.inspect")
                        .map_err(|error| invalid(&error.to_string()))?,
                );
                owner.authorize(
                    session,
                    AuthorityOperation::AdministerCapabilities,
                    resources,
                    "method.inspect",
                )?;
                visible.push(record);
            }
            let after = visible.last().map(|record| serde_json::json!({"capability": record.method.descriptor.identity(), "generation": record.method.descriptor.descriptor_revision()}));
            serde_json::json!({"methods": visible, "after": after})
        }
        _ => return Err(internal()),
    };
    Ok(CommandAccepted {
        command_id: request.command_id.clone(),
        replayed: false,
        resulting_sequence: None,
        result_type: operation.to_owned(),
        value,
    })
}
