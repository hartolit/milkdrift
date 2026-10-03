//! Bind one exact start request to its authenticated installation and caller before submission.
use crate::{ClientError, ControlClient};
use milkdrift_control_protocol::{AuthorityRead, Command, CommandAccepted, CommandRequest};
use serde::{Deserialize, Serialize};

/// Portable recovery record for one ordinary run start, with no credentials or local input paths.
///
/// Obtain this with [`ControlClient::prepare_run`], persist its complete bounded JSON privately,
/// and only then call [`ControlClient::submit_saved_run`]. A timeout leaves the outcome unknown;
/// resubmit this same record to recover the original result. Deliberately new work needs a new
/// request and run identity. The record preserves artifact references and guards, while runtime
/// dispatch retains ownership of capability selection and execution history.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SavedRunRequest {
    /// Recovery document version; currently 1.
    pub schema_version: u32,
    /// Authenticated installation, actor, and exact grant observed during preparation.
    pub authority: AuthorityRead,
    /// Exact original command, including run/revision/input identities and optimistic guards.
    pub request: CommandRequest,
}

impl SavedRunRequest {
    /// Refuses unsupported records and operations before any replay can cause work.
    pub fn validate(&self) -> Result<(), ClientError> {
        if self.schema_version != 1
            || self.authority.host.is_empty()
            || self.authority.actor.is_empty()
        {
            return Err(ClientError::Configuration(
                "unsupported or incomplete saved run request".into(),
            ));
        }
        self.request.validate()?;
        let Command::StartRun {
            run_id,
            workflow_id,
            revision_id,
            ..
        } = &self.request.command
        else {
            return Err(ClientError::Configuration(
                "saved run request must contain start_run".into(),
            ));
        };
        if run_id.is_empty()
            || workflow_id.is_empty()
            || revision_id.is_empty()
            || self
                .request
                .expected_sequence
                .is_some_and(|sequence| sequence != 0)
            || self
                .request
                .expected_revision
                .as_ref()
                .is_some_and(|guard| guard != revision_id)
        {
            return Err(ClientError::Configuration(
                "saved run identity or guards are invalid".into(),
            ));
        }
        milkdrift_control_protocol::encode_json(self)?;
        Ok(())
    }
}

impl ControlClient {
    /// Captures authenticated recovery identity without submitting or pinning a model selection.
    pub async fn prepare_run(
        &self,
        request: CommandRequest,
    ) -> Result<SavedRunRequest, ClientError> {
        let saved = SavedRunRequest {
            schema_version: 1,
            authority: self.authority().await?,
            request,
        };
        saved.validate()?;
        Ok(saved)
    }

    /// Sends the exact saved start once, after checking the current host, caller, and grant.
    ///
    /// The server's canonical command receipt owns replay/conflict behavior. This check prevents
    /// inadvertently using a recovery file with another endpoint or credential. It does not
    /// relax current authority, renew execution allowances, or select a replacement capability.
    pub async fn submit_saved_run(
        &self,
        saved: &SavedRunRequest,
    ) -> Result<CommandAccepted, ClientError> {
        saved.validate()?;
        if self.authority().await? != saved.authority {
            return Err(ClientError::Configuration(
                "saved run belongs to another host, caller, or grant; use its original authority"
                    .into(),
            ));
        }
        self.submit(&saved.request).await
    }
}
