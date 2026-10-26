//! Accepted connection reads share ordinary client authentication and bounded safe retries.

use crate::{ClientError, ControlClient, path_segment};
use milkdrift_control_protocol::topology::WorkflowTopologyDto;

impl ControlClient {
    /// Reads one generation-bound page of accepted task ports and connections.
    ///
    /// # Errors
    /// Refuses changed generation/authority cursors, missing workflows and transport failures.
    pub async fn workflow_topology(
        &self,
        workflow: &str,
        cursor: Option<&str>,
    ) -> Result<WorkflowTopologyDto, ClientError> {
        let path = format!("v1/workflows/{}/topology", path_segment(workflow)?);
        let mut url = self.url(&path)?;
        if let Some(cursor) = cursor {
            url.query_pairs_mut().append_pair("cursor", cursor);
        }
        let path = match url.query() {
            Some(query) => format!("{path}?{query}"),
            None => path,
        };
        self.safe_get(&path).await
    }
}
