//! Thin project clients preserve exact requests across lost replies.
use super::{ClientError, ControlClient, encoded_path_segment};
use milkdrift_control_protocol::projects::{
    ProjectCommandRequest, ProjectDirectoryPageDto, ProjectDto, ProjectEnvironmentPageDto,
    ProjectPageDto, ProjectReceiptDto,
};
use reqwest::Method;
impl ControlClient {
    /// Discover a bounded authorized project page.
    /// # Errors
    /// Returns authority, cursor, bounds, transport or protocol failures.
    pub async fn projects(
        &self,
        cursor: Option<&str>,
        limit: u32,
    ) -> Result<ProjectPageDto, ClientError> {
        self.project_page_request("v1/projects", None, cursor, limit)
            .await
    }
    /// Read one authorized project and its current folder availability.
    /// # Errors
    /// Returns missing/restricted identity, transport or protocol failures.
    pub async fn project(&self, id: &str) -> Result<ProjectDto, ClientError> {
        self.safe_get(&format!("v1/projects/{}", encoded_path_segment(id)?))
            .await
    }
    /// Browse permitted roots or one live directory on the daemon.
    /// # Errors
    /// Returns authority, cursor, bounds, transport or protocol failures.
    pub async fn project_directories(
        &self,
        parent: Option<&str>,
        cursor: Option<&str>,
        limit: u32,
    ) -> Result<ProjectDirectoryPageDto, ClientError> {
        self.project_page_request("v1/project-directories", parent, cursor, limit)
            .await
    }
    /// Discover configured compatible tool environments without secrets or arguments.
    /// # Errors
    /// Returns authority, transport or protocol failures.
    pub async fn project_environments(&self) -> Result<ProjectEnvironmentPageDto, ClientError> {
        self.safe_get("v1/project-environments").await
    }
    /// Submit exactly once; recover the original key after a lost reply.
    /// # Errors
    /// Returns validation, authority, conflict, capacity or transport failures.
    pub async fn project_command(
        &self,
        request: &ProjectCommandRequest,
    ) -> Result<ProjectReceiptDto, ClientError> {
        self.json_request(Method::POST, "v1/project-commands", Some(request), false)
            .await
    }
    /// Recover this actor's exact accepted result or unresolved creation.
    /// # Errors
    /// Returns missing/restricted receipt, transport or protocol failures.
    pub async fn project_receipt(&self, key: &str) -> Result<ProjectReceiptDto, ClientError> {
        self.safe_get(&format!(
            "v1/project-receipts/{}",
            encoded_path_segment(key)?
        ))
        .await
    }
    /// List this actor's bounded unresolved creations for explicit recovery.
    /// # Errors
    /// Returns authority, transport or protocol failures.
    pub async fn project_operations(&self) -> Result<Vec<ProjectReceiptDto>, ClientError> {
        self.safe_get("v1/project-operations").await
    }
    async fn project_page_request<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        parent: Option<&str>,
        cursor: Option<&str>,
        limit: u32,
    ) -> Result<T, ClientError> {
        if !(1..=200).contains(&limit) {
            return Err(ClientError::Configuration(
                "project pages require 1..=200 items".into(),
            ));
        }
        let mut url = self.url(path)?;
        url.query_pairs_mut()
            .append_pair("limit", &limit.to_string());
        if let Some(parent) = parent {
            url.query_pairs_mut().append_pair("parent", parent);
        }
        if let Some(cursor) = cursor {
            url.query_pairs_mut().append_pair("cursor", cursor);
        }
        self.safe_get(&format!("{path}?{}", url.query().unwrap_or_default()))
            .await
    }
}
