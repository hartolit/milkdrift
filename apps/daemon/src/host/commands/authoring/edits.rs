use super::{
    ActorSession, BuildResult, Owner, PublicFailure,
    graph::{ModelWorkflow, Step, checked_name, model_source},
};
use milkdrift_blueprint::{BindingSource, FieldId, PortId};
use milkdrift_capability::{CapabilityRequirement, OperationId, PlacementRequirement};
use milkdrift_control_protocol::{BlueprintEdit, ModelInputSource};
use milkdrift_model::{ContentPart, Message, MessageRole, ModelTaskRequest, SessionSelection};
use std::collections::{BTreeMap, BTreeSet};

fn selection(
    owner: &Owner,
    session: &ActorSession,
    capability: &str,
    operation: &str,
) -> Result<CapabilityRequirement, PublicFailure> {
    let operation = OperationId::new(operation).map_err(super::failure)?;
    let selected = owner
        .capability_generations(session)?
        .into_iter()
        .find(|value| {
            value.capability.as_str() == capability
                && value.current
                && !value.draining
                && value.operation_contracts.contains_key(&operation)
        })
        .ok_or_else(|| {
            super::invalid(
                "required capability is not available in the caller's permitted catalogue",
            )
        })?;
    let contract = selected
        .operation_contracts
        .get(&operation)
        .ok_or_else(|| super::invalid("operation unavailable"))?;
    // An exact identity alone leaves every omitted dimension as Any at run admission.
    // Retain the catalogue's constraints so the author's narrow grant can admit this task.
    let mut requirement = CapabilityRequirement::new(operation)
        .exact(selected.capability)
        .category(selected.category)
        .execution_trust(selected.execution_trust)
        .maximum_side_effect(contract.side_effect())
        .with_placement(
            PlacementRequirement::new(
                Some(BTreeSet::from([selected.locality])),
                selected.peer.map(|peer| BTreeSet::from([peer])),
            )
            .map_err(super::failure)?,
        );
    for zone in selected.trust_zones {
        requirement = requirement.trust_zone(zone);
    }
    if let Some(profile) = selected.provider_profile {
        requirement = requirement.provider_profile(profile);
    }
    Ok(requirement)
}
fn request(prompt: &str, maximum: u64) -> BuildResult<ModelTaskRequest> {
    Ok(ModelTaskRequest::new(
        vec![Message::new(
            MessageRole::User,
            vec![ContentPart::Text {
                text: prompt.to_owned(),
            }],
            None,
        )?],
        vec![],
        None,
        SessionSelection::Fresh,
        None,
        maximum,
        false,
        BTreeMap::new(),
    )?)
}

impl ModelWorkflow {
    pub(super) fn authorize_models(
        &self,
        owner: &Owner,
        session: &ActorSession,
    ) -> Result<(), PublicFailure> {
        for step in &self.steps {
            let id = step
                .requirement
                .exact_capability()
                .ok_or_else(|| super::invalid("model selection must be explicit"))?;
            if selection(owner, session, id.as_str(), "model.generate")? != step.requirement
                || selection(
                    owner,
                    session,
                    "milkdrift-workflow-control",
                    "workflow.accept_result",
                )? != step.acceptance_requirement
            {
                return Err(super::invalid(
                    "selected model profile changed; explicitly select the model again",
                ));
            }
        }
        Ok(())
    }
    fn step_mut(&mut self, id: &str) -> BuildResult<&mut Step> {
        self.steps
            .iter_mut()
            .find(|step| step.id == id)
            .ok_or_else(|| format!("step {id} does not exist").into())
    }
    pub(super) fn edit(
        &mut self,
        owner: &Owner,
        session: &ActorSession,
        edit: &BlueprintEdit,
    ) -> BuildResult<()> {
        match edit {
            BlueprintEdit::Rename { name } => self.name = name.clone(),
            BlueprintEdit::AddModel {
                step,
                capability,
                prompt,
                maximum_output_units,
            } => {
                checked_name(step)?;
                if self.steps.iter().any(|existing| existing.id == *step) {
                    return Err("step already exists".into());
                }
                let requirement = selection(owner, session, capability, "model.generate")
                    .map_err(|error| error.message)?;
                self.steps.push(Step {
                    id: step.clone(),
                    requirement,
                    acceptance_requirement: selection(
                        owner,
                        session,
                        "milkdrift-workflow-control",
                        "workflow.accept_result",
                    )
                    .map_err(|error| error.message)?,
                    request: request(prompt, *maximum_output_units)?,
                    inputs: BTreeMap::new(),
                });
            }
            BlueprintEdit::Prompt { step, prompt } => {
                let step = self.step_mut(step)?;
                step.request = request(prompt, step.request.maximum_output_units())?;
            }
            BlueprintEdit::Model { step, capability } => {
                let requirement = selection(owner, session, capability, "model.generate")
                    .map_err(|error| error.message)?;
                let acceptance = selection(
                    owner,
                    session,
                    "milkdrift-workflow-control",
                    "workflow.accept_result",
                )
                .map_err(|error| error.message)?;
                let step = self.step_mut(step)?;
                step.requirement = requirement;
                step.acceptance_requirement = acceptance;
            }
            BlueprintEdit::Input { name } => {
                FieldId::new(name)?;
                if !self.inputs.insert(name.clone()) {
                    return Err("run input already declared".into());
                }
            }
            BlueprintEdit::Connect {
                step,
                input,
                source,
            } => {
                PortId::new(input)?;
                if input.starts_with("milkdrift.") {
                    return Err("reserved input name".into());
                }
                let source = match source {
                    ModelInputSource::RunInput { name } => {
                        if !self.inputs.contains(name) {
                            return Err("declare the run input before connecting it".into());
                        }
                        BindingSource::WorkflowInput {
                            field: FieldId::new(name)?,
                        }
                    }
                    ModelInputSource::Step { step } => model_source(step)?,
                };
                self.step_mut(step)?.inputs.insert(input.clone(), source);
            }
            BlueprintEdit::Disconnect { step, input } => {
                if self.step_mut(step)?.inputs.remove(input).is_none() {
                    return Err("connection does not exist".into());
                }
            }
            BlueprintEdit::Output { step, name } => {
                self.step_mut(step)?;
                FieldId::new(name)?;
                self.output = Some((step.clone(), name.clone()));
            }
            BlueprintEdit::Remove { step } => {
                self.step_mut(step)?;
                if self.output.as_ref().is_some_and(|(selected, _)| selected == step) || self.steps.iter().any(|value| value.inputs.values().any(|binding| matches!(binding, BindingSource::NodeOutput { node, .. } if node.as_str() == step))) {
                    return Err("step is still used by a connection or final output; change those selections first".into());
                }
                self.steps.retain(|value| value.id != *step);
            }
            BlueprintEdit::Move { step, before } => {
                let index = self
                    .steps
                    .iter()
                    .position(|value| value.id == *step)
                    .ok_or("step does not exist")?;
                if before.as_ref() == Some(step) {
                    return Err("cannot move a step before itself".into());
                }
                let moved = self.steps.remove(index);
                let target = match before {
                    Some(id) => self
                        .steps
                        .iter()
                        .position(|value| value.id == *id)
                        .ok_or("target step does not exist")?,
                    None => self.steps.len(),
                };
                self.steps.insert(target, moved);
            }
        }
        Ok(())
    }
}
