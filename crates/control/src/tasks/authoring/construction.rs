//! Translate visible card and wire intentions into the existing ordinary task admission path.

use super::{
    AuthoringIssue, AuthoringReview, AuthoringScope, TaskControlService, context_input_name,
    invalid,
};
use crate::{ActorAuthorityContext, ControlError};
use milkdrift_authority::{
    AuthorityBudget, AuthorityExecutionProvenance, AuthorityOperation, AuthorityRequest,
    BoundaryTimeMillis, DecisionId, RequestedResourceFacts,
};
use milkdrift_blueprint::authoring::{
    AuthoringConnection, AuthoringConnectionKind, AuthoringDocument,
};
use milkdrift_blueprint::{
    MAX_TASK_BATCH_ACTIONS, MAX_TASK_OUTPUT_BYTES, TaskBatch, TaskChange, TaskDefinition,
    TaskDependencies, TaskId, TaskInputBinding, TaskInputValue, TaskOrderCondition,
    TaskOutputDeclaration,
};
use milkdrift_capability::TaskAuthoringContract;
use milkdrift_runtime::WorkflowRecord;
use std::collections::{BTreeMap, BTreeSet};

impl TaskControlService {
    pub(super) fn compile_authoring(
        &self,
        context: &ActorAuthorityContext,
        record: &WorkflowRecord,
        document: &AuthoringDocument,
        generation: u64,
        scope: &AuthoringScope,
    ) -> Result<(AuthoringReview, Vec<TaskChange>), ControlError> {
        document
            .validate()
            .map_err(|error| invalid(error.to_string()))?;
        if scope.tasks.len() > 200 || scope.changes.len() > 200 || scope.use_current.len() > 200 {
            return Err(invalid("Run scope exceeds the authoring envelope"));
        }
        let mut selected: BTreeSet<String> = scope.tasks.iter().cloned().collect();
        let mut issues = Vec::new();
        let mut prospective_inputs = record.inputs.clone();
        for change in scope.changes.iter().filter_map(|index| {
            usize::try_from(*index)
                .ok()
                .and_then(|index| document.changes.get(index))
        }) {
            if let TaskChange::BindInput { name, value } = change {
                prospective_inputs.insert(name.clone(), value.clone());
            }
        }
        if selected.len() != scope.tasks.len()
            || scope.changes.iter().collect::<BTreeSet<_>>().len() != scope.changes.len()
        {
            issue(
                &mut issues,
                "scope",
                "Choose each task and change once",
                None,
            );
        }
        // Expansion is a read result requiring review. Run compares this full set to its request.
        loop {
            let prior = selected.len();
            for wire in &document.connections {
                if selected.contains(&wire.target)
                    && let Some(predecessor) =
                        document.tasks.iter().find(|task| task.id == wire.source)
                    && predecessor.version.is_none()
                {
                    selected.insert(predecessor.id.clone());
                }
            }
            if selected.len() == prior {
                break;
            }
        }
        let mut definitions = BTreeMap::<String, TaskDefinition>::new();
        let mut contracts = BTreeMap::<String, TaskAuthoringContract>::new();
        for id in &selected {
            let Some(task) = document.tasks.iter().find(|task| &task.id == id) else {
                issue(
                    &mut issues,
                    "scope",
                    "This task is no longer in the saved pending plan",
                    Some(id),
                );
                continue;
            };
            if scope.use_current.contains(id) {
                issue(
                    &mut issues,
                    "predecessor_choice",
                    "Choose either the current task or its candidate changes",
                    Some(id),
                );
            }
            if let Some(version) = task.version {
                let accepted = record
                    .tasks
                    .iter()
                    .find(|(key, _)| key.as_str() == id)
                    .map(|(_, task)| task);
                if accepted.is_none_or(|task| {
                    task.version != version || task.frozen.is_some() || task.removed
                }) {
                    issue(
                        &mut issues,
                        "start_conflict",
                        "This task changed or started. Keep the candidate as future improvement",
                        Some(id),
                    );
                    continue;
                }
            }
            let Ok(mut definition) = task.definition.task_definition() else {
                issue(
                    &mut issues,
                    "incomplete",
                    "Choose a model or agent for this task",
                    Some(id),
                );
                continue;
            };
            if definition.instructions.trim().is_empty() {
                issue(
                    &mut issues,
                    "incomplete",
                    "Describe what this task should do",
                    Some(id),
                );
            }
            if definition.title.trim().is_empty() {
                definition.title = milkdrift_contracts::truncate_utf8(
                    definition
                        .instructions
                        .lines()
                        .next()
                        .unwrap_or_default()
                        .trim(),
                    96,
                )
                .to_owned();
            }
            let contract = self.runtime.task_authoring(&definition.capability)?;
            let Some((descriptor, contract)) = contract else {
                issue(
                    &mut issues,
                    "contract_changed",
                    "This model or agent is unavailable. Choose a current connection",
                    Some(id),
                );
                continue;
            };
            if !self.can_inspect_authoring(
                context,
                &descriptor,
                definition.capability.operation(),
            )? {
                issue(
                    &mut issues,
                    "contract_changed",
                    "This model or agent is unavailable. Choose a current connection",
                    Some(id),
                );
                continue;
            }
            let current = descriptor.descriptor_revision();
            if task.contract_generation != Some(current) {
                issue(
                    &mut issues,
                    "contract_changed",
                    "Review this model or agent's current input and output choices",
                    Some(id),
                );
                continue;
            }
            definition.capability_generation = task.contract_generation;
            for port in &contract.outputs {
                definition
                    .outputs
                    .entry(port.name.clone())
                    .or_insert_with(|| {
                        // Accepted promises survive an ordinary edit even if its sketch omitted
                        // them. New cards advertise their real results before anyone draws a wire.
                        task.version
                            .and_then(|_| record.tasks.iter().find(|(key, _)| key.as_str() == id))
                            .and_then(|(_, accepted)| accepted.definition.outputs.get(&port.name))
                            .cloned()
                            .unwrap_or_else(|| TaskOutputDeclaration {
                                required: port.required,
                                media_type: port.media_type.clone(),
                                max_bytes: port.max_bytes.min(MAX_TASK_OUTPUT_BYTES),
                            })
                    });
            }
            if !definition.inputs.is_empty()
                || definition.dependencies.references().next().is_some()
            {
                issue(
                    &mut issues,
                    "connections",
                    "Represent this candidate's inputs and ordering as connections before running it",
                    Some(id),
                );
            }
            // All included candidates use the same physical intentions. Legacy predicates are
            // projected explicitly when editing accepted work, rather than guessed from a changed input map.
            definition.dependencies = TaskDependencies::default();
            definition.inputs.clear();
            definitions.insert(id.clone(), definition);
            contracts.insert(id.clone(), contract);
        }
        for wire in &document.connections {
            if !selected.contains(&wire.target) {
                continue;
            }
            if let Some(source) = document.sources.iter().find(|source| source.id == wire.source)
                && let (Some(name), Some(value)) = (&source.shared_name, &source.value)
                && record.inputs.get(name) != Some(value)
                && !scope.changes.iter().filter_map(|index| usize::try_from(*index).ok()
                    .and_then(|index| document.changes.get(index)))
                    .any(|change| matches!(change, TaskChange::BindInput { name: supplied, value: supplied_value }
                        if supplied == name && supplied_value == value)) {
                wire_issue(&mut issues, "source_change", "Include the shared input change explicitly before using these edited bytes", wire);
            }
            let predecessor = document.accepted.get(&wire.source).unwrap_or(&wire.source);
            if let Some(candidate) = document.tasks.iter().find(|task| &task.id == predecessor)
                && candidate.version.is_some()
                && !selected.contains(predecessor)
                && !scope.use_current.contains(predecessor)
            {
                if issues.len() < 64 {
                    let mut issue = AuthoringIssue::task(
                        "predecessor_choice",
                        "This predecessor has unapplied changes. Choose Use current task or Apply these changes",
                        Some(predecessor),
                    );
                    issue.connection_id = Some(wire.id.clone());
                    issue.source_id = Some(wire.source.clone());
                    issues.push(issue);
                }
                continue;
            }
            // A selected result may have been published after this sketch was checkpointed.
            // Its current publication authority must precede any media or size diagnostic.
            if definitions.contains_key(&wire.target)
                && wire.kind == AuthoringConnectionKind::Data
                && !document
                    .sources
                    .iter()
                    .any(|source| source.id == wire.source)
                && let Some((task, producer)) = record
                    .tasks
                    .iter()
                    .find(|(task, _)| task.as_str() == predecessor)
            {
                if wire.outcome {
                    self.authorize_outcome_source(context, record, task)?;
                } else if let Some(reference) = wire
                    .output
                    .as_ref()
                    .and_then(|output| producer.result.as_ref()?.outputs.get(output))
                {
                    self.authorize_published_output(context, record, reference)?;
                }
            }
            compile_wire(
                document,
                record,
                wire,
                &contracts,
                &prospective_inputs,
                &mut definitions,
                &mut issues,
            )?;
        }
        for (id, definition) in &definitions {
            if let Some(contract) = contracts.get(id) {
                for input in &contract.inputs {
                    if input.required && !definition.inputs.contains_key(&input.name) {
                        issue(
                            &mut issues,
                            "incomplete",
                            format!("Connect {} before running this task", input.label),
                            Some(id),
                        );
                    }
                }
            }
            if let Err(error) = definition.validate() {
                issue(&mut issues, "incomplete", error.to_string(), Some(id));
            }
        }
        let mut changes = Vec::new();
        for task in document
            .tasks
            .iter()
            .filter(|task| selected.contains(&task.id))
        {
            if let Some(definition) = definitions.remove(&task.id) {
                let id = TaskId::new(&task.id).map_err(|error| invalid(error.to_string()))?;
                changes.push(match task.version {
                    Some(expected_version) => TaskChange::Revise {
                        task: id,
                        expected_version,
                        definition,
                    },
                    None => TaskChange::Add {
                        alias: id,
                        definition,
                    },
                });
            }
        }
        for index in &scope.changes {
            match usize::try_from(*index)
                .ok()
                .and_then(|index| document.changes.get(index))
            {
                Some(change) => changes.push(change.clone()),
                None => issue(
                    &mut issues,
                    "scope",
                    "A selected change is no longer saved",
                    None,
                ),
            }
        }
        if changes.is_empty() {
            issue(
                &mut issues,
                "scope",
                "Choose pending work to run or apply",
                None,
            );
        }
        if changes.len() > MAX_TASK_BATCH_ACTIONS {
            issue(
                &mut issues,
                "capacity",
                "One submission accepts at most 32 changes. Choose a smaller complete scope",
                None,
            );
        }
        let mut prospective = record
            .tasks
            .iter()
            .filter(|(_, task)| !task.removed)
            .map(|(id, task)| (id.clone(), task.definition.clone()))
            .collect::<BTreeMap<_, _>>();
        let mut limits = record.limits.clone();
        for change in &changes {
            self.review_project_change(context, record, change, &mut issues)?;
            match change {
                TaskChange::Add { alias, definition } => {
                    prospective.insert(alias.clone(), definition.clone());
                }
                TaskChange::Revise {
                    task, definition, ..
                } => {
                    prospective.insert(task.clone(), definition.clone());
                }
                TaskChange::Remove { task, .. } => {
                    prospective.remove(task);
                }
                TaskChange::SetLimits { limits: next } => {
                    limits = next.clone();
                }
                _ => {}
            }
        }
        if issues.is_empty()
            && let Err(error) =
                milkdrift_blueprint::validate_task_set(&prospective, &limits, |task, output| {
                    record
                        .tasks
                        .get(task)
                        .and_then(|task| task.result.as_ref())
                        .and_then(|result| result.outputs.get(output))
                })
        {
            issue(&mut issues, "topology", error.to_string(), None);
        }
        Ok((
            AuthoringReview {
                authoring_generation: generation,
                expected_generation: record.generation,
                tasks: selected.into_iter().collect(),
                changes: scope.changes.clone(),
                use_current: scope.use_current.clone(),
                issues,
            },
            changes,
        ))
    }

    fn review_project_change(
        &self,
        context: &ActorAuthorityContext,
        record: &WorkflowRecord,
        change: &TaskChange,
        issues: &mut Vec<AuthoringIssue>,
    ) -> Result<(), ControlError> {
        let task = match change {
            TaskChange::Add { alias, .. } => alias,
            TaskChange::Revise { task, .. } => task,
            _ => return Ok(()),
        };
        // Review reads the same project and native identities as Run, without retaining a new
        // binding. Run still resolves and transactionally checks the settings it actually admits.
        let mut probe = TaskBatch {
            expected_generation: record.generation,
            changes: vec![change.clone()],
        };
        match self.resolve_project_batch(context, record, &mut probe, false) {
            Ok(()) => Ok(()),
            Err(ControlError::Persistence(
                milkdrift_persistence::PersistenceError::ProjectGenerationConflict { .. },
            )) => {
                issue(
                    issues,
                    "project_changed",
                    "Project settings changed. Review this task's project settings before running it",
                    Some(task.as_str()),
                );
                Ok(())
            }
            Err(ControlError::InvalidContract(_)) => {
                issue(
                    issues,
                    "project_settings",
                    "Review this task's project and execution settings before running it",
                    Some(task.as_str()),
                );
                Ok(())
            }
            // Denied scope is not disclosed through local diagnostics, and unavailable storage
            // remains a retryable request failure rather than an instruction to change the plan.
            Err(error) => Err(error),
        }
    }

    fn can_inspect_authoring(
        &self,
        context: &ActorAuthorityContext,
        descriptor: &milkdrift_capability::CapabilityDescriptor,
        operation: &milkdrift_capability::OperationId,
    ) -> Result<bool, ControlError> {
        let Some(contract) = descriptor.operation(operation) else {
            return Ok(false);
        };
        let mut resources = RequestedResourceFacts::empty();
        resources.capability = Some(descriptor.identity().clone());
        resources.category = Some(descriptor.category().clone());
        resources.capability_operation = Some(operation.clone());
        resources.provider_profile = descriptor.provider_profile().cloned();
        resources.trust_zones = descriptor.trust_zones().clone();
        resources.execution_trust_class = Some(descriptor.execution_trust());
        resources.locality = Some(descriptor.locality());
        resources.peer = descriptor.peer().cloned();
        resources.side_effect = contract.side_effect();
        let claim = context.authority();
        // These are the existing catalog gates, evaluated with the exact descriptor's complete
        // scope facts. Reading an editor's choices never requires permission to invoke them.
        for permission in [
            AuthorityOperation::ListCapabilities,
            AuthorityOperation::InspectCapabilityHealth,
            AuthorityOperation::InspectProviderProfile,
        ] {
            let identity = serde_json::to_vec(&(
                context.actor(),
                descriptor.identity(),
                descriptor.descriptor_revision(),
                operation,
                permission,
            ))?;
            let request = AuthorityRequest {
                decision: DecisionId::new(format!("task_authoring_{}", blake3::hash(&identity)))?,
                actor: context.actor().clone(),
                grant: claim.grant().clone(),
                grant_revision: claim.grant_revision(),
                grant_digest: claim.grant_digest().clone(),
                revocation_generation: claim.revocation_generation(),
                operation: permission,
                resources: resources.clone(),
                budget: AuthorityBudget::default(),
                evaluated_at: BoundaryTimeMillis::new(self.clock.now()?.get()),
                provenance: AuthorityExecutionProvenance::default(),
            };
            if !self.authority.evaluate(&request)?.is_allowed() {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

fn compile_wire(
    document: &AuthoringDocument,
    record: &WorkflowRecord,
    wire: &AuthoringConnection,
    contracts: &BTreeMap<String, TaskAuthoringContract>,
    prospective_inputs: &BTreeMap<String, TaskInputValue>,
    definitions: &mut BTreeMap<String, TaskDefinition>,
    issues: &mut Vec<AuthoringIssue>,
) -> Result<(), ControlError> {
    if !definitions.contains_key(&wire.target) {
        return Ok(());
    }
    let source_id = document.accepted.get(&wire.source).unwrap_or(&wire.source);
    if wire.kind != AuthoringConnectionKind::Data {
        let source = TaskId::new(source_id).map_err(|error| invalid(error.to_string()))?;
        if !definitions.contains_key(source_id) && !record.tasks.contains_key(&source) {
            wire_issue(
                issues,
                "source_unavailable",
                "The ordering predecessor is unavailable",
                wire,
            );
            return Ok(());
        }
        if let Some(definition) = definitions.get_mut(&wire.target) {
            let TaskDependencies::Ordered(order) = &mut definition.dependencies else {
                return Err(invalid("Authoring order is not explicit"));
            };
            let condition = if wire.kind == AuthoringConnectionKind::AfterCompletion {
                TaskOrderCondition::Settled
            } else {
                TaskOrderCondition::Success
            };
            if order
                .insert(source, condition)
                .is_some_and(|prior| prior != condition)
            {
                wire_issue(
                    issues,
                    "ordering",
                    "Choose one independent ordering condition for this predecessor",
                    wire,
                );
            }
        }
        return Ok(());
    }
    let Some(contract) = contracts.get(&wire.target) else {
        return Ok(());
    };
    let input = match &wire.input {
        Some(name) => name.clone(),
        None if contract.inputs.len() == 1 && !contract.allows_additional_inputs => contract
            .inputs
            .first()
            .map(|port| port.name.clone())
            .unwrap_or_default(),
        None if contract.inputs.is_empty() && contract.allows_additional_inputs => {
            context_input_name(&wire.id)
        }
        None => {
            wire_issue(
                issues,
                "incomplete",
                "Choose an input at this connection",
                wire,
            );
            return Ok(());
        }
    };
    if !contract.allows_additional_inputs && !contract.inputs.iter().any(|port| port.name == input)
    {
        wire_issue(
            issues,
            "contract_changed",
            "Choose an input from the current model or agent",
            wire,
        );
        return Ok(());
    }
    let binding = if let Some(source) = document
        .sources
        .iter()
        .find(|source| source.id == wire.source)
    {
        if wire.outcome || wire.output.is_some() {
            wire_issue(
                issues,
                "connection",
                "Text and file sources supply content directly",
                wire,
            );
            return Ok(());
        }
        match (&source.value, &source.shared_name) {
            (_, Some(name)) => TaskInputBinding::Supplied {
                name: name.clone(),
                optional: wire.optional,
            },
            (Some(value), None) => TaskInputBinding::Inline {
                value: value.clone(),
            },
            (None, None) => {
                wire_issue(
                    issues,
                    "incomplete",
                    "Supply this input before running the connected task",
                    wire,
                );
                return Ok(());
            }
        }
    } else {
        let source = TaskId::new(source_id).map_err(|error| invalid(error.to_string()))?;
        let pending = definitions.contains_key(source_id);
        let accepted = record.tasks.get(&source).filter(|task| !task.removed);
        if !pending && accepted.is_none() {
            wire_issue(
                issues,
                "source_unavailable",
                "This predecessor is unavailable or outside the reviewed scope",
                wire,
            );
            return Ok(());
        }
        if wire.outcome {
            TaskInputBinding::SettledOutcome { task: source }
        } else {
            let output = if pending {
                let Some(source_contract) = contracts.get(source_id) else {
                    return Ok(());
                };
                let selected = wire
                    .output
                    .as_ref()
                    .or(source_contract.default_output.as_ref())
                    .cloned()
                    .or_else(|| {
                        (source_contract.outputs.len() == 1)
                            .then(|| {
                                source_contract
                                    .outputs
                                    .first()
                                    .map(|port| port.name.clone())
                            })
                            .flatten()
                    });
                let Some(output) = selected else {
                    wire_issue(
                        issues,
                        "incomplete",
                        "Choose an output at this connection",
                        wire,
                    );
                    return Ok(());
                };
                if !source_contract
                    .outputs
                    .iter()
                    .any(|port| port.name == output)
                {
                    wire_issue(
                        issues,
                        "contract_changed",
                        "The selected output is no longer offered",
                        wire,
                    );
                    return Ok(());
                }
                output
            } else {
                let Some(output) = wire.output.clone() else {
                    wire_issue(
                        issues,
                        "incomplete",
                        "Choose an existing named result",
                        wire,
                    );
                    return Ok(());
                };
                if accepted.is_none_or(|task| {
                    !task.definition.outputs.contains_key(&output)
                        && !task
                            .result
                            .as_ref()
                            .is_some_and(|result| result.outputs.contains_key(&output))
                }) {
                    wire_issue(
                        issues,
                        "source_unavailable",
                        "That result exists only in an unapplied candidate, or is unavailable. Apply the predecessor changes explicitly",
                        wire,
                    );
                    return Ok(());
                }
                output
            };
            TaskInputBinding::TaskOutput {
                task: source,
                output,
                optional: wire.optional,
            }
        }
    };
    let known = match &binding {
        TaskInputBinding::Inline { value } => known_value(value),
        TaskInputBinding::Supplied { name, .. } => {
            prospective_inputs.get(name).and_then(known_value)
        }
        TaskInputBinding::SettledOutcome { .. } => {
            Some(("application/json".to_owned(), None, false))
        }
        TaskInputBinding::TaskOutput { task, output, .. } => record
            .tasks
            .get(task)
            .and_then(|task| task.result.as_ref())
            .and_then(|result| result.outputs.get(output))
            .and_then(|reference| {
                reference
                    .media_type()
                    .map(|media| (media.to_owned(), reference.size_bytes(), true))
            })
            .or_else(|| {
                definitions
                    .get(task.as_str())
                    .or_else(|| record.tasks.get(task).map(|task| &task.definition))
                    .and_then(|definition| definition.outputs.get(output))
                    .and_then(|output| {
                        output
                            .media_type
                            .as_ref()
                            .map(|media| (media.clone(), None, true))
                    })
            }),
    };
    if let Some((media, bytes, artifact)) = known {
        let port = contract.inputs.iter().find(|port| port.name == input);
        if !contract.input_media.accepts(&media)
            || port.is_some_and(|port| {
                (artifact && !port.accepts_artifacts)
                    || port
                        .media_type
                        .as_ref()
                        .is_some_and(|expected| expected != &media)
                    || bytes.is_some_and(|bytes| bytes > port.max_bytes)
            })
        {
            wire_issue(
                issues,
                "media_mismatch",
                "This source's content does not fit the receiving input. Choose a compatible source or task",
                wire,
            );
            return Ok(());
        }
    }
    if let Some(definition) = definitions.get_mut(&wire.target)
        && definition.inputs.insert(input, binding).is_some()
    {
        wire_issue(
            issues,
            "occupied_input",
            "This input has more than one connection. Choose the intended source",
            wire,
        );
    }
    Ok(())
}

fn known_value(value: &milkdrift_blueprint::TaskInputValue) -> Option<(String, Option<u64>, bool)> {
    use milkdrift_blueprint::TaskInputValue;
    match value {
        TaskInputValue::Text { text } | TaskInputValue::TextFile { text, .. } => {
            Some(("text/plain".into(), u64::try_from(text.len()).ok(), false))
        }
        TaskInputValue::Json { .. } => Some(("application/json".into(), None, false)),
        TaskInputValue::Artifact { reference } => reference
            .media_type()
            .map(|media| (media.to_owned(), reference.size_bytes(), true)),
        TaskInputValue::Absent => None,
    }
}

fn issue(
    issues: &mut Vec<AuthoringIssue>,
    code: &str,
    message: impl Into<String>,
    task: Option<&str>,
) {
    if issues.len() < 64 {
        issues.push(AuthoringIssue::task(code, message, task));
    }
}

fn wire_issue(
    issues: &mut Vec<AuthoringIssue>,
    code: &str,
    message: &str,
    wire: &AuthoringConnection,
) {
    if issues.len() < 64 {
        let mut issue = AuthoringIssue::task(code, message, Some(&wire.target));
        issue.connection_id = Some(wire.id.clone());
        issue.source_id = Some(wire.source.clone());
        issues.push(issue);
    }
}
