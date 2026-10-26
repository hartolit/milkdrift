//! Conditional saves are never automatically replayed; Run retains exact receipt recovery.
use crate::{ClientError, ControlClient, path_segment};
use milkdrift_control_protocol::authoring::{
    AuthoringReviewDto, AuthoringSnapshotDto, CheckpointAuthoringRequest, PositionsSnapshotDto,
    PutPositionsRequest, ReviewAuthoringRequest, RunAuthoringRequest,
};
use milkdrift_control_protocol::tasks::WorkflowReceipt;
use reqwest::Method;

impl ControlClient {
    /// Reads the latest sketch to resume work or reconcile a lost checkpoint reply.
    /// # Errors
    /// Returns current source/scope denial, absence, corrupt protocol or transport failure.
    pub async fn workflow_authoring(
        &self,
        workflow: &str,
    ) -> Result<AuthoringSnapshotDto, ClientError> {
        self.safe_get(&format!(
            "v1/workflows/{}/authoring",
            path_segment(workflow)?
        ))
        .await
    }

    /// Replaces only the expected current sketch. A lost reply requires reading current content.
    /// # Errors
    /// Returns stale generation, authority, capacity, protocol or transport failure.
    pub async fn checkpoint_workflow_authoring(
        &self,
        workflow: &str,
        request: &CheckpointAuthoringRequest,
    ) -> Result<AuthoringSnapshotDto, ClientError> {
        self.json_request(
            Method::PUT,
            &format!("v1/workflows/{}/authoring", path_segment(workflow)?),
            Some(request),
            false,
        )
        .await
    }

    /// Reads separately versioned card positions without opening full task/result content.
    /// # Errors
    /// Returns scope, absence, protocol or transport failure.
    pub async fn workflow_positions(
        &self,
        workflow: &str,
    ) -> Result<PositionsSnapshotDto, ClientError> {
        self.safe_get(&format!(
            "v1/workflows/{}/positions",
            path_segment(workflow)?
        ))
        .await
    }

    /// Replaces a reviewed layout without invalidating semantic authoring or Run.
    /// # Errors
    /// Returns stale layout, invalid positions, authority, capacity or transport failure.
    pub async fn checkpoint_workflow_positions(
        &self,
        workflow: &str,
        request: &PutPositionsRequest,
    ) -> Result<PositionsSnapshotDto, ClientError> {
        self.json_request(
            Method::PUT,
            &format!("v1/workflows/{}/positions", path_segment(workflow)?),
            Some(request),
            false,
        )
        .await
    }

    /// Shows required new predecessors and local repairs. The returned scope executes nothing.
    /// # Errors
    /// Returns stale saved content, scope/source denial, protocol or transport failure.
    pub async fn review_workflow_authoring(
        &self,
        workflow: &str,
        request: &ReviewAuthoringRequest,
    ) -> Result<AuthoringReviewDto, ClientError> {
        self.json_request(
            Method::POST,
            &format!("v1/workflows/{}/authoring/review", path_segment(workflow)?),
            Some(request),
            false,
        )
        .await
    }

    /// Admits exactly the reviewed saved scope. Recover a lost response through workflow_receipt.
    /// # Errors
    /// Returns local repairs, changed decisions, current denial, capacity or transport failure.
    pub async fn run_workflow_authoring(
        &self,
        workflow: &str,
        request: &RunAuthoringRequest,
    ) -> Result<WorkflowReceipt, ClientError> {
        self.json_request(
            Method::POST,
            &format!("v1/workflows/{}/authoring/run", path_segment(workflow)?),
            Some(request),
            false,
        )
        .await
    }
}
