//! Shared blueprint and workflow fixtures for structured runtime integration tests.

use super::*;

pub(super) fn wait_revision(workflow: &str, duration_ms: u64) -> TestResult<BlueprintRevision> {
    let wait = Node::new(NodeId::new("wait")?, NodeKind::Wait { duration_ms })?
        .with_control_output(PortId::new("out")?)?;
    revision(
        workflow,
        vec![wait, terminal("done", TerminalOutcome::Success)?],
        vec![control_edge("wait-done", "wait", "out", "done", "in")?],
    )
}

pub(super) fn signal_revision(workflow: &str) -> TestResult<BlueprintRevision> {
    let signal = Node::new(
        NodeId::new("signal")?,
        NodeKind::SignalWait {
            signal: OperationId::new("notify.ready")?,
        },
    )?
    .with_control_output(PortId::new("out")?)?;
    let wait = Node::new(NodeId::new("settle")?, NodeKind::Wait { duration_ms: 50 })?
        .with_control_input(PortId::new("in")?)?
        .with_control_output(PortId::new("out")?)?;
    revision(
        workflow,
        vec![signal, wait, terminal("done", TerminalOutcome::Success)?],
        vec![
            control_edge("signal-settle", "signal", "out", "settle", "in")?,
            control_edge("settle-done", "settle", "out", "done", "in")?,
        ],
    )
}

pub(super) fn broadcast_fanout_revision(
    workflow: &str,
    outputs_per_wait: usize,
) -> TestResult<BlueprintRevision> {
    let schema = item_schema()?;
    let a = PortId::new("a")?;
    let b = PortId::new("b")?;
    let fork = Node::new(
        NodeId::new("fork")?,
        NodeKind::Fork {
            config: ForkConfig::new(BTreeSet::from([a.clone(), b.clone()]))?,
        },
    )?
    .with_control_output(a)?
    .with_control_output(b)?;
    let mut left = Node::new(
        NodeId::new("left-wait")?,
        NodeKind::SignalWait {
            signal: OperationId::new("notify.broadcast")?,
        },
    )?
    .with_control_input(PortId::new("in")?)?
    .with_control_output(PortId::new("out")?)?;
    let mut right = Node::new(
        NodeId::new("right-wait")?,
        NodeKind::SignalWait {
            signal: OperationId::new("notify.broadcast")?,
        },
    )?
    .with_control_input(PortId::new("in")?)?
    .with_control_output(PortId::new("out")?)?;
    for index in 0..outputs_per_wait {
        let port = PortId::new(format!("payload-{index:03}"))?;
        left = left.with_data_output(port.clone(), DataPort::output(schema.clone()))?;
        right = right.with_data_output(port, DataPort::output(schema.clone()))?;
    }
    let join = Node::new(
        NodeId::new("join")?,
        NodeKind::Join {
            config: JoinConfig::new(NodeId::new("fork")?, JoinPolicy::All),
        },
    )?
    .with_control_input(PortId::new("a-in")?)?
    .with_control_input(PortId::new("b-in")?)?
    .with_control_output(PortId::new("out")?)?;
    revision(
        workflow,
        vec![
            fork,
            left,
            right,
            join,
            terminal("done", TerminalOutcome::Success)?,
        ],
        vec![
            control_edge("fork-left", "fork", "a", "left-wait", "in")?,
            control_edge("fork-right", "fork", "b", "right-wait", "in")?,
            control_edge("left-join", "left-wait", "out", "join", "a-in")?,
            control_edge("right-join", "right-wait", "out", "join", "b-in")?,
            control_edge("join-done", "join", "out", "done", "in")?,
        ],
    )
}

pub(super) fn output_child_revision(workflow: &str) -> TestResult<BlueprintRevision> {
    let schema = item_schema()?;
    let task = task("produce", "model.generate")?
        .with_data_output(PortId::new("result")?, DataPort::output(schema.clone()))?;
    let terminal = terminal("done", TerminalOutcome::Success)?.with_data_input(
        PortId::new("result")?,
        DataPort::input(schema.clone(), true, None)?,
    )?;
    revision_with_interface(
        workflow,
        WorkflowInterface::new(
            [],
            [(FieldId::new("result")?, InterfaceField::required(schema))],
        )?,
        vec![task, terminal],
        vec![
            control_edge("produce-done", "produce", "out", "done", "in")?,
            data_edge("produce-result", "produce", "result", "done", "result")?,
        ],
    )
}

pub(super) fn task_revision(workflow: &str) -> TestResult<BlueprintRevision> {
    revision(
        workflow,
        vec![
            task("work", "model.generate")?,
            terminal("done", TerminalOutcome::Success)?,
        ],
        vec![control_edge("work-done", "work", "out", "done", "in")?],
    )
}

pub(super) fn optional_workflow_input_revision(workflow: &str) -> TestResult<BlueprintRevision> {
    let schema = item_schema()?;
    let field = FieldId::new("optional")?;
    let consume = task("consume", "model.generate")?.with_data_input(
        PortId::new("optional")?,
        DataPort::input(
            schema.clone(),
            false,
            Some(BindingSource::WorkflowInput {
                field: field.clone(),
            }),
        )?,
    )?;
    revision_with_interface(
        workflow,
        WorkflowInterface::new([(field, InterfaceField::optional(schema))], [])?,
        vec![consume, terminal("done", TerminalOutcome::Success)?],
        vec![control_edge(
            "consume-done",
            "consume",
            "out",
            "done",
            "in",
        )?],
    )
}

pub(super) fn producer_consumer_revision(workflow: &str) -> TestResult<BlueprintRevision> {
    let schema = item_schema()?;
    let produce = task("produce", "model.generate")?
        .with_data_output(PortId::new("result")?, DataPort::output(schema.clone()))?;
    let consume = task("consume", "model.fail")?
        .with_data_input(PortId::new("input")?, DataPort::input(schema, true, None)?)?;
    revision(
        workflow,
        vec![
            produce,
            consume,
            terminal("done", TerminalOutcome::Success)?,
        ],
        vec![
            control_edge("produce-consume", "produce", "out", "consume", "in")?,
            control_edge("consume-done", "consume", "out", "done", "in")?,
            data_edge("result-input", "produce", "result", "consume", "input")?,
        ],
    )
}

pub(super) fn removable_task_revision(workflow: &str) -> TestResult<BlueprintRevision> {
    let signal = Node::new(
        NodeId::new("signal")?,
        NodeKind::SignalWait {
            signal: OperationId::new("notify.ready")?,
        },
    )?
    .with_control_input(PortId::new("in")?)?
    .with_control_output(PortId::new("out")?)?;
    revision(
        workflow,
        vec![
            task("retired", "model.generate")?,
            signal,
            terminal("done", TerminalOutcome::Success)?,
        ],
        vec![
            control_edge("retired-signal", "retired", "out", "signal", "in")?,
            control_edge("signal-done", "signal", "out", "done", "in")?,
        ],
    )
}

pub(super) fn revision_without_completed_task(
    base: &BlueprintRevision,
) -> TestResult<BlueprintRevision> {
    Ok(base.revise(
        base.id(),
        MutationBatch::new(vec![
            Mutation::RemoveEdge {
                edge: EdgeId::new("retired-signal")?,
            },
            Mutation::RemoveNode {
                node: NodeId::new("retired")?,
            },
        ])?,
        AuthorRef::new("human:structured-runtime-test")?,
        "remove completed work without reinterpreting its history",
    )?)
}

pub(super) fn artifact_reuse_revision(workflow: &str) -> TestResult<BlueprintRevision> {
    let schema = item_schema()?;
    let task = task("reuse", "model.generate")?
        .with_data_output(PortId::new("result")?, DataPort::output(schema.clone()))?;
    revision_with_interface(
        workflow,
        WorkflowInterface::new(
            [(
                FieldId::new("initial-artifact")?,
                InterfaceField::required(schema),
            )],
            [],
        )?,
        vec![task, terminal("done", TerminalOutcome::Success)?],
        vec![control_edge("reuse-done", "reuse", "out", "done", "in")?],
    )
}

pub(super) fn direct_artifact_input_revision(
    workflow: &str,
    artifact: &milkdrift_workspace::ArtifactReference,
) -> TestResult<BlueprintRevision> {
    let schema = item_schema()?;
    let artifact_binding = BindingSource::Artifact {
        reference: serde_json::to_string(artifact)?,
        contract: schema.clone(),
    };
    let optional_binding = BindingSource::WorkflowInput {
        field: FieldId::new("optional")?,
    };
    let task = task("consume", "model.generate")?
        .with_data_input(
            PortId::new("artifact")?,
            DataPort::input(schema.clone(), true, Some(artifact_binding))?,
        )?
        .with_data_input(
            PortId::new("optional")?,
            DataPort::input(schema.clone(), false, Some(optional_binding))?,
        )?;
    revision_with_interface(
        workflow,
        WorkflowInterface::new(
            [(FieldId::new("optional")?, InterfaceField::optional(schema))],
            [],
        )?,
        vec![task, terminal("done", TerminalOutcome::Success)?],
        vec![control_edge(
            "consume-done",
            "consume",
            "out",
            "done",
            "in",
        )?],
    )
}

pub(super) fn terminal_binding_revision(workflow: &str) -> TestResult<BlueprintRevision> {
    let schema = item_schema()?;
    let terminal = Node::new(
        NodeId::new("done")?,
        NodeKind::Terminal {
            outcome: TerminalOutcome::Success,
        },
    )?
    .with_data_input(
        PortId::new("pass")?,
        DataPort::input(
            schema.clone(),
            true,
            Some(BindingSource::WorkflowInput {
                field: FieldId::new("source")?,
            }),
        )?,
    )?
    .with_data_input(
        PortId::new("literal")?,
        DataPort::input(
            schema.clone(),
            true,
            Some(BindingSource::Literal {
                value: BoundedJson::new(json!({"materialized": true}))?,
            }),
        )?,
    )?;
    revision_with_interface(
        workflow,
        WorkflowInterface::new(
            [(
                FieldId::new("source")?,
                InterfaceField::required(schema.clone()),
            )],
            [
                (
                    FieldId::new("pass")?,
                    InterfaceField::required(schema.clone()),
                ),
                (FieldId::new("literal")?, InterfaceField::required(schema)),
            ],
        )?,
        vec![terminal],
        Vec::new(),
    )
}

pub(super) fn missing_optional_condition_revision(workflow: &str) -> TestResult<BlueprintRevision> {
    let schema = item_schema()?;
    let source = BindingSource::WorkflowInput {
        field: FieldId::new("maybe")?,
    };
    let present = PortId::new("present")?;
    let missing = PortId::new("missing")?;
    let branch = Node::new(
        NodeId::new("route")?,
        NodeKind::Branch {
            config: BranchConfig::new(
                BTreeMap::from([(
                    present.clone(),
                    Condition::Exists {
                        source: source.clone(),
                    },
                )]),
                Some(missing.clone()),
            )?,
        },
    )?
    .with_control_output(present)?
    .with_control_output(missing)?
    .with_data_input(
        PortId::new("maybe")?,
        DataPort::input(schema.clone(), false, Some(source))?,
    )?;
    revision_with_interface(
        workflow,
        WorkflowInterface::new(
            [(FieldId::new("maybe")?, InterfaceField::optional(schema))],
            [],
        )?,
        vec![
            branch,
            terminal("unexpected", TerminalOutcome::Failure)?,
            terminal("expected", TerminalOutcome::Success)?,
        ],
        vec![
            control_edge("present-route", "route", "present", "unexpected", "in")?,
            control_edge("missing-route", "route", "missing", "expected", "in")?,
        ],
    )
}

pub(super) fn multi_path_condition_revision(workflow: &str) -> TestResult<BlueprintRevision> {
    let schema = item_schema()?;
    let signal = Node::new(
        NodeId::new("signal")?,
        NodeKind::SignalWait {
            signal: OperationId::new("notify.payload")?,
        },
    )?
    .with_control_output(PortId::new("out")?)?
    .with_data_output(PortId::new("payload")?, DataPort::output(schema.clone()))?;
    let left = BindingSource::NodeOutput {
        node: NodeId::new("signal")?,
        port: PortId::new("payload")?,
        path: PathSelector::new(vec![PathSegment::Field(FieldId::new("left")?)])?,
    };
    let right = BindingSource::NodeOutput {
        node: NodeId::new("signal")?,
        port: PortId::new("payload")?,
        path: PathSelector::new(vec![PathSegment::Field(FieldId::new("right")?)])?,
    };
    let expected = PortId::new("expected")?;
    let unexpected = PortId::new("unexpected")?;
    let branch = Node::new(
        NodeId::new("route")?,
        NodeKind::Branch {
            config: BranchConfig::new(
                BTreeMap::from([(
                    expected.clone(),
                    Condition::All {
                        conditions: vec![
                            Condition::Compare {
                                left: ConditionOperand::Binding {
                                    source: left.clone(),
                                },
                                comparison: Comparison::Equal,
                                right: ConditionOperand::Literal {
                                    value: BoundedJson::new(json!(1))?,
                                },
                            },
                            Condition::Compare {
                                left: ConditionOperand::Binding {
                                    source: right.clone(),
                                },
                                comparison: Comparison::Equal,
                                right: ConditionOperand::Literal {
                                    value: BoundedJson::new(json!(2))?,
                                },
                            },
                        ],
                    },
                )]),
                Some(unexpected.clone()),
            )?,
        },
    )?
    .with_control_input(PortId::new("in")?)?
    .with_control_output(expected)?
    .with_control_output(unexpected)?
    .with_data_input(
        PortId::new("left")?,
        DataPort::input(schema.clone(), true, Some(left))?,
    )?
    .with_data_input(
        PortId::new("right")?,
        DataPort::input(schema, true, Some(right))?,
    )?;
    revision(
        workflow,
        vec![
            signal,
            branch,
            terminal("good", TerminalOutcome::Success)?,
            terminal("bad", TerminalOutcome::Failure)?,
        ],
        vec![
            control_edge("signal-route", "signal", "out", "route", "in")?,
            data_edge("signal-left", "signal", "payload", "route", "left")?,
            data_edge("signal-right", "signal", "payload", "route", "right")?,
            control_edge("route-good", "route", "expected", "good", "in")?,
            control_edge("route-bad", "route", "unexpected", "bad", "in")?,
        ],
    )
}

pub(super) fn long_deterministic_chain_revision(
    workflow: &str,
    branch_count: usize,
) -> TestResult<BlueprintRevision> {
    let next = PortId::new("next")?;
    let mut nodes = Vec::with_capacity(branch_count.saturating_add(1));
    let mut edges = Vec::with_capacity(branch_count);
    for index in 0..branch_count {
        let id = format!("step-{index:04}");
        let mut node = Node::new(
            NodeId::new(id.clone())?,
            NodeKind::Branch {
                config: BranchConfig::new(
                    BTreeMap::from([(next.clone(), Condition::Constant { value: true })]),
                    None,
                )?,
            },
        )?
        .with_control_output(next.clone())?;
        if index > 0 {
            node = node.with_control_input(PortId::new("in")?)?;
            edges.push(control_edge(
                &format!("edge-{index:04}"),
                &format!("step-{:04}", index - 1),
                "next",
                &id,
                "in",
            )?);
        }
        nodes.push(node);
    }
    nodes.push(terminal("done", TerminalOutcome::Success)?);
    edges.push(control_edge(
        "edge-terminal",
        &format!("step-{:04}", branch_count - 1),
        "next",
        "done",
        "in",
    )?);
    revision(workflow, nodes, edges)
}

pub(super) fn subworkflow_revision(
    workflow: &str,
    child: &BlueprintRevision,
) -> TestResult<BlueprintRevision> {
    let mut child_node = Node::new(
        NodeId::new("child")?,
        NodeKind::Subworkflow {
            reference: PinnedSubworkflow::new(
                child.semantic().workflow().clone(),
                child.id().clone(),
                child.semantic().interface().clone(),
            ),
        },
    )?
    .with_control_output(PortId::new("out")?)?;
    for (field, declaration) in child.semantic().interface().outputs() {
        child_node = child_node.with_data_output(
            PortId::new(field.as_str())?,
            DataPort::output(declaration.schema().clone()),
        )?;
    }
    revision(
        workflow,
        vec![child_node, terminal("done", TerminalOutcome::Success)?],
        vec![control_edge("child-done", "child", "out", "done", "in")?],
    )
}

pub(super) fn repeat_revision(
    workflow: &str,
    child: &BlueprintRevision,
) -> TestResult<BlueprintRevision> {
    let mut repeat = Node::new(
        NodeId::new("repeat")?,
        NodeKind::Repeat {
            config: RepeatConfig::new(
                PinnedSubworkflow::new(
                    child.semantic().workflow().clone(),
                    child.id().clone(),
                    child.semantic().interface().clone(),
                ),
                Condition::Constant { value: true },
                2,
                RepeatBudget {
                    max_duration_ms: None,
                    max_cost_micros: None,
                    max_cost_currency: None,
                },
                RepeatTermination::SucceedWithLatest,
            )?,
        },
    )?
    .with_control_output(PortId::new("out")?)?;
    for (field, declaration) in child.semantic().interface().outputs() {
        repeat = repeat.with_data_output(
            PortId::new(field.as_str())?,
            DataPort::output(declaration.schema().clone()),
        )?;
    }
    revision(
        workflow,
        vec![repeat, terminal("done", TerminalOutcome::Success)?],
        vec![control_edge("repeat-done", "repeat", "out", "done", "in")?],
    )
}

pub(super) fn approval_repeat_revision(
    workflow: &str,
    child: &BlueprintRevision,
) -> TestResult<BlueprintRevision> {
    let repeat = Node::new(
        NodeId::new("repeat")?,
        NodeKind::Repeat {
            config: RepeatConfig::new(
                PinnedSubworkflow::new(
                    child.semantic().workflow().clone(),
                    child.id().clone(),
                    child.semantic().interface().clone(),
                ),
                Condition::Constant { value: true },
                1,
                RepeatBudget {
                    max_duration_ms: None,
                    max_cost_micros: None,
                    max_cost_currency: None,
                },
                RepeatTermination::AwaitApproval,
            )?,
        },
    )?
    .with_control_output(PortId::new("out")?)?;
    revision(
        workflow,
        vec![repeat, terminal("done", TerminalOutcome::Success)?],
        vec![control_edge("repeat-done", "repeat", "out", "done", "in")?],
    )
}

pub(super) fn revised_wait_revision(
    base: &BlueprintRevision,
    duration_ms: u64,
) -> TestResult<BlueprintRevision> {
    let wait = Node::new(NodeId::new("wait")?, NodeKind::Wait { duration_ms })?
        .with_control_output(PortId::new("out")?)?;
    Ok(base.revise(
        base.id(),
        MutationBatch::new(vec![Mutation::ReplaceNode { node: wait }])?,
        AuthorRef::new("human:structured-runtime-test")?,
        "change the prospective wait definition",
    )?)
}

pub(super) fn revision_without_entry_node(
    base: &BlueprintRevision,
    node: &str,
    incident_edges: &[&str],
) -> TestResult<BlueprintRevision> {
    let mut mutations = incident_edges
        .iter()
        .map(|edge| {
            Ok(Mutation::RemoveEdge {
                edge: EdgeId::new((*edge).to_owned())?,
            })
        })
        .collect::<TestResult<Vec<_>>>()?;
    mutations.push(Mutation::RemoveNode {
        node: NodeId::new(node.to_owned())?,
    });
    Ok(base.revise(
        base.id(),
        MutationBatch::new(mutations)?,
        AuthorRef::new("human:structured-runtime-test")?,
        "remove a prospective entry without rewriting its runtime-owned history",
    )?)
}

pub(super) fn revision_with_added_root_wait(
    base: &BlueprintRevision,
    duration_ms: u64,
) -> TestResult<BlueprintRevision> {
    let prior_duration = match base
        .semantic()
        .nodes()
        .get(&NodeId::new("wait")?)
        .map(Node::kind)
    {
        Some(NodeKind::Wait { duration_ms }) => *duration_ms,
        _ => return Err("base revision has no wait node".into()),
    };
    let node = Node::new(NodeId::new("added-root")?, NodeKind::Wait { duration_ms })?
        .with_control_output(PortId::new("out")?)?;
    let wait = Node::new(
        NodeId::new("wait")?,
        NodeKind::Wait {
            duration_ms: prior_duration,
        },
    )?
    .with_control_input(PortId::new("in")?)?
    .with_control_output(PortId::new("out")?)?;
    Ok(base.revise(
        base.id(),
        MutationBatch::new(vec![
            Mutation::AddNode { node },
            Mutation::ReplaceNode { node: wait },
            Mutation::AddEdge {
                edge: control_edge("added-root-wait", "added-root", "out", "wait", "in")?,
            },
        ])?,
        AuthorRef::new("human:structured-runtime-test")?,
        "add an independent root entry node",
    )?)
}

pub(super) fn revised_task_revision(
    base: &BlueprintRevision,
    operation: &str,
) -> TestResult<BlueprintRevision> {
    Ok(base.revise(
        base.id(),
        MutationBatch::new(vec![Mutation::ReplaceNode {
            node: task("work", operation)?,
        }])?,
        AuthorRef::new("human:structured-runtime-test")?,
        "change an active task's prospective capability",
    )?)
}
