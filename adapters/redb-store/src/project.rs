//! Registrations, organizational association and bounded exact creation recovery.
use crate::{
    RedbStore, error, json,
    schema::{PROJECT_OPERATIONS, PROJECTS, TASK_WORKFLOWS, WORKFLOW_PROJECTS},
};
use milkdrift_authority::ActorRef;
use milkdrift_blueprint::WorkflowId;
use milkdrift_persistence::{
    MAX_PROJECT_OPERATION_BYTES, MAX_PROJECT_OPERATIONS, MAX_PROJECTS, NativePathRoot,
    PersistenceError, ProjectMutation, ProjectOperation, ProjectOperationState, ProjectRecord,
    ProjectStore, StorageFailureClass,
};
use redb::{ReadableTable, ReadableTableMetadata};

impl ProjectStore for RedbStore {
    fn project(&self, id: &str) -> Result<Option<ProjectRecord>, PersistenceError> {
        identity(id)?;
        let read = self.database().begin_read().map_err(error::redb)?;
        get_project(&read.open_table(PROJECTS).map_err(error::redb)?, id)
    }
    fn projects(
        &self,
        after: Option<&str>,
        limit: u16,
    ) -> Result<Vec<ProjectRecord>, PersistenceError> {
        if limit == 0 || limit > 200 {
            return Err(bounds("project page requires 1..=200 records"));
        }
        if let Some(after) = after {
            identity(after)?;
        }
        let read = self.database().begin_read().map_err(error::redb)?;
        let table = read.open_table(PROJECTS).map_err(error::redb)?;
        let mut values = Vec::new();
        for row in table.range(after.unwrap_or("")..).map_err(error::redb)? {
            let (key, bytes) = row.map_err(error::redb)?;
            if after.is_some_and(|after| key.value() <= after) {
                continue;
            }
            values.push(decode_project(key.value(), bytes.value())?);
            if values.len() == usize::from(limit) {
                break;
            }
        }
        Ok(values)
    }
    fn project_for_workflow(
        &self,
        workflow: &WorkflowId,
    ) -> Result<Option<ProjectRecord>, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
        let associations = read.open_table(WORKFLOW_PROJECTS).map_err(error::redb)?;
        associations
            .get(workflow.as_str())
            .map_err(error::redb)?
            .map(|id| {
                get_project(&read.open_table(PROJECTS).map_err(error::redb)?, id.value())?
                    .ok_or_else(|| error::corruption("workflow references absent project"))
            })
            .transpose()
    }
    fn project_operation(
        &self,
        actor: &ActorRef,
        key: &str,
    ) -> Result<Option<ProjectOperation>, PersistenceError> {
        let storage_key = receipt_key(actor, key)?;
        let read = self.database().begin_read().map_err(error::redb)?;
        get_operation(
            &read.open_table(PROJECT_OPERATIONS).map_err(error::redb)?,
            &storage_key,
        )
    }
    fn project_operations(
        &self,
        actor: &ActorRef,
    ) -> Result<Vec<ProjectOperation>, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
        let table = read.open_table(PROJECT_OPERATIONS).map_err(error::redb)?;
        let mut result = Vec::new();
        for row in table.iter().map_err(error::redb)? {
            let (key, bytes) = row.map_err(error::redb)?;
            let operation = decode_operation(key.value(), bytes.value())?;
            if &operation.actor == actor
                && matches!(operation.state, ProjectOperationState::Creating { .. })
            {
                result.push(operation);
            }
        }
        Ok(result)
    }
    fn mutate_project(
        &self,
        mut operation: ProjectOperation,
        mutation: ProjectMutation,
    ) -> Result<ProjectOperation, PersistenceError> {
        validate_operation(&operation)?;
        let key = receipt_key(&operation.actor, &operation.key)?;
        let write = self.database().begin_write().map_err(error::redb)?;
        let mut operations = write.open_table(PROJECT_OPERATIONS).map_err(error::redb)?;
        if let Some(existing) = get_operation(&operations, &key)? {
            if existing.canonical_request != operation.canonical_request {
                return Err(conflict(&operation));
            }
            return Ok(existing);
        }
        let reserved = pending_creations(&operations)?;
        let required = match &mutation {
            ProjectMutation::BeginCreate => 2,
            ProjectMutation::Reconcile { .. } => 0,
            _ => 1,
        };
        if operations
            .len()
            .map_err(error::redb)?
            .checked_add(reserved)
            .and_then(|total| total.checked_add(required))
            .is_none_or(|total| total > MAX_PROJECT_OPERATIONS)
        {
            return Err(exhausted(
                "project receipt capacity is full; exact replay and creation recovery remain available",
            ));
        }
        let mut projects = write.open_table(PROJECTS).map_err(error::redb)?;
        match mutation {
            ProjectMutation::Attach(project) => {
                reserve_registration(&projects, &operations)?;
                insert_project(&mut projects, &project)?;
                operation.state = accepted(Some(project), None);
            }
            ProjectMutation::Update {
                project,
                expected_generation,
            } => {
                let current = require_project(&projects, &project.id, expected_generation)?;
                if project.folder != current.folder
                    || project.generation
                        != expected_generation
                            .checked_add(1)
                            .ok_or_else(|| bounds("project generation exhausted"))?
                {
                    return Err(bounds(
                        "project identity and folder are immutable; generation must advance once",
                    ));
                }
                validate_project(&project)?;
                let bytes = json::encode(&project, "project")?;
                projects
                    .insert(project.id.as_str(), bytes.as_slice())
                    .map_err(error::redb)?;
                operation.state = accepted(Some(project), None);
            }
            ProjectMutation::Remove {
                id,
                expected_generation,
            } => {
                let project = require_project(&projects, &id, expected_generation)?;
                let links = write.open_table(WORKFLOW_PROJECTS).map_err(error::redb)?;
                for row in links.iter().map_err(error::redb)? {
                    let (_, linked) = row.map_err(error::redb)?;
                    if linked.value() == id {
                        return Err(bounds(
                            "project has retained workflows; registration cannot be removed",
                        ));
                    }
                }
                for row in operations.iter().map_err(error::redb)? {
                    let (key, bytes) = row.map_err(error::redb)?;
                    if let ProjectOperationState::Creating {
                        project: pending, ..
                    } = decode_operation(key.value(), bytes.value())?.state
                        && (pending.id == id
                            || pending.folder.requested == project.folder.requested)
                    {
                        return Err(bounds("unresolved creation protects this registration"));
                    }
                }
                projects.remove(id.as_str()).map_err(error::redb)?;
                operation.state = accepted(Some(project), None);
            }
            ProjectMutation::Associate {
                id,
                expected_generation,
                workflow,
            } => {
                let project = require_project(&projects, &id, expected_generation)?;
                if write
                    .open_table(TASK_WORKFLOWS)
                    .map_err(error::redb)?
                    .get(workflow.as_str())
                    .map_err(error::redb)?
                    .is_none()
                {
                    return Err(PersistenceError::NotFound {
                        entity: "workflow",
                        identity: workflow.to_string(),
                    });
                }
                let mut links = write.open_table(WORKFLOW_PROJECTS).map_err(error::redb)?;
                if let Some(existing) = links.get(workflow.as_str()).map_err(error::redb)?
                    && existing.value() != id
                {
                    return Err(bounds("workflow already belongs to another project"));
                }
                links
                    .insert(workflow.as_str(), id.as_str())
                    .map_err(error::redb)?;
                operation.state = accepted(Some(project), Some(workflow));
            }
            ProjectMutation::BeginCreate => {
                if !matches!(operation.state, ProjectOperationState::Creating { .. }) {
                    return Err(bounds("creation requires durable intent"));
                }
                reserve_registration(&projects, &operations)?;
                if let ProjectOperationState::Creating { project, .. } = &operation.state {
                    for row in operations.iter().map_err(error::redb)? {
                        let (key, bytes) = row.map_err(error::redb)?;
                        if let ProjectOperationState::Creating {
                            project: pending, ..
                        } = decode_operation(key.value(), bytes.value())?.state
                            && pending.folder.requested == project.folder.requested
                        {
                            return Err(bounds(
                                "another unresolved creation already reserves this directory",
                            ));
                        }
                    }
                }
            }
            ProjectMutation::Reconcile {
                original_key,
                folder,
            } => {
                let original_storage_key = receipt_key(&operation.actor, &original_key)?;
                let mut original = get_operation(&operations, &original_storage_key)?
                    .ok_or_else(|| bounds("creation intent not found"))?;
                settle(&mut original, folder, &mut projects)?;
                let bytes = json::encode(&original, "project operation")?;
                operations
                    .insert(original_storage_key.as_str(), bytes.as_slice())
                    .map_err(error::redb)?;
                operation.state = original.state;
            }
        }
        validate_operation(&operation)?;
        let bytes = json::encode(&operation, "project operation")?;
        operations
            .insert(key.as_str(), bytes.as_slice())
            .map_err(error::redb)?;
        drop(operations);
        drop(projects);
        self.faults
            .check(crate::fault::FaultPoint::BeforeProjectCommit)?;
        write.commit().map_err(error::redb)?;
        self.faults
            .check(crate::fault::FaultPoint::AfterProjectCommit)?;
        Ok(operation)
    }
    fn settle_project_creation(
        &self,
        actor: &ActorRef,
        key: &str,
        folder: Option<NativePathRoot>,
    ) -> Result<ProjectOperation, PersistenceError> {
        let key = receipt_key(actor, key)?;
        let write = self.database().begin_write().map_err(error::redb)?;
        let mut operations = write.open_table(PROJECT_OPERATIONS).map_err(error::redb)?;
        let mut operation =
            get_operation(&operations, &key)?.ok_or_else(|| bounds("creation intent not found"))?;
        let mut projects = write.open_table(PROJECTS).map_err(error::redb)?;
        settle(&mut operation, folder, &mut projects)?;
        let bytes = json::encode(&operation, "project operation")?;
        operations
            .insert(key.as_str(), bytes.as_slice())
            .map_err(error::redb)?;
        drop(operations);
        drop(projects);
        self.faults
            .check(crate::fault::FaultPoint::BeforeProjectCommit)?;
        write.commit().map_err(error::redb)?;
        self.faults
            .check(crate::fault::FaultPoint::AfterProjectCommit)?;
        Ok(operation)
    }
}

fn settle(
    operation: &mut ProjectOperation,
    folder: Option<NativePathRoot>,
    projects: &mut redb::Table<'_, &str, &[u8]>,
) -> Result<(), PersistenceError> {
    let ProjectOperationState::Creating {
        project,
        parent,
        folder_name,
    } = &operation.state
    else {
        return Err(bounds(
            "creation is already resolved; replay its original receipt",
        ));
    };
    operation.state = if let Some(folder) = folder {
        if folder.requested != parent.requested.join(folder_name)
            || folder.physical.parent() != Some(parent.physical.as_path())
        {
            return Err(bounds(
                "reconciliation must inspect the original child beneath its pinned parent",
            ));
        }
        let mut project = project.clone();
        project.folder = folder;
        insert_project(projects, &project)?;
        accepted(Some(project), None)
    } else {
        ProjectOperationState::Abandoned
    };
    Ok(())
}
fn reserve_registration(
    projects: &impl ReadableTable<&'static str, &'static [u8]>,
    operations: &impl ReadableTable<&'static str, &'static [u8]>,
) -> Result<(), PersistenceError> {
    let reserved = projects
        .len()
        .map_err(error::redb)?
        .checked_add(pending_creations(operations)?)
        .ok_or_else(|| bounds("project reservation overflow"))?;
    if reserved >= MAX_PROJECTS {
        return Err(exhausted(
            "project capacity is full; unresolved creations retain their registration reservations",
        ));
    }
    Ok(())
}
fn pending_creations(
    operations: &impl ReadableTable<&'static str, &'static [u8]>,
) -> Result<u64, PersistenceError> {
    let mut reserved = 0_u64;
    for row in operations.iter().map_err(error::redb)? {
        let (key, bytes) = row.map_err(error::redb)?;
        if matches!(
            decode_operation(key.value(), bytes.value())?.state,
            ProjectOperationState::Creating { .. }
        ) {
            reserved = reserved
                .checked_add(1)
                .ok_or_else(|| bounds("project reservation overflow"))?;
        }
    }
    Ok(reserved)
}
fn accepted(project: Option<ProjectRecord>, workflow: Option<WorkflowId>) -> ProjectOperationState {
    ProjectOperationState::Accepted { project, workflow }
}
fn get_project(
    table: &impl ReadableTable<&'static str, &'static [u8]>,
    id: &str,
) -> Result<Option<ProjectRecord>, PersistenceError> {
    table
        .get(id)
        .map_err(error::redb)?
        .map(|v| decode_project(id, v.value()))
        .transpose()
}
fn require_project(
    table: &impl ReadableTable<&'static str, &'static [u8]>,
    id: &str,
    expected: u64,
) -> Result<ProjectRecord, PersistenceError> {
    let value = get_project(table, id)?.ok_or_else(|| PersistenceError::NotFound {
        entity: "project",
        identity: id.into(),
    })?;
    if value.generation != expected {
        return Err(PersistenceError::ProjectGenerationConflict {
            expected,
            actual: value.generation,
        });
    }
    Ok(value)
}
fn insert_project(
    table: &mut redb::Table<'_, &str, &[u8]>,
    project: &ProjectRecord,
) -> Result<(), PersistenceError> {
    validate_project(project)?;
    if table.len().map_err(error::redb)? >= MAX_PROJECTS {
        return Err(exhausted("project registration capacity is full"));
    }
    for row in table.iter().map_err(error::redb)? {
        let (key, bytes) = row.map_err(error::redb)?;
        let existing = decode_project(key.value(), bytes.value())?;
        if existing.id == project.id
            || existing.folder.identity == project.folder.identity
            || existing.folder.physical == project.folder.physical
        {
            return Err(bounds(
                "this physical directory is already registered; use its existing project",
            ));
        }
    }
    let bytes = json::encode(project, "project")?;
    table
        .insert(project.id.as_str(), bytes.as_slice())
        .map_err(error::redb)?;
    Ok(())
}
fn get_operation(
    table: &impl ReadableTable<&'static str, &'static [u8]>,
    key: &str,
) -> Result<Option<ProjectOperation>, PersistenceError> {
    table
        .get(key)
        .map_err(error::redb)?
        .map(|v| decode_operation(key, v.value()))
        .transpose()
}
fn decode_project(id: &str, bytes: &[u8]) -> Result<ProjectRecord, PersistenceError> {
    let project: ProjectRecord = json::decode(bytes, "project")?;
    validate_project(&project)?;
    if id != project.id {
        return Err(error::corruption("project key differs from record"));
    }
    Ok(project)
}
fn decode_operation(key: &str, bytes: &[u8]) -> Result<ProjectOperation, PersistenceError> {
    let operation: ProjectOperation = json::decode(bytes, "project operation")?;
    validate_operation(&operation)?;
    if key != receipt_key(&operation.actor, &operation.key)? {
        return Err(error::corruption(
            "project receipt key differs from actor and request",
        ));
    }
    Ok(operation)
}
fn validate_project(project: &ProjectRecord) -> Result<(), PersistenceError> {
    identity(&project.id)?;
    if project.generation == 0
        || !project.folder.requested.is_absolute()
        || !project.folder.physical.is_absolute()
        || project.folder.identity.is_empty()
    {
        return Err(bounds(
            "project needs a positive generation and pinned absolute directory identity",
        ));
    }
    Ok(())
}
fn validate_operation(operation: &ProjectOperation) -> Result<(), PersistenceError> {
    identity(&operation.id)?;
    receipt_key(&operation.actor, &operation.key)?;
    if serde_json::to_vec(operation)?.len() > MAX_PROJECT_OPERATION_BYTES {
        return Err(bounds("project operation exceeds 32 KiB"));
    }
    let value: serde_json::Value = serde_json::from_slice(&operation.canonical_request)?;
    if serde_json::to_vec(&value)? != operation.canonical_request {
        return Err(bounds("project request is not canonical JSON"));
    }
    match &operation.state {
        ProjectOperationState::Creating {
            parent,
            folder_name,
            project,
        } => {
            validate_project(project)?;
            if folder_name.is_empty()
                || folder_name == "."
                || folder_name == ".."
                || folder_name.len() > 255
                || folder_name.contains(['/', '\\'])
                || folder_name.chars().any(char::is_control)
                || project.folder.requested != parent.requested.join(folder_name)
            {
                return Err(bounds("creation requires one exact child component"));
            }
        }
        ProjectOperationState::Accepted {
            project: Some(project),
            ..
        } => validate_project(project)?,
        ProjectOperationState::Accepted { project: None, .. }
        | ProjectOperationState::Abandoned => {}
    }
    Ok(())
}
fn receipt_key(actor: &ActorRef, key: &str) -> Result<String, PersistenceError> {
    identity(key)?;
    Ok(format!("{}:{actor}{key}", actor.as_str().len()))
}
fn identity(value: &str) -> Result<(), PersistenceError> {
    if value.is_empty()
        || value.len() > 512
        || matches!(value, "." | "..")
        || value.chars().any(char::is_control)
    {
        Err(bounds(
            "project/request identity must contain 1–512 non-control bytes",
        ))
    } else {
        Ok(())
    }
}
fn bounds(reason: &str) -> PersistenceError {
    PersistenceError::Bounds {
        location: "project",
        reason: reason.into(),
    }
}
fn exhausted(message: &str) -> PersistenceError {
    PersistenceError::Storage {
        class: StorageFailureClass::ResourceExhausted,
        message: message.into(),
    }
}
fn conflict(operation: &ProjectOperation) -> PersistenceError {
    PersistenceError::ProjectIdempotencyConflict {
        actor: operation.actor.clone(),
        key: operation.key.clone(),
    }
}

pub(crate) fn associate_new(
    write: &redb::WriteTransaction,
    workflow: &WorkflowId,
    project: &str,
) -> Result<(), PersistenceError> {
    let table = write.open_table(PROJECTS).map_err(error::redb)?;
    if get_project(&table, project)?.is_none() {
        return Err(bounds("new workflow project is missing"));
    }
    let mut links = write.open_table(WORKFLOW_PROJECTS).map_err(error::redb)?;
    if links.get(workflow.as_str()).map_err(error::redb)?.is_some() {
        return Err(bounds("new workflow already has a project association"));
    }
    links
        .insert(workflow.as_str(), project)
        .map_err(error::redb)?;
    Ok(())
}

pub(crate) fn guard_generation(
    write: &redb::WriteTransaction,
    project: &str,
    generation: u64,
) -> Result<(), PersistenceError> {
    let projects = write.open_table(PROJECTS).map_err(error::redb)?;
    require_project(&projects, project, generation)?;
    Ok(())
}

pub(crate) fn validate(read: &redb::ReadTransaction) -> Result<(), PersistenceError> {
    let projects = read.open_table(PROJECTS).map_err(error::redb)?;
    let operations = read.open_table(PROJECT_OPERATIONS).map_err(error::redb)?;
    let reserved = pending_creations(&operations)?;
    if projects
        .len()
        .map_err(error::redb)?
        .checked_add(reserved)
        .is_none_or(|n| n > MAX_PROJECTS)
        || operations
            .len()
            .map_err(error::redb)?
            .checked_add(reserved)
            .is_none_or(|n| n > MAX_PROJECT_OPERATIONS)
    {
        return Err(error::corruption("project capacity exceeded"));
    }
    for row in projects.iter().map_err(error::redb)? {
        let (key, value) = row.map_err(error::redb)?;
        decode_project(key.value(), value.value())?;
    }
    for row in operations.iter().map_err(error::redb)? {
        let (key, value) = row.map_err(error::redb)?;
        decode_operation(key.value(), value.value())?;
    }
    let workflows = read.open_table(TASK_WORKFLOWS).map_err(error::redb)?;
    for row in read
        .open_table(WORKFLOW_PROJECTS)
        .map_err(error::redb)?
        .iter()
        .map_err(error::redb)?
    {
        let (workflow, project) = row.map_err(error::redb)?;
        if get_project(&projects, project.value())?.is_none()
            || workflows
                .get(workflow.value())
                .map_err(error::redb)?
                .is_none()
        {
            return Err(error::corruption(
                "project workflow association is dangling",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
