use super::*;

pub(super) fn generous_budget() -> TestResult<WorkspaceBudget> {
    Ok(WorkspaceBudget::new(
        2_048,
        8 * 1_024 * 1_024,
        16 * 1_024 * 1_024,
        256,
        64 * 1_024 * 1_024,
        128 * 1_024 * 1_024,
    )?)
}

pub(super) fn test_descriptor() -> TestResult<CapabilityDescriptor> {
    descriptor_with_model_side_effect("none")
}

pub(super) fn descriptor_with_model_side_effect(
    side_effect: &str,
) -> TestResult<CapabilityDescriptor> {
    let mut value: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../capability/tests/fixtures/descriptor-v1.json"
    ))?;
    let operations = value
        .get_mut("descriptor")
        .and_then(|descriptor| descriptor.get_mut("operations"))
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("descriptor fixture has no operations object")?;
    operations
        .get_mut("model.generate")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("descriptor fixture has no model.generate operation")?
        .insert(
            "side_effect".to_owned(),
            serde_json::Value::String(side_effect.to_owned()),
        );
    let template = operations
        .get("model.generate")
        .cloned()
        .ok_or("descriptor fixture has no model.generate operation")?;
    operations.insert("model.fail".to_owned(), template);
    Ok(
        CapabilityDescriptorDocument::from_json(&serde_json::to_vec(&value)?)?
            .body()
            .clone(),
    )
}

pub(super) fn empty_interface() -> TestResult<WorkflowInterface> {
    Ok(WorkflowInterface::new([], [])?)
}

pub(super) fn revision(
    workflow: &str,
    nodes: Vec<Node>,
    edges: Vec<Edge>,
) -> TestResult<BlueprintRevision> {
    revision_with_interface(workflow, empty_interface()?, nodes, edges)
}

pub(super) fn revision_with_interface(
    workflow: &str,
    interface: WorkflowInterface,
    nodes: Vec<Node>,
    edges: Vec<Edge>,
) -> TestResult<BlueprintRevision> {
    let mut operations = vec![Mutation::SetInterface { interface }];
    operations.extend(nodes.into_iter().map(|node| Mutation::AddNode { node }));
    operations.extend(edges.into_iter().map(|edge| Mutation::AddEdge { edge }));
    Ok(BlueprintRevision::genesis(
        WorkflowId::new(workflow)?,
        MutationBatch::new(operations)?,
        AuthorRef::new("human:structured-runtime-test")?,
        "structured runtime integration fixture",
    )?)
}

pub(super) fn control_edge(
    id: &str,
    source: &str,
    source_port: &str,
    target: &str,
    target_port: &str,
) -> TestResult<Edge> {
    Ok(Edge::new(
        EdgeId::new(id)?,
        EdgeKind::Control,
        NodeId::new(source)?,
        PortId::new(source_port)?,
        NodeId::new(target)?,
        PortId::new(target_port)?,
    ))
}

pub(super) fn data_edge(
    id: &str,
    source: &str,
    source_port: &str,
    target: &str,
    target_port: &str,
) -> TestResult<Edge> {
    Ok(Edge::new(
        EdgeId::new(id)?,
        EdgeKind::Data,
        NodeId::new(source)?,
        PortId::new(source_port)?,
        NodeId::new(target)?,
        PortId::new(target_port)?,
    ))
}

pub(super) fn terminal(id: &str, outcome: TerminalOutcome) -> TestResult<Node> {
    Ok(Node::new(NodeId::new(id)?, NodeKind::Terminal { outcome })?
        .with_control_input(PortId::new("in")?)?)
}

pub(super) fn task(id: &str, operation: &str) -> TestResult<Node> {
    Ok(Node::new(
        NodeId::new(id)?,
        NodeKind::task_direct_inputs(CapabilityRequirement::new(OperationId::new(operation)?))?,
    )?
    .with_control_input(PortId::new("in")?)?
    .with_control_output(PortId::new("out")?)?)
}

pub(super) fn successful_terminal() -> TestResult<InvocationTerminal> {
    Ok(InvocationTerminal::new(
        TerminalStatus::Success,
        Vec::new(),
        None,
        None,
        SideEffectClass::None,
    )?)
}

pub(super) fn failed_terminal() -> TestResult<InvocationTerminal> {
    Ok(InvocationTerminal::new(
        TerminalStatus::Failure,
        Vec::new(),
        Some(InvocationFailure::new(
            ErrorClass::Provider,
            false,
            "scripted_failure",
            "deterministic branch failure",
            None,
        )?),
        None,
        SideEffectClass::None,
    )?)
}

pub(super) fn branch_revision(workflow: &str) -> TestResult<BlueprintRevision> {
    let true_port = PortId::new("true")?;
    let false_port = PortId::new("false")?;
    let branch = Node::new(
        NodeId::new("route")?,
        NodeKind::Branch {
            config: BranchConfig::new(
                BTreeMap::from([(true_port.clone(), Condition::Constant { value: true })]),
                Some(false_port.clone()),
            )?,
        },
    )?
    .with_control_output(true_port)?
    .with_control_output(false_port)?;
    revision(
        workflow,
        vec![
            branch,
            terminal("selected", TerminalOutcome::Success)?,
            terminal("unselected", TerminalOutcome::Failure)?,
        ],
        vec![
            control_edge("route-selected", "route", "true", "selected", "in")?,
            control_edge("route-unselected", "route", "false", "unselected", "in")?,
        ],
    )
}

pub(super) fn optional_unselected_edge_revision(workflow: &str) -> TestResult<BlueprintRevision> {
    let schema = item_schema()?;
    let selected = PortId::new("selected")?;
    let unselected = PortId::new("unselected")?;
    let branch = Node::new(
        NodeId::new("route")?,
        NodeKind::Branch {
            config: BranchConfig::new(
                BTreeMap::from([(selected.clone(), Condition::Constant { value: true })]),
                Some(unselected.clone()),
            )?,
        },
    )?
    .with_control_output(selected)?
    .with_control_output(unselected)?;
    let consume = task("consume", "model.generate")?.with_data_input(
        PortId::new("optional")?,
        DataPort::input(schema.clone(), false, None)?,
    )?;
    let produce = task("produce", "model.generate")?
        .with_data_output(PortId::new("item")?, DataPort::output(schema))?;
    revision(
        workflow,
        vec![
            branch,
            consume,
            produce,
            terminal("done", TerminalOutcome::Success)?,
            terminal("unused", TerminalOutcome::Failure)?,
        ],
        vec![
            control_edge("route-consume", "route", "selected", "consume", "in")?,
            control_edge("route-produce", "route", "unselected", "produce", "in")?,
            control_edge("consume-done", "consume", "out", "done", "in")?,
            control_edge("produce-unused", "produce", "out", "unused", "in")?,
            data_edge("optional-item", "produce", "item", "consume", "optional")?,
        ],
    )
}

pub(super) fn fork_revision(
    workflow: &str,
    policy: JoinPolicy,
    second_operation: &str,
) -> TestResult<BlueprintRevision> {
    fork_revision_with_terminal(workflow, policy, second_operation, TerminalOutcome::Success)
}

pub(super) fn fork_revision_with_terminal(
    workflow: &str,
    policy: JoinPolicy,
    second_operation: &str,
    terminal_outcome: TerminalOutcome,
) -> TestResult<BlueprintRevision> {
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
    let join = Node::new(
        NodeId::new("join")?,
        NodeKind::Join {
            config: JoinConfig::new(NodeId::new("fork")?, policy),
        },
    )?
    .with_control_input(PortId::new("a-in")?)?
    .with_control_input(PortId::new("b-in")?)?
    .with_control_output(PortId::new("out")?)?;
    revision(
        workflow,
        vec![
            fork,
            task("a-task", "model.generate")?,
            task("b-task", second_operation)?,
            join,
            terminal("done", terminal_outcome)?,
        ],
        vec![
            control_edge("fork-a", "fork", "a", "a-task", "in")?,
            control_edge("fork-b", "fork", "b", "b-task", "in")?,
            control_edge("a-join", "a-task", "out", "join", "a-in")?,
            control_edge("b-join", "b-task", "out", "join", "b-in")?,
            control_edge("join-done", "join", "out", "done", "in")?,
        ],
    )
}

pub(super) fn nested_fork_revision(workflow: &str) -> TestResult<BlueprintRevision> {
    let schema = item_schema()?;
    let outer_a = PortId::new("a")?;
    let outer_b = PortId::new("b")?;
    let inner_left = PortId::new("left")?;
    let inner_right = PortId::new("right")?;
    let outer_fork = Node::new(
        NodeId::new("outer-fork")?,
        NodeKind::Fork {
            config: ForkConfig::new(BTreeSet::from([outer_a.clone(), outer_b.clone()]))?,
        },
    )?
    .with_control_output(outer_a)?
    .with_control_output(outer_b)?;
    let inner_fork = Node::new(
        NodeId::new("inner-fork")?,
        NodeKind::Fork {
            config: ForkConfig::new(BTreeSet::from([inner_left.clone(), inner_right.clone()]))?,
        },
    )?
    .with_control_input(PortId::new("in")?)?
    .with_control_output(inner_left)?
    .with_control_output(inner_right)?;
    let inner_left_task = task("inner-left", "model.generate")?
        .with_data_output(PortId::new("item")?, DataPort::output(schema.clone()))?;
    let inner_right_task = task("inner-right", "model.fail")?
        .with_data_output(PortId::new("item")?, DataPort::output(schema.clone()))?;
    let inner_join = Node::new(
        NodeId::new("inner-join")?,
        NodeKind::Join {
            config: JoinConfig::new(NodeId::new("inner-fork")?, JoinPolicy::All),
        },
    )?
    .with_control_input(PortId::new("left-in")?)?
    .with_control_input(PortId::new("right-in")?)?
    .with_control_output(PortId::new("out")?)?;
    let outer_a_tail = task("outer-a-tail", "model.generate")?
        .with_data_output(PortId::new("item")?, DataPort::output(schema.clone()))?;
    let outer_b_task = task("outer-b-task", "model.fail")?
        .with_data_output(PortId::new("item")?, DataPort::output(schema.clone()))?;
    let outer_join = Node::new(
        NodeId::new("outer-join")?,
        NodeKind::Join {
            config: JoinConfig::new(NodeId::new("outer-fork")?, JoinPolicy::All),
        },
    )?
    .with_control_input(PortId::new("a-in")?)?
    .with_control_input(PortId::new("b-in")?)?
    .with_control_output(PortId::new("out")?)?
    .with_data_input(
        PortId::new("tail-item")?,
        DataPort::input(schema, false, None)?,
    )?;
    revision(
        workflow,
        vec![
            outer_fork,
            inner_fork,
            inner_left_task,
            inner_right_task,
            inner_join,
            outer_a_tail,
            outer_b_task,
            outer_join,
            terminal("done", TerminalOutcome::Success)?,
        ],
        vec![
            control_edge("outer-fork-a", "outer-fork", "a", "inner-fork", "in")?,
            control_edge("outer-fork-b", "outer-fork", "b", "outer-b-task", "in")?,
            control_edge("inner-fork-left", "inner-fork", "left", "inner-left", "in")?,
            control_edge(
                "inner-fork-right",
                "inner-fork",
                "right",
                "inner-right",
                "in",
            )?,
            control_edge(
                "inner-left-join",
                "inner-left",
                "out",
                "inner-join",
                "left-in",
            )?,
            control_edge(
                "inner-right-join",
                "inner-right",
                "out",
                "inner-join",
                "right-in",
            )?,
            control_edge("inner-join-tail", "inner-join", "out", "outer-a-tail", "in")?,
            control_edge("outer-a-join", "outer-a-tail", "out", "outer-join", "a-in")?,
            control_edge("outer-b-join", "outer-b-task", "out", "outer-join", "b-in")?,
            data_edge(
                "outer-tail-data",
                "outer-a-tail",
                "item",
                "outer-join",
                "tail-item",
            )?,
            control_edge("outer-join-done", "outer-join", "out", "done", "in")?,
        ],
    )
}

pub(super) fn direct_terminal_fork_revision(
    workflow: &str,
    a_outcome: TerminalOutcome,
    b_outcome: TerminalOutcome,
) -> TestResult<BlueprintRevision> {
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
    revision(
        workflow,
        vec![
            fork,
            terminal("a-terminal", a_outcome)?,
            terminal("b-terminal", b_outcome)?,
        ],
        vec![
            control_edge("fork-a-terminal", "fork", "a", "a-terminal", "in")?,
            control_edge("fork-b-terminal", "fork", "b", "b-terminal", "in")?,
        ],
    )
}

pub(super) fn fork_revision_with_post_join_task(workflow: &str) -> TestResult<BlueprintRevision> {
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
    let join = Node::new(
        NodeId::new("join")?,
        NodeKind::Join {
            config: JoinConfig::new(NodeId::new("fork")?, JoinPolicy::Any),
        },
    )?
    .with_control_input(PortId::new("a-in")?)?
    .with_control_input(PortId::new("b-in")?)?
    .with_control_output(PortId::new("out")?)?;
    revision(
        workflow,
        vec![
            fork,
            task("a-task", "model.generate")?,
            task("b-task", "model.fail")?,
            join,
            task("independent", "model.generate")?,
            terminal("done", TerminalOutcome::Success)?,
        ],
        vec![
            control_edge("fork-a", "fork", "a", "a-task", "in")?,
            control_edge("fork-b", "fork", "b", "b-task", "in")?,
            control_edge("a-join", "a-task", "out", "join", "a-in")?,
            control_edge("b-join", "b-task", "out", "join", "b-in")?,
            control_edge("join-independent", "join", "out", "independent", "in")?,
            control_edge("independent-done", "independent", "out", "done", "in")?,
        ],
    )
}

pub(super) fn revision_without_post_join_task(
    base: &BlueprintRevision,
) -> TestResult<BlueprintRevision> {
    Ok(base.revise(
        base.id(),
        MutationBatch::new(vec![
            Mutation::RemoveEdge {
                edge: EdgeId::new("join-independent")?,
            },
            Mutation::RemoveEdge {
                edge: EdgeId::new("independent-done")?,
            },
            Mutation::RemoveNode {
                node: NodeId::new("independent")?,
            },
            Mutation::AddEdge {
                edge: control_edge("join-done", "join", "out", "done", "in")?,
            },
        ])?,
        AuthorRef::new("human:structured-runtime-test")?,
        "remove an actually unstarted post-join task",
    )?)
}

pub(super) fn item_schema() -> TestResult<SchemaRef> {
    Ok(SchemaRef::new(SchemaId::new("milkdrift.test.item")?, 1)?)
}

pub(super) fn reducer_revision(
    workflow: &str,
    strategy: ReducerStrategy,
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
    let a_task = task("a-task", "model.generate")?
        .with_data_output(PortId::new("item")?, DataPort::output(schema.clone()))?;
    let b_task = task("b-task", "model.fail")?
        .with_data_output(PortId::new("item")?, DataPort::output(schema.clone()))?;
    let join = Node::new(
        NodeId::new("join")?,
        NodeKind::Join {
            config: JoinConfig::new(NodeId::new("fork")?, JoinPolicy::All),
        },
    )?
    .with_control_input(PortId::new("a-in")?)?
    .with_control_input(PortId::new("b-in")?)?
    .with_control_output(PortId::new("out")?)?;
    let reducer = Node::new(
        NodeId::new("reduce")?,
        NodeKind::Reducer {
            config: ReducerConfig::new(PortId::new("items")?, schema.clone(), 2, strategy)?,
        },
    )?
    .with_control_input(PortId::new("in")?)?
    .with_control_output(PortId::new("out")?)?
    .with_data_input(
        PortId::new("items")?,
        DataPort::input(schema.clone(), true, None)?,
    )?
    .with_data_output(PortId::new("reduced")?, DataPort::output(schema))?;
    revision(
        workflow,
        vec![
            fork,
            a_task,
            b_task,
            join,
            reducer,
            terminal("done", TerminalOutcome::Success)?,
        ],
        vec![
            control_edge("fork-a", "fork", "a", "a-task", "in")?,
            control_edge("fork-b", "fork", "b", "b-task", "in")?,
            control_edge("a-join", "a-task", "out", "join", "a-in")?,
            control_edge("b-join", "b-task", "out", "join", "b-in")?,
            control_edge("join-reduce", "join", "out", "reduce", "in")?,
            control_edge("reduce-done", "reduce", "out", "done", "in")?,
            data_edge("a-reduce", "a-task", "item", "reduce", "items")?,
            data_edge("b-reduce", "b-task", "item", "reduce", "items")?,
        ],
    )
}

pub(super) fn publish_artifact(
    harness: &Harness,
    suffix: &str,
    bytes: &[u8],
) -> TestResult<InvocationArtifactReference> {
    publish_artifact_in_store(
        harness.store.as_ref(),
        &RunId::new(format!("artifact-publisher-{suffix}"))?,
        suffix,
        bytes,
    )
}

pub(super) fn publish_artifact_in_store(
    store: &RedbStore,
    owner: &RunId,
    suffix: &str,
    bytes: &[u8],
) -> TestResult<InvocationArtifactReference> {
    let digest = ContentDigest::for_bytes(bytes);
    let artifact = ArtifactId::new(format!("artifact-{suffix}"))?;
    let reference = milkdrift_workspace::ArtifactReference::new(
        artifact,
        digest,
        MediaType::new("application/octet-stream")?,
        u64::try_from(bytes.len())?,
    );
    let metadata = ArtifactMetadata::new(
        reference,
        ArtifactSensitivity::Public,
        ArtifactRetention::WhileReferenced,
        ArtifactProvenance::new(
            CausalReference::External {
                source: CausalId::new(format!("source-{suffix}"))?,
            },
            Vec::new(),
        )?,
    )?;
    let publication = ArtifactPublicationId::new(format!("publication-{suffix}"))?;
    let request = BeginArtifactPublication::new(
        publication.clone(),
        owner.clone(),
        metadata.clone(),
        generous_budget()?,
        WorkspaceUsage::EMPTY,
    )?;
    store.begin_publication(&request)?;
    store.write_chunk(&publication, 0, bytes)?;
    store.commit_publication(&publication)?;
    Ok(InvocationArtifactReference::new(
        metadata.reference().artifact().as_str(),
        digest.to_hex(),
        Some("application/octet-stream".to_owned()),
        Some(u64::try_from(bytes.len())?),
    )?)
}

pub(super) fn publish_artifact_for_run(
    harness: &Harness,
    run: &RunId,
    suffix: &str,
    bytes: &[u8],
) -> TestResult<milkdrift_workspace::ArtifactReference> {
    let digest = ContentDigest::for_bytes(bytes);
    let reference = milkdrift_workspace::ArtifactReference::new(
        ArtifactId::new(format!("artifact-{suffix}"))?,
        digest,
        MediaType::new("application/octet-stream")?,
        u64::try_from(bytes.len())?,
    );
    let metadata = ArtifactMetadata::new(
        reference.clone(),
        ArtifactSensitivity::Public,
        ArtifactRetention::WhileReferenced,
        ArtifactProvenance::new(
            CausalReference::External {
                source: CausalId::new(format!("source-{suffix}"))?,
            },
            Vec::new(),
        )?,
    )?;
    let publication = ArtifactPublicationId::new(format!("publication-{suffix}"))?;
    harness
        .store
        .begin_publication(&BeginArtifactPublication::new(
            publication.clone(),
            run.clone(),
            metadata,
            generous_budget()?,
            WorkspaceUsage::EMPTY,
        )?)?;
    harness.store.write_chunk(&publication, 0, bytes)?;
    harness.store.commit_publication(&publication)?;
    Ok(reference)
}

pub(super) fn install_output_scripts(harness: &Harness) -> TestResult {
    let a = publish_artifact(harness, "z-branch-a", b"artifact-a")?;
    let b = publish_artifact(harness, "a-branch-b", b"artifact-b")?;
    harness.executor.set_script(
        OperationId::new("model.generate")?,
        vec![
            InvocationEventKind::Output {
                name: "item".to_owned(),
                reference: a,
            },
            InvocationEventKind::Terminal {
                terminal: successful_terminal()?,
            },
        ],
    )?;
    harness.executor.set_script(
        OperationId::new("model.fail")?,
        vec![
            InvocationEventKind::Output {
                name: "item".to_owned(),
                reference: b,
            },
            InvocationEventKind::Terminal {
                terminal: successful_terminal()?,
            },
        ],
    )?;
    Ok(())
}

pub(super) fn install_child_output_script(harness: &Harness) -> TestResult {
    let artifact = publish_artifact(harness, "child-result", b"child-result")?;
    harness.executor.set_script(
        OperationId::new("model.generate")?,
        vec![
            InvocationEventKind::Output {
                name: "result".to_owned(),
                reference: artifact,
            },
            InvocationEventKind::Terminal {
                terminal: successful_terminal()?,
            },
        ],
    )?;
    Ok(())
}

pub(super) fn install_non_idempotent_success_script(harness: &Harness) -> TestResult {
    harness.executor.set_script(
        OperationId::new("model.generate")?,
        vec![InvocationEventKind::Terminal {
            terminal: InvocationTerminal::new(
                TerminalStatus::Success,
                Vec::new(),
                None,
                None,
                SideEffectClass::NonIdempotentWrite,
            )?,
        }],
    )?;
    Ok(())
}
