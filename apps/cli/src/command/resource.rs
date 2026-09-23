use crate::{ResourceArgs, ResourceCommand, error::CliError, session::CliSession};
use milkdrift_capability::managed::{
    DataDisposition, MANAGED_SCHEMA_VERSION, ManagedAction, ManagedName, ManagedRequest,
    RecipeReference,
};

impl ResourceArgs {
    pub(super) fn request(&self, command_id: &str) -> Result<ManagedRequest, CliError> {
        let name =
            |value: &str| ManagedName::new(value).map_err(|e| CliError::Invalid(e.to_string()));
        let reference = |recipe: &str, digest: &str| -> Result<RecipeReference, CliError> {
            Ok(RecipeReference {
                name: name(recipe)?,
                digest: digest.to_owned(),
            })
        };
        let action = match &self.command {
            ResourceCommand::Evaluate {
                artifact,
                digest,
                media_type,
                size_bytes,
            } => ManagedAction::Evaluate {
                candidate: milkdrift_capability::ArtifactReference::new(
                    artifact.clone(),
                    digest.clone(),
                    Some(media_type.clone()),
                    Some(*size_bytes),
                )
                .map_err(|e| CliError::Invalid(e.to_string()))?,
            },
            ResourceCommand::Evidence { evaluation } => ManagedAction::Evidence {
                evaluation: evaluation.clone(),
            },
            ResourceCommand::Publish { evaluation } => ManagedAction::Publish {
                evaluation: evaluation.clone(),
            },
            ResourceCommand::Handoff(transfer) => ManagedAction::Handoff {
                transfer: transfer.document(name(&self.installation)?),
            },
            ResourceCommand::Return {
                transfer,
                resume_parent,
            } => ManagedAction::Return {
                transfer: transfer.document(name(&self.installation)?),
                resume_parent: *resume_parent,
            },
            ResourceCommand::Prepare { recipe, digest } => ManagedAction::Prepare {
                recipe: reference(recipe, digest)?,
            },
            ResourceCommand::Apply { recipe, digest } => ManagedAction::Apply {
                recipe: reference(recipe, digest)?,
            },
            ResourceCommand::Inspect => ManagedAction::Inspect {},
            ResourceCommand::Start => ManagedAction::Start {},
            ResourceCommand::Stop => ManagedAction::Stop {},
            ResourceCommand::Update {
                recipe,
                digest,
                allow_interruption,
            } => ManagedAction::Update {
                recipe: reference(recipe, digest)?,
                allow_interruption: *allow_interruption,
            },
            ResourceCommand::Preserve { delete_on_removal } => ManagedAction::Preserve {
                disposition: if *delete_on_removal {
                    DataDisposition::DeleteOnRemoval
                } else {
                    DataDisposition::Preserve
                },
            },
            ResourceCommand::Remove => ManagedAction::Remove {},
            ResourceCommand::Recover => ManagedAction::Recover {},
            ResourceCommand::Resolve {
                use_id,
                expected_claim,
            } => ManagedAction::Resolve {
                use_id: use_id.clone(),
                expected_claim: *expected_claim,
            },
        };
        let request = ManagedRequest {
            schema_version: MANAGED_SCHEMA_VERSION,
            installation: name(&self.installation)?,
            command: name(command_id)?,
            expected_version: self.expected_version,
            action,
        };
        request
            .validate()
            .map_err(|e| CliError::Invalid(e.to_string()))?;
        Ok(request)
    }
}

impl crate::EditingArgs {
    fn document(&self, installation: ManagedName) -> milkdrift_capability::managed::EditingHandoff {
        milkdrift_capability::managed::EditingHandoff {
            installation,
            generation: self.generation,
            parent: self.parent.clone(),
            child: self.child.clone(),
            parent_claim: self.parent_claim,
            child_claim: self.child_claim,
            association: self.association.clone(),
        }
    }
}

pub(super) async fn execute(session: &CliSession, args: &ResourceArgs) -> Result<(), CliError> {
    let request = args.request(session.cli().command_id.as_deref().ok_or_else(|| {
        CliError::Invalid("resource operation requires a stable command id".to_owned())
    })?)?;
    let response = session.client().manage_resources(&request).await?;
    session.output(request.operation(), &response)
}
