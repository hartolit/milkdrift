use super::{
    ActorSession, BuildResult, Owner, PublicFailure,
    graph::{ModelWorkflow, Step, checked_name, model_source},
};
use milkdrift_blueprint::{BindingSource, FieldId, PortId};
use milkdrift_capability::{
    CapabilityId, CapabilityRequirement, OperationId, ProviderProfileRef, SideEffectClass,
};
use milkdrift_control_protocol::{BlueprintEdit, ModelInputSource};
use milkdrift_model::{ContentPart, Message, MessageRole, ModelTaskRequest, SessionSelection};
use std::collections::BTreeMap;

fn model(
    owner: &Owner,
    session: &ActorSession,
    capability: &str,
) -> Result<CapabilityRequirement, PublicFailure> {
    let selected = owner
        .capabilities(session)?
        .into_iter()
        .find(|value| {
            value.capability_id == capability
                && value.current
                && !value.draining
                && value.operations.iter().any(|op| op == "model.generate")
        })
        .ok_or_else(|| {
            super::invalid("model is not available in the caller's permitted catalogue")
        })?;
    let mut requirement =
        CapabilityRequirement::new(OperationId::new("model.generate").map_err(super::failure)?)
            .exact(CapabilityId::new(capability).map_err(super::failure)?)
            .maximum_side_effect(SideEffectClass::Unknown);
    if let Some(profile) = selected.provider_profile {
        requirement =
            requirement.provider_profile(ProviderProfileRef::new(profile).map_err(super::failure)?);
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
            if model(owner, session, id.as_str())? != step.requirement {
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
                let requirement =
                    model(owner, session, capability).map_err(|error| error.message)?;
                self.steps.push(Step {
                    id: step.clone(),
                    requirement,
                    request: request(prompt, *maximum_output_units)?,
                    inputs: BTreeMap::new(),
                });
            }
            BlueprintEdit::Prompt { step, prompt } => {
                let step = self.step_mut(step)?;
                step.request = request(prompt, step.request.maximum_output_units())?;
            }
            BlueprintEdit::Model { step, capability } => {
                self.step_mut(step)?.requirement =
                    model(owner, session, capability).map_err(|error| error.message)?;
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
