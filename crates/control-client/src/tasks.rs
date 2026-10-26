//! Thin ordinary task operations. Lost mutation replies require receipt lookup or exact replay.
mod authoring;
mod topology;
use super::{ClientError, ControlClient, path_segment};
use milkdrift_control_protocol::tasks::{
    ChangeWorkflowRequest, CreateWorkflowRequest, TaskReadDto, TaskResultDto, TaskSessionDto,
    TaskUploadReceipt, TaskUploadRequest, WorkflowPageDto, WorkflowReadDto, WorkflowReceipt,
};
use reqwest::Method;

impl ControlClient {
    /// Discovers this authenticated connection's daemon, actor and available task choices.
    /// # Errors
    /// Returns transport, protocol or authorization failures.
    pub async fn task_session(&self) -> Result<TaskSessionDto, ClientError> {
        self.safe_get("v1/session").await
    }

    /// Submits an exact request once. Discover or replay it if the response is lost.
    /// # Errors
    /// Returns validation, authority, capacity, conflict or transport failures.
    pub async fn create_workflow(
        &self,
        request: &CreateWorkflowRequest,
    ) -> Result<WorkflowReceipt, ClientError> {
        self.json_request(Method::POST, "v1/workflows", Some(request), false)
            .await
    }

    /// Lists across all permitted workflow scopes without a known workflow identity.
    /// # Errors
    /// Returns invalid bounds, expired cursor, authority or transport failures.
    pub async fn workflows(
        &self,
        search: &str,
        cursor: Option<&str>,
        limit: u32,
    ) -> Result<WorkflowPageDto, ClientError> {
        self.workflows_in_project(search, cursor, limit, None, false)
            .await
    }

    /// Lists one project's workflows or earlier unassociated workflows with an actor-bound cursor.
    /// # Errors
    /// Returns invalid filters/bounds, expired cursor, authority or transport failures.
    pub async fn workflows_in_project(
        &self,
        search: &str,
        cursor: Option<&str>,
        limit: u32,
        project: Option<&str>,
        earlier: bool,
    ) -> Result<WorkflowPageDto, ClientError> {
        if earlier && project.is_some() {
            return Err(ClientError::Configuration(
                "choose a project or earlier workflows".into(),
            ));
        }
        if !(1..=200).contains(&limit) || search.len() > 256 {
            return Err(ClientError::Configuration(
                "workflow pages require 1..=200 items and search at most 256 bytes".into(),
            ));
        }
        let mut url = self.url("v1/workflows")?;
        url.query_pairs_mut()
            .append_pair("search", search)
            .append_pair("limit", &limit.to_string());
        if let Some(cursor) = cursor {
            url.query_pairs_mut().append_pair("cursor", cursor);
        }
        if let Some(project) = project {
            url.query_pairs_mut().append_pair("project_id", project);
        }
        if earlier {
            url.query_pairs_mut().append_pair("earlier", "true");
        }
        let path = format!("v1/workflows?{}", url.query().unwrap_or_default());
        self.safe_get(&path).await
    }

    /// Reads a workflow and its tasks from the owning daemon.
    /// # Errors
    /// Returns invalid identity, authorization, missing content or transport failure.
    pub async fn workflow(&self, workflow: &str) -> Result<WorkflowReadDto, ClientError> {
        self.workflow_page(workflow, None).await
    }

    /// Reads one live page of compact task summaries. Full task content has its own route.
    /// # Errors
    /// Returns missing workflow, invalid cursor, authorization or transport failure.
    pub async fn workflow_page(
        &self,
        workflow: &str,
        cursor: Option<&str>,
    ) -> Result<WorkflowReadDto, ClientError> {
        let path = format!("v1/workflows/{}", path_segment(workflow)?);
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

    /// Applies one generation-guarded batch without automatic semantic retries.
    /// # Errors
    /// Returns admission, generation/start conflicts or transport failure.
    pub async fn change_workflow(
        &self,
        workflow: &str,
        request: &ChangeWorkflowRequest,
    ) -> Result<WorkflowReceipt, ClientError> {
        self.json_request(
            Method::POST,
            &format!("v1/workflows/{}/changes", path_segment(workflow)?),
            Some(request),
            false,
        )
        .await
    }

    /// Finds the durable result of a request whose reply may have been lost.
    /// # Errors
    /// Returns absent request, authorization or transport failure.
    pub async fn workflow_receipt(&self, request: &str) -> Result<WorkflowReceipt, ClientError> {
        self.safe_get(&format!("v1/workflow-requests/{}", path_segment(request)?))
            .await
    }

    /// Reads the actual task state including frozen start times and uncertainty.
    /// # Errors
    /// Returns missing task, authorization or transport failure.
    pub async fn task(&self, workflow: &str, task: &str) -> Result<TaskReadDto, ClientError> {
        self.safe_get(&format!(
            "v1/workflows/{}/tasks/{}",
            path_segment(workflow)?,
            path_segment(task)?
        ))
        .await
    }

    /// Reads retained immutable output; queued work never yields fabricated content.
    /// # Errors
    /// Returns unavailable/expired result, authorization or transport failure.
    pub async fn task_result(
        &self,
        workflow: &str,
        task: &str,
    ) -> Result<TaskResultDto, ClientError> {
        self.safe_get(&format!(
            "v1/workflows/{}/tasks/{}/result",
            path_segment(workflow)?,
            path_segment(task)?
        ))
        .await
    }

    /// Captures UTF-8 file bytes for later named task input bindings.
    /// # Errors
    /// Returns invalid content, capacity, replay conflict, authority or transport failure.
    pub async fn upload_task_input(
        &self,
        request: &TaskUploadRequest,
    ) -> Result<TaskUploadReceipt, ClientError> {
        self.json_request(Method::POST, "v1/task-inputs", Some(request), false)
            .await
    }
}
