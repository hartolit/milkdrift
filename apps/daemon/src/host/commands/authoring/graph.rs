//! The supported editor shape is recognized by rebuilding and comparing all semantic content.
//! This makes refusal explicit when an imported definition carries richer behavior.

use super::BuildResult;
use milkdrift_blueprint::{
    AuthorRef, BindingSource, BlueprintMetadata, BlueprintRevision, DataPort, Edge, EdgeId,
    EdgeKind, FieldId, InterfaceField, Mutation, MutationBatch, Node, NodeId, NodeKind,
    PathSelector, PortId, SchemaRef, TerminalOutcome, WorkflowId, WorkflowInterface,
};
use milkdrift_capability::{BoundedJson, CapabilityRequirement, OperationId, SchemaId};
use milkdrift_control::{
    ResultAcceptanceContract, ResultRequirement, result_acceptance_gate, result_acceptance_task,
};
use milkdrift_model::{MODEL_TASK_INPUT_NAME, ModelTaskRequest, ModelTaskRequestDocument};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

const DONE: &str = "author.done";
const PREFIX: &str = "author.";

#[derive(Clone)]
pub(super) struct Step {
    pub(super) id: String,
    pub(super) requirement: CapabilityRequirement,
    pub(super) acceptance_requirement: CapabilityRequirement,
    pub(super) request: ModelTaskRequest,
    pub(super) inputs: BTreeMap<String, BindingSource>,
}

pub(super) struct ModelWorkflow {
    pub(super) name: String,
    pub(super) inputs: BTreeSet<String>,
    pub(super) steps: Vec<Step>,
    pub(super) output: Option<(String, String)>,
}

pub(super) fn checked_name(name: &str) -> BuildResult<()> {
    NodeId::new(name)?;
    if name.starts_with(PREFIX) {
        return Err("the author. namespace is reserved for editor routing".into());
    }
    Ok(())
}
fn schema(id: &str) -> BuildResult<SchemaRef> {
    Ok(SchemaRef::new(SchemaId::new(id)?, 1)?)
}
pub(super) fn artifact() -> BuildResult<SchemaRef> {
    schema("milkdrift.artifact-reference")
}
pub(super) fn model_source(step: &str) -> BuildResult<BindingSource> {
    Ok(BindingSource::NodeOutput {
        node: NodeId::new(step)?,
        port: PortId::new("final_text")?,
        path: PathSelector::new(vec![])?,
    })
}
fn generated(step: &str, role: &str) -> String {
    format!("author.{step}.{role}")
}

impl ModelWorkflow {
    pub(super) fn empty(workflow: &str) -> Self {
        Self {
            name: workflow.to_owned(),
            inputs: BTreeSet::new(),
            steps: vec![],
            output: None,
        }
    }

    pub(super) fn read(revision: &BlueprintRevision) -> BuildResult<Self> {
        let semantic = revision.semantic();
        let mut result = Self::empty(semantic.workflow().as_str());
        result.name = semantic.metadata().name().to_owned();
        result.inputs = semantic
            .interface()
            .inputs()
            .keys()
            .map(ToString::to_string)
            .collect();
        let mut next = semantic.nodes().values().find(|node| matches!(node.kind(), NodeKind::Task { config } if config.requirement().operation().as_str() == "model.generate") && node.control_inputs().is_empty()).map(|node| node.id().clone());
        let mut seen = BTreeSet::new();
        while let Some(id) = next {
            if !seen.insert(id.clone()) {
                return Err("unsupported cyclic editor ordering".into());
            }
            let node = semantic.nodes().get(&id).ok_or("missing model step")?;
            let NodeKind::Task { config } = node.kind() else {
                return Err("unsupported editor step".into());
            };
            let Some(BindingSource::Literal { value }) = node
                .data_inputs()
                .get(&PortId::new(MODEL_TASK_INPUT_NAME)?)
                .and_then(DataPort::binding)
            else {
                return Err("editor needs an inline model request".into());
            };
            let request = ModelTaskRequestDocument::from_json(&serde_json::to_vec(value.value())?)?
                .body()
                .clone();
            prompt(&request)?;
            let acceptance = semantic
                .nodes()
                .get(&NodeId::new(generated(id.as_str(), "accept"))?)
                .ok_or("missing result acceptance")?;
            let NodeKind::Task { config: acceptance } = acceptance.kind() else {
                return Err("unsupported result acceptance".into());
            };
            let inputs = node
                .data_inputs()
                .iter()
                .filter(|(name, _)| name.as_str() != MODEL_TASK_INPUT_NAME)
                .map(|(name, port)| {
                    Ok((
                        name.to_string(),
                        port.binding()
                            .ok_or("unsupported unbound editor input")?
                            .clone(),
                    ))
                })
                .collect::<BuildResult<_>>()?;
            result.steps.push(Step {
                id: id.to_string(),
                requirement: config.requirement().clone(),
                acceptance_requirement: acceptance.requirement().clone(),
                request,
                inputs,
            });
            let gate = generated(id.as_str(), "gate");
            next = semantic
                .edges()
                .values()
                .find(|edge| {
                    edge.kind() == EdgeKind::Control
                        && edge.source_node().as_str() == gate
                        && edge.source_port().as_str() == "pass"
                })
                .map(|edge| edge.target_node().clone())
                .filter(|node| node.as_str() != DONE);
        }
        let done = semantic
            .nodes()
            .get(&NodeId::new(DONE)?)
            .ok_or("unsupported editor terminal")?;
        if let Some((name, port)) = done.data_inputs().iter().next() {
            let Some(BindingSource::NodeOutput { node, .. }) = port.binding() else {
                return Err("unsupported final output".into());
            };
            result.output = Some((node.to_string(), name.to_string()));
        }
        let rebuilt = result.build(
            semantic.workflow().clone(),
            revision.author().clone(),
            revision.reason(),
        )?;
        if rebuilt.semantic() != semantic {
            return Err("unsupported edit: this definition has features the model workflow editor cannot preserve; use explicit blueprint mutations".into());
        }
        Ok(result)
    }

    pub(super) fn complete(&self) -> BuildResult<()> {
        if self.steps.is_empty() || self.output.is_none() {
            return Err("add a model step and select a final output before saving".into());
        }
        for step in &self.steps {
            let text = prompt(&step.request)?;
            if text.trim().is_empty() {
                return Err(format!("step {} requires a nonempty prompt", step.id).into());
            }
        }
        Ok(())
    }

    pub(super) fn view(&self) -> Value {
        json!({"name":self.name,"inputs":self.inputs,"output":self.output,"steps":self.steps.iter().map(|step| json!({"step":step.id,"capability":step.requirement.exact_capability(),"provider_profile":step.requirement.provider_profile_ref(),"prompt":prompt(&step.request).ok(),"maximum_output_units":step.request.maximum_output_units(),"inputs":step.inputs})).collect::<Vec<_>>()})
    }

    pub(super) fn build(
        &self,
        workflow: WorkflowId,
        author: AuthorRef,
        reason: &str,
    ) -> BuildResult<BlueprintRevision> {
        let mut nodes = Vec::new();
        let mut edges = Vec::new();
        let inputs = self
            .inputs
            .iter()
            .map(|name| Ok((FieldId::new(name)?, InterfaceField::required(artifact()?))))
            .collect::<BuildResult<Vec<_>>>()?;
        let outputs = self
            .output
            .iter()
            .map(|(_, name)| Ok((FieldId::new(name)?, InterfaceField::required(artifact()?))))
            .collect::<BuildResult<Vec<_>>>()?;
        let mut done = Node::new(
            NodeId::new(DONE)?,
            NodeKind::Terminal {
                outcome: TerminalOutcome::Success,
            },
        )?;
        if !self.steps.is_empty() {
            done = done.with_control_input(PortId::new("in")?)?;
        }
        if let Some((step, name)) = &self.output {
            done = done.with_data_input(
                PortId::new(name)?,
                DataPort::input(artifact()?, true, Some(model_source(step)?))?,
            )?;
            add_edge(&mut edges, EdgeKind::Data, step, "final_text", DONE, name)?;
        }
        nodes.push(done);
        for (index, step) in self.steps.iter().enumerate() {
            checked_name(&step.id)?;
            let mut task = Node::new(
                NodeId::new(&step.id)?,
                NodeKind::task_direct_inputs(step.requirement.clone())?,
            )?
            .with_control_output(PortId::new("out")?)?;
            if index > 0 {
                task = task.with_control_input(PortId::new("in")?)?;
            }
            task = task.with_data_input(
                PortId::new(MODEL_TASK_INPUT_NAME)?,
                DataPort::input(
                    schema("milkdrift.model-task")?,
                    true,
                    Some(BindingSource::Literal {
                        value: BoundedJson::new(serde_json::to_value(
                            ModelTaskRequestDocument::new(step.request.clone()),
                        )?)?,
                    }),
                )?,
            )?;
            for (name, source) in &step.inputs {
                if let BindingSource::NodeOutput { node, port, .. } = source {
                    if !self.steps[..index]
                        .iter()
                        .any(|step| step.id == node.as_str())
                    {
                        return Err(format!(
                            "input {name} of {} must reference an earlier step",
                            step.id
                        )
                        .into());
                    }
                    add_edge(
                        &mut edges,
                        EdgeKind::Data,
                        node.as_str(),
                        port.as_str(),
                        &step.id,
                        name,
                    )?;
                }
                task = task.with_data_input(
                    PortId::new(name)?,
                    DataPort::input(artifact()?, true, Some(source.clone()))?,
                )?;
            }
            for name in [
                "final_text",
                "model_response",
                "structured_output",
                "tool_calls",
                "provider_metadata",
            ] {
                task = task.with_data_output(PortId::new(name)?, DataPort::output(artifact()?))?;
            }
            nodes.push(task);
            let accept = generated(&step.id, "accept");
            let gate = generated(&step.id, "gate");
            let hold = generated(&step.id, "hold");
            let failed = generated(&step.id, "failed");
            nodes.push(acceptance_task(
                &accept,
                step.acceptance_requirement.clone(),
            )?);
            nodes.push(result_acceptance_gate(
                NodeId::new(&gate)?,
                NodeId::new(&accept)?,
                artifact()?,
            )?);
            nodes.push(
                Node::new(
                    NodeId::new(&hold)?,
                    NodeKind::SignalWait {
                        signal: OperationId::new("workflow.reviewed")?,
                    },
                )?
                .with_control_input(PortId::new("in")?)?
                .with_control_output(PortId::new("out")?)?,
            );
            nodes.push(
                Node::new(
                    NodeId::new(&failed)?,
                    NodeKind::Terminal {
                        outcome: TerminalOutcome::Failure,
                    },
                )?
                .with_control_input(PortId::new("in")?)?,
            );
            add_edge(
                &mut edges,
                EdgeKind::Control,
                &step.id,
                "out",
                &accept,
                "in",
            )?;
            add_edge(
                &mut edges,
                EdgeKind::Data,
                &step.id,
                "model_response",
                &accept,
                "result",
            )?;
            add_edge(&mut edges, EdgeKind::Control, &accept, "out", &gate, "in")?;
            add_edge(
                &mut edges,
                EdgeKind::Data,
                &accept,
                "accepted_result",
                &gate,
                "accepted_result",
            )?;
            add_edge(&mut edges, EdgeKind::Control, &gate, "fail", &hold, "in")?;
            add_edge(&mut edges, EdgeKind::Control, &hold, "out", &failed, "in")?;
            let next = self
                .steps
                .get(index + 1)
                .map_or(DONE, |step| step.id.as_str());
            add_edge(&mut edges, EdgeKind::Control, &gate, "pass", next, "in")?;
        }
        let mut mutations = vec![
            Mutation::SetMetadata {
                metadata: BlueprintMetadata::new(&self.name, "", BTreeSet::new(), BTreeMap::new())?,
            },
            Mutation::SetInterface {
                interface: WorkflowInterface::new(inputs, outputs)?,
            },
        ];
        mutations.extend(nodes.into_iter().map(|node| Mutation::AddNode { node }));
        mutations.extend(edges.into_iter().map(|edge| Mutation::AddEdge { edge }));
        Ok(BlueprintRevision::genesis(
            workflow,
            MutationBatch::new(mutations)?,
            author,
            reason,
        )?)
    }
}

pub(super) fn prompt(request: &ModelTaskRequest) -> BuildResult<&str> {
    use milkdrift_model::{ContentPart, MessageRole, SessionSelection};
    let [message] = request.messages() else {
        return Err("unsupported message structure".into());
    };
    let [ContentPart::Text { text }] = message.parts() else {
        return Err("unsupported prompt parts".into());
    };
    if message.role() != MessageRole::User
        || !request.tools().is_empty()
        || request.structured_output().is_some()
        || request.session() != &SessionSelection::Fresh
        || request.reasoning().is_some()
        || !request.extensions().is_empty()
        || request.streaming()
    {
        return Err("unsupported model request options".into());
    }
    Ok(text)
}

pub(super) fn add_edge(
    edges: &mut Vec<Edge>,
    kind: EdgeKind,
    source: &str,
    port: &str,
    target: &str,
    input: &str,
) -> BuildResult<()> {
    // Edge IDs are implementation details returned with ordinary mutations, never client hashes.
    let id = format!(
        "author.{}",
        blake3::hash(format!("{kind:?}:{source}:{port}:{target}:{input}").as_bytes())
    );
    edges.push(Edge::new(
        EdgeId::new(id)?,
        kind,
        NodeId::new(source)?,
        PortId::new(port)?,
        NodeId::new(target)?,
        PortId::new(input)?,
    ));
    Ok(())
}

pub(super) fn difference(
    base: Option<&BlueprintRevision>,
    target: &BlueprintRevision,
) -> Vec<Mutation> {
    let target = target.semantic();
    let Some(base) = base.map(BlueprintRevision::semantic) else {
        let mut result = vec![
            Mutation::SetMetadata {
                metadata: target.metadata().clone(),
            },
            Mutation::SetInterface {
                interface: target.interface().clone(),
            },
        ];
        result.extend(
            target
                .nodes()
                .values()
                .cloned()
                .map(|node| Mutation::AddNode { node }),
        );
        result.extend(
            target
                .edges()
                .values()
                .cloned()
                .map(|edge| Mutation::AddEdge { edge }),
        );
        return result;
    };
    let mut result = Vec::new();
    if base.metadata() != target.metadata() {
        result.push(Mutation::SetMetadata {
            metadata: target.metadata().clone(),
        });
    }
    if base.interface() != target.interface() {
        result.push(Mutation::SetInterface {
            interface: target.interface().clone(),
        });
    }
    for id in base
        .edges()
        .keys()
        .filter(|id| !target.edges().contains_key(*id))
    {
        result.push(Mutation::RemoveEdge { edge: id.clone() });
    }
    for id in base
        .nodes()
        .keys()
        .filter(|id| !target.nodes().contains_key(*id))
    {
        result.push(Mutation::RemoveNode { node: id.clone() });
    }
    for (id, node) in target.nodes() {
        match base.nodes().get(id) {
            None => result.push(Mutation::AddNode { node: node.clone() }),
            Some(old) if old != node => result.push(Mutation::ReplaceNode { node: node.clone() }),
            _ => {}
        }
    }
    for (id, edge) in target.edges() {
        match base.edges().get(id) {
            None => result.push(Mutation::AddEdge { edge: edge.clone() }),
            Some(old) if old != edge => result.push(Mutation::ReplaceEdge { edge: edge.clone() }),
            _ => {}
        }
    }
    result
}

fn acceptance_task(id: &str, requirement: CapabilityRequirement) -> BuildResult<Node> {
    let template = result_acceptance_task(
        NodeId::new(id)?,
        ResultAcceptanceContract::new(ResultRequirement::ModelProse, BTreeSet::new())?,
        artifact()?,
    )?;
    let mut node = Node::new(
        template.id().clone(),
        NodeKind::task_direct_inputs(requirement)?,
    )?;
    for port in template.control_inputs() {
        node = node.with_control_input(port.clone())?;
    }
    for port in template.control_outputs() {
        node = node.with_control_output(port.clone())?;
    }
    for (name, port) in template.data_inputs() {
        node = node.with_data_input(name.clone(), port.clone())?;
    }
    for (name, port) in template.data_outputs() {
        node = node.with_data_output(name.clone(), port.clone())?;
    }
    Ok(node)
}
