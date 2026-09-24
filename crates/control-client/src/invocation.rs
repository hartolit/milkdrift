//! Typed independent execution requests over the same authenticated control connection.
use super::{ClientError, ControlClient};
use milkdrift_peer_protocol::{
    DirectDiscovery, DirectInvocationRequest, InvocationAcceptance, InvocationLookup,
    ObservationPage, PeerCancellationAcknowledgement, PeerCancellationRequest, PeerExecutionId,
    PeerRequestId, ServingInvocationRead,
};
use reqwest::Method;

impl ControlClient {
    /// Execute a typed lifecycle request once. Preserve its exact command document for replay;
    /// an acceptance receipt and the later inspected installation state have separate meanings.
    pub async fn manage_resources(
        &self,
        request: &milkdrift_capability::managed::ManagedRequest,
    ) -> Result<milkdrift_capability::managed::ManagedResponse, ClientError> {
        request.validate().map_err(protocol)?;
        let response: milkdrift_capability::managed::ManagedResponse = self
            .json_request(Method::POST, "v1/resources", Some(request), false)
            .await?;
        response.validate_for(request).map_err(protocol)?;
        Ok(response)
    }
    /// Publishes one bounded input atomically. Replay the exact request after a lost reply.
    pub async fn upload_input(
        &self,
        request: &milkdrift_control_protocol::InputUploadRequest,
    ) -> Result<milkdrift_control_protocol::ArtifactMetadataRead, ClientError> {
        self.json_request(Method::POST, "v1/artifact-inputs", Some(request), false)
            .await
    }
    /// Discovers the exact host identity, ceilings, and authorized capability generations.
    pub async fn execution_discovery(&self) -> Result<DirectDiscovery, ClientError> {
        let discovery: DirectDiscovery = self.safe_get("v1/execution/catalog").await?;
        discovery.catalog.validate().map_err(protocol)?;
        discovery.limits.validate().map_err(protocol)?;
        Ok(discovery)
    }

    /// Submits once. A lost response requires lookup or exact replay of this saved document.
    pub async fn invoke(
        &self,
        request: &DirectInvocationRequest,
    ) -> Result<InvocationAcceptance, ClientError> {
        let acceptance: InvocationAcceptance = self
            .json_request(Method::POST, "v1/invocations", Some(request), false)
            .await?;
        let response_id = match &acceptance {
            InvocationAcceptance::Accepted { request_id, .. }
            | InvocationAcceptance::Archived { request_id, .. }
            | InvocationAcceptance::Rejected { request_id, .. } => request_id,
        };
        if response_id != &request.request_id {
            return Err(protocol("acceptance names another request"));
        }
        if let InvocationAcceptance::Archived {
            execution, summary, ..
        } = &acceptance
        {
            summary.validate(execution).map_err(protocol)?;
        }
        Ok(acceptance)
    }

    /// Recovers acceptance in this authenticated actor's request namespace.
    pub async fn invocation_lookup(
        &self,
        request: &PeerRequestId,
    ) -> Result<InvocationLookup, ClientError> {
        let lookup: InvocationLookup = self
            .safe_get(&format!(
                "v1/invocation-requests/{}",
                super::path_segment(request.as_str())?
            ))
            .await?;
        lookup.validate_for(request).map_err(protocol)?;
        Ok(lookup)
    }

    /// Inspects an accepted invocation after current permission checks.
    pub async fn invocation(
        &self,
        execution: &PeerExecutionId,
    ) -> Result<ServingInvocationRead, ClientError> {
        let read: ServingInvocationRead = self
            .safe_get(&format!(
                "v1/invocations/{}",
                super::path_segment(execution.as_str())?
            ))
            .await?;
        match &read.acceptance {
            InvocationLookup::Known {
                execution: actual,
                request_id,
                ..
            } if actual == execution => {
                read.acceptance.validate_for(request_id).map_err(protocol)?;
            }
            _ => return Err(protocol("inspection names another accepted execution")),
        }
        Ok(read)
    }

    /// Fetches one bounded observation page; callers own the overall wait deadline.
    pub async fn invocation_observations(
        &self,
        execution: &PeerExecutionId,
        after: u64,
        limit: u32,
    ) -> Result<ObservationPage, ClientError> {
        let page: ObservationPage = self
            .safe_get(&format!(
                "v1/invocations/{}/observations?after={after}&limit={limit}",
                super::path_segment(execution.as_str())?
            ))
            .await?;
        page.validate(limit as usize).map_err(protocol)?;
        if page.execution != *execution || page.after_sequence != after {
            return Err(protocol(
                "observation page names another execution or cursor",
            ));
        }
        Ok(page)
    }

    /// Read bytes only from the retained terminal outputs of this caller's accepted capability.
    pub async fn invocation_output(
        &self,
        execution: &PeerExecutionId,
        artifact: &str,
        offset: u64,
        maximum: u32,
    ) -> Result<milkdrift_peer_protocol::InvocationOutputChunk, ClientError> {
        let chunk: milkdrift_peer_protocol::InvocationOutputChunk = self
            .safe_get(&format!(
                "v1/invocations/{}/outputs/{}?offset={offset}&maximum={maximum}",
                super::path_segment(execution.as_str())?,
                super::path_segment(artifact)?
            ))
            .await?;
        if chunk.execution != *execution
            || chunk.metadata.reference().artifact().as_str() != artifact
            || chunk.offset != offset
            || chunk.bytes.len() > maximum as usize
            || offset
                .checked_add(chunk.bytes.len() as u64)
                .is_none_or(|end| {
                    end > chunk.metadata.reference().size_bytes()
                        || chunk.complete != (end == chunk.metadata.reference().size_bytes())
                })
        {
            return Err(protocol(
                "output chunk differs from its requested execution, artifact or range",
            ));
        }
        Ok(chunk)
    }

    /// Requests cancellation once. The acknowledgement does not prove terminal completion.
    pub async fn cancel_invocation(
        &self,
        request: &PeerCancellationRequest,
    ) -> Result<PeerCancellationAcknowledgement, ClientError> {
        let acknowledgement: PeerCancellationAcknowledgement = self
            .json_request(
                Method::POST,
                &format!(
                    "v1/invocations/{}/cancel",
                    super::path_segment(request.execution.as_str())?
                ),
                Some(request),
                false,
            )
            .await?;
        acknowledgement.validate_for(request).map_err(protocol)?;
        Ok(acknowledgement)
    }
}

fn protocol(error: impl std::fmt::Display) -> ClientError {
    milkdrift_control_protocol::ProtocolError::InvalidContract(error.to_string()).into()
}

#[cfg(test)]
mod tests;
