use milkdrift_blueprint::{
    BindingSource, BranchConfig, Condition, DataPort, Node, NodeId, NodeKind, PathSelector, PortId,
    SchemaRef,
};
use milkdrift_capability::{
    BoundedJson, CapabilityId, CapabilityRequirement, OperationId, SchemaId, SideEffectClass,
};
use std::collections::BTreeMap;

use crate::{
    ACCEPTED_RESULT_OUTPUT, ControlError, RESULT_ACCEPTANCE_INPUT, RESULT_ACCEPTANCE_OUTPUT,
    ResultAcceptanceContract, WORKFLOW_ACCEPT_RESULT_OPERATION,
};

/// Builds an ordinary task that evaluates immutable results through the installed control adapter.
///
/// Connect `in`/`out` control edges, the upstream `result` artifact, and the contract's named
/// evidence inputs. Inputs are optional so missing output becomes a recorded rejection. Route
/// an ordinary branch using `accepted_result`; the task's own success means evaluation finished.
pub fn result_acceptance_task(
    identity: NodeId,
    contract: ResultAcceptanceContract,
    artifact_schema: SchemaRef,
) -> Result<Node, ControlError> {
    let requirement =
        CapabilityRequirement::new(OperationId::new(WORKFLOW_ACCEPT_RESULT_OPERATION)?)
            .exact(CapabilityId::new("milkdrift-workflow-control")?)
            .maximum_side_effect(SideEffectClass::ReadOnly);
    let build = || -> Result<Node, ControlError> {
        let mut node = Node::new(identity, NodeKind::task_direct_inputs(requirement)?)?
            .with_control_input(PortId::new("in")?)?
            .with_control_output(PortId::new("out")?)?
            .with_data_input(
                PortId::new(RESULT_ACCEPTANCE_INPUT)?,
                DataPort::input(
                    SchemaRef::new(SchemaId::new("milkdrift.result_acceptance")?, 1)?,
                    true,
                    Some(BindingSource::Literal {
                        value: BoundedJson::new(serde_json::to_value(&contract)?)?,
                    }),
                )?,
            )?
            .with_data_input(
                PortId::new("result")?,
                DataPort::input(artifact_schema.clone(), false, None)?,
            )?
            .with_data_output(
                PortId::new(RESULT_ACCEPTANCE_OUTPUT)?,
                DataPort::output(artifact_schema.clone()),
            )?
            .with_data_output(
                PortId::new(ACCEPTED_RESULT_OUTPUT)?,
                DataPort::output(artifact_schema.clone()),
            )?;
        for name in contract.evidence_inputs() {
            node = node.with_data_input(
                PortId::new(name)?,
                DataPort::input(artifact_schema.clone(), false, None)?,
            )?;
        }
        Ok(node)
    };
    build()
}

/// Builds the ordinary `pass`/`fail` branch following an acceptance task.
/// Connect `in` from that task and its `accepted_result` data edge. Only `pass` may lead
/// to dependent work; `fail` must lead to an explicit failure or a separate durable hold.
pub fn result_acceptance_gate(
    identity: NodeId,
    acceptance: NodeId,
    artifact_schema: SchemaRef,
) -> Result<Node, ControlError> {
    let source = BindingSource::NodeOutput {
        node: acceptance,
        port: PortId::new(ACCEPTED_RESULT_OUTPUT)?,
        path: PathSelector::new(Vec::new())
            .map_err(|error| ControlError::InvalidContract(error.to_string()))?,
    };
    Ok(Node::new(
        identity,
        NodeKind::Branch {
            config: BranchConfig::new(
                BTreeMap::from([(
                    PortId::new("pass")?,
                    Condition::Exists {
                        source: source.clone(),
                    },
                )]),
                Some(PortId::new("fail")?),
            )?,
        },
    )?
    .with_control_input(PortId::new("in")?)?
    .with_control_output(PortId::new("pass")?)?
    .with_control_output(PortId::new("fail")?)?
    .with_data_input(
        PortId::new(ACCEPTED_RESULT_OUTPUT)?,
        DataPort::input(artifact_schema, false, Some(source))?,
    )?)
}
