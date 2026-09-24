//! A governed ordinary-process method shared by direct and peer transport tests.
use milkdrift_blueprint::{
    AdaptationScope, AuthorRef, BlueprintRevision, DataPort, Edge, EdgeId, EdgeKind, FieldId,
    GoverningAgreement, InterfaceField, Mutation, MutationBatch, Node, NodeId, NodeKind, PortId,
    SchemaRef, TerminalOutcome, WorkflowId, WorkflowInterface,
};
use milkdrift_capability::{
    CapabilityId, CapabilityRequirement, OperationId, SchemaId, SideEffectClass,
};
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

pub fn definition() -> TestResult<(BlueprintRevision, BlueprintRevision)> {
    let schema = SchemaRef::new(SchemaId::new("milkdrift.artifact-reference")?, 1)?;
    let requirement = CapabilityRequirement::new(OperationId::new("process.execute")?)
        .exact(CapabilityId::new("golden-local-process")?)
        .maximum_side_effect(SideEffectClass::None);
    let start = Node::new(NodeId::new("start")?, NodeKind::Wait { duration_ms: 250 })?
        .with_control_output(PortId::new("out")?)?;
    let work = Node::new(
        NodeId::new("repair.work")?,
        NodeKind::task_direct_inputs(requirement.clone())?,
    )?
    .with_control_input(PortId::new("in")?)?
    .with_control_output(PortId::new("out")?)?
    .with_data_output(PortId::new("result")?, DataPort::output(schema.clone()))?;
    let done = Node::new(
        NodeId::new("done")?,
        NodeKind::Terminal {
            outcome: TerminalOutcome::Success,
        },
    )?
    .with_control_input(PortId::new("in")?)?
    .with_data_input(
        PortId::new("result")?,
        DataPort::input(schema.clone(), true, None)?,
    )?;
    let edge =
        |id: &str, kind, from: &str, output: &str, to: &str, input: &str| -> TestResult<Edge> {
            Ok(Edge::new(
                EdgeId::new(id)?,
                kind,
                NodeId::new(from)?,
                PortId::new(output)?,
                NodeId::new(to)?,
                PortId::new(input)?,
            ))
        };
    let base = BlueprintRevision::genesis(
        WorkflowId::new("published-test")?,
        MutationBatch::new(vec![
            Mutation::SetInterface {
                interface: WorkflowInterface::new(
                    [],
                    [(FieldId::new("result")?, InterfaceField::required(schema))],
                )?,
            },
            Mutation::AddNode { node: start },
            Mutation::AddNode { node: work },
            Mutation::AddNode { node: done },
            Mutation::AddEdge {
                edge: edge(
                    "begin",
                    EdgeKind::Control,
                    "start",
                    "out",
                    "repair.work",
                    "in",
                )?,
            },
            Mutation::AddEdge {
                edge: edge(
                    "finish",
                    EdgeKind::Control,
                    "repair.work",
                    "out",
                    "done",
                    "in",
                )?,
            },
            Mutation::AddEdge {
                edge: edge(
                    "result",
                    EdgeKind::Data,
                    "repair.work",
                    "result",
                    "done",
                    "result",
                )?,
            },
        ])?,
        AuthorRef::new("human:operator")?,
        "finite public output fixture",
    )?;
    let agreement = GoverningAgreement::seal(
        NodeId::new("agreement")?,
        &base,
        AdaptationScope::new("repair.".to_owned(), 4, vec![requirement])?
            .with_maximum_revisions(2)?,
        format!("b3_{}", "1".repeat(64)),
    )?;
    let revision = base.revise(
        base.id(),
        MutationBatch::new(vec![Mutation::SetAgreement {
            agreement: Some(agreement),
        }])?,
        AuthorRef::new("human:operator")?,
        "seal output and operation boundary",
    )?;
    Ok((base, revision))
}
