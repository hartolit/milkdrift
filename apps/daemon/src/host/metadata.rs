//! Shared installation names are durable daemon facts, independent of workflow ownership.
use super::{DaemonHost, Owner, PublicFailure, read_model};
use crate::auth::ActorSession;
use milkdrift_authority::{AuthorityOperation, RequestedResourceFacts};
use milkdrift_control_protocol::{
    DaemonMetadataDto, ErrorCode, RenameDaemonReceipt, RenameDaemonRequest,
};
use milkdrift_persistence::{
    DaemonDisplayName, DaemonMetadataStore, DaemonRename, DaemonRenameOutcome,
};

pub(crate) enum DaemonMetadataRequest {
    Read,
    Rename {
        request: RenameDaemonRequest,
        canonical_request: Vec<u8>,
    },
    Receipt(String),
}

impl DaemonHost {
    pub(crate) async fn daemon_metadata_request(
        &self,
        session: ActorSession,
        expected_host: Option<String>,
        request: DaemonMetadataRequest,
    ) -> Result<serde_json::Value, PublicFailure> {
        if matches!(&request, DaemonMetadataRequest::Rename { .. }) && !self.accepting_mutations() {
            return Err(PublicFailure::new(
                ErrorCode::Unavailable,
                "daemon is draining",
                true,
            ));
        }
        self.dispatch(false, move |owner| {
            owner.check_daemon_identity(expected_host.as_deref())?;
            match request {
                DaemonMetadataRequest::Read => {
                    serde_json::to_value(owner.daemon_metadata(&session)?)
                }
                DaemonMetadataRequest::Rename {
                    request,
                    canonical_request,
                } => serde_json::to_value(owner.rename_daemon(
                    &session,
                    request,
                    canonical_request,
                )?),
                DaemonMetadataRequest::Receipt(key) => {
                    serde_json::to_value(owner.daemon_rename_receipt(&session, &key)?)
                }
            }
            .map_err(|_| read_model::internal())
        })
        .await
    }
}

fn configuration_resource() -> RequestedResourceFacts {
    let mut resources = RequestedResourceFacts::empty();
    resources.daemon_configuration = true;
    resources
}

impl Owner {
    pub(super) fn check_daemon_identity(
        &self,
        expected: Option<&str>,
    ) -> Result<(), PublicFailure> {
        if expected.is_some_and(|expected| expected != self.host_id.as_str()) {
            return Err(PublicFailure::new(
                ErrorCode::DaemonIdentityMismatch,
                "This address now serves a different daemon. Forget the saved connection and connect again to use it.",
                false,
            ));
        }
        Ok(())
    }

    pub(super) fn daemon_metadata(
        &self,
        session: &ActorSession,
    ) -> Result<DaemonMetadataDto, PublicFailure> {
        self.authorize(
            session,
            AuthorityOperation::NegotiateControlProtocol,
            RequestedResourceFacts::empty(),
            "daemon:metadata",
        )?;
        let metadata = self
            .store
            .daemon_metadata(&self.host_id)
            .map_err(read_model::public_persistence)?
            .ok_or_else(read_model::internal)?;
        let can_rename = self
            .evaluate_authority(
                session,
                AuthorityOperation::RenameDaemon,
                configuration_resource(),
                "daemon:rename-available",
            )?
            .is_allowed();
        Ok(DaemonMetadataDto {
            daemon_id: metadata.installation.to_string(),
            name: metadata.display_name.as_str().to_owned(),
            generation: metadata.generation,
            can_rename,
        })
    }

    fn rename_daemon(
        &self,
        session: &ActorSession,
        request: RenameDaemonRequest,
        canonical_request: Vec<u8>,
    ) -> Result<RenameDaemonReceipt, PublicFailure> {
        self.authorize(
            session,
            AuthorityOperation::RenameDaemon,
            configuration_resource(),
            "daemon:rename",
        )?;
        let outcome = self.store.rename_daemon(DaemonRename {
            installation: self.host_id.clone(), actor: session.actor.clone(), key: request.request_id,
            expected_generation: request.expected_generation,
            display_name: DaemonDisplayName::new(request.name).map_err(read_model::public_persistence)?,
            canonical_request,
        }).map_err(|error| match error {
            milkdrift_persistence::PersistenceError::Storage {
                class: milkdrift_persistence::StorageFailureClass::ResourceExhausted, ..
            } => PublicFailure::new(ErrorCode::Overload,
                "This daemon has retained the maximum number of rename receipts. Existing names and receipts remain available.", false),
            other => read_model::public_persistence(other),
        })?;
        match outcome {
            DaemonRenameOutcome::Applied(receipt) | DaemonRenameOutcome::Replay(receipt) => {
                Ok(public_receipt(receipt))
            }
        }
    }

    fn daemon_rename_receipt(
        &self,
        session: &ActorSession,
        key: &str,
    ) -> Result<RenameDaemonReceipt, PublicFailure> {
        self.authorize(
            session,
            AuthorityOperation::NegotiateControlProtocol,
            RequestedResourceFacts::empty(),
            "daemon:rename-receipt",
        )?;
        self.store
            .daemon_rename_receipt(&self.host_id, &session.actor, key)
            .map_err(read_model::public_persistence)?
            .map(public_receipt)
            .ok_or_else(read_model::not_found)
    }
}

fn public_receipt(receipt: milkdrift_persistence::DaemonRenameReceipt) -> RenameDaemonReceipt {
    RenameDaemonReceipt {
        request_id: receipt.key,
        daemon_id: receipt.result.installation.to_string(),
        name: receipt.result.display_name.as_str().to_owned(),
        generation: receipt.result.generation,
    }
}
