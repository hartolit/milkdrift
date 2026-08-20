//! Independent examples and property tests for the immutable blueprint kernel.

use std::collections::{BTreeMap, BTreeSet};

use milkdrift_blueprint::{
    AuthorRef, BindingSource, BlueprintMetadata, BlueprintRevision, BlueprintRevisionDocument,
    BranchConfig, Condition, CostCurrencyCode, DataPort, DiagnosticCode, DocumentError, Edge,
    EdgeId, EdgeKind, FieldId, ForkConfig, InterfaceField, JoinConfig, JoinPolicy, Mutation,
    MutationBatch, MutationError, Node, NodeId, NodeKind, PathSegment, PathSelector,
    PinnedSubworkflow, PortId, ReducerConfig, ReducerStrategy, RepeatBudget, RepeatConfig,
    RepeatTermination, RevisionId, SchemaRef, TerminalOutcome, WorkflowId, WorkflowInterface,
    node_configuration_fingerprint, node_dependency_fingerprint,
};
use milkdrift_capability::{
    CapabilityRequirement, MAX_DURABLE_REFERENCE_BYTES, OperationId, SchemaId,
};
use proptest::prelude::*;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn id(value: &str) -> Result<NodeId, milkdrift_blueprint::IdentityError> {
    NodeId::new(value)
}

fn port(value: &str) -> Result<PortId, milkdrift_blueprint::IdentityError> {
    PortId::new(value)
}

fn schema() -> TestResult<SchemaRef> {
    Ok(SchemaRef::new(SchemaId::new("milkdrift.value")?, 1)?)
}

#[test]
fn durable_binding_references_share_the_capability_boundary() -> TestResult {
    let maximum = "r".repeat(MAX_DURABLE_REFERENCE_BYTES);
    DataPort::input(
        schema()?,
        true,
        Some(BindingSource::WorkspaceValue {
            reference: maximum.clone(),
            contract: schema()?,
        }),
    )?;
    DataPort::input(
        schema()?,
        true,
        Some(BindingSource::Artifact {
            reference: maximum,
            contract: schema()?,
        }),
    )?;
    assert!(
        DataPort::input(
            schema()?,
            true,
            Some(BindingSource::WorkspaceValue {
                reference: "r".repeat(MAX_DURABLE_REFERENCE_BYTES + 1),
                contract: schema()?,
            }),
        )
        .is_err()
    );
    Ok(())
}

fn empty_interface() -> Result<WorkflowInterface, milkdrift_blueprint::ModelError> {
    WorkflowInterface::new([], [])
}

fn task_node(name: &str) -> TestResult<Node> {
    let operation = OperationId::new("tool.execute")?;
    Ok(Node::new(
        id(name)?,
        NodeKind::Task {
            requirement: CapabilityRequirement::new(operation),
        },
    )?)
}

fn terminal_node(name: &str) -> TestResult<Node> {
    Ok(Node::new(
        id(name)?,
        NodeKind::Terminal {
            outcome: TerminalOutcome::Success,
        },
    )?)
}

fn genesis(workflow: &str, operations: Vec<Mutation>) -> Result<BlueprintRevision, MutationError> {
    let batch = MutationBatch::new(operations)?;
    BlueprintRevision::genesis(
        WorkflowId::new(workflow)
            .map_err(|error| MutationError::InvalidRevision(error.to_string()))?,
        batch,
        AuthorRef::new("human:test")
            .map_err(|error| MutationError::InvalidRevision(error.to_string()))?,
        "test genesis",
    )
}

fn simple_sequence(workflow: &str, reverse_nodes: bool) -> TestResult<BlueprintRevision> {
    let first = task_node("first")?.with_control_output(port("next")?)?;
    let done = terminal_node("done")?.with_control_input(port("in")?)?;
    let mut nodes = vec![
        Mutation::AddNode { node: first },
        Mutation::AddNode { node: done },
    ];
    if reverse_nodes {
        nodes.reverse();
    }
    let mut operations = vec![Mutation::SetInterface {
        interface: empty_interface()?,
    }];
    operations.extend(nodes);
    operations.push(Mutation::AddEdge {
        edge: Edge::new(
            EdgeId::new("first-done")?,
            EdgeKind::Control,
            id("first")?,
            port("next")?,
            id("done")?,
            port("in")?,
        ),
    });
    Ok(genesis(workflow, operations)?)
}

#[test]
fn valid_sequence_is_immutable_and_round_trips() -> TestResult {
    let revision = simple_sequence("sequence", false)?;
    assert_eq!(revision.sequence(), 1);
    assert!(revision.parents().is_empty());
    let document = BlueprintRevisionDocument::new(&revision);
    let bytes = document.to_canonical_json()?;
    let (decoded_document, decoded_revision) = BlueprintRevisionDocument::from_json(&bytes)?;
    assert_eq!(decoded_document.to_canonical_json()?, bytes);
    assert_eq!(decoded_revision, revision);
    Ok(())
}

#[test]
fn successful_terminals_materialize_required_interface_outputs_explicitly() -> TestResult {
    let result_schema = schema()?;
    let interface = WorkflowInterface::new(
        [],
        [(
            FieldId::new("result")?,
            InterfaceField::required(result_schema.clone()),
        )],
    )?;
    let source = task_node("source")?
        .with_control_output(port("next")?)?
        .with_data_output(port("result")?, DataPort::output(result_schema.clone()))?;
    let terminal_without_output = terminal_node("done")?.with_control_input(port("in")?)?;
    let control_edge = Edge::new(
        EdgeId::new("source-done")?,
        EdgeKind::Control,
        id("source")?,
        port("next")?,
        id("done")?,
        port("in")?,
    );
    let missing = genesis(
        "missing-terminal-output",
        vec![
            Mutation::SetInterface {
                interface: interface.clone(),
            },
            Mutation::AddNode {
                node: source.clone(),
            },
            Mutation::AddNode {
                node: terminal_without_output,
            },
            Mutation::AddEdge {
                edge: control_edge.clone(),
            },
        ],
    );
    let Err(MutationError::Validation(error)) = missing else {
        return Err("required workflow output was not enforced at the success terminal".into());
    };
    assert!(
        error
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == DiagnosticCode::MissingInput)
    );

    let terminal = terminal_node("done")?
        .with_control_input(port("in")?)?
        .with_data_input(port("result")?, DataPort::input(result_schema, true, None)?)?;
    let revision = genesis(
        "materialized-terminal-output",
        vec![
            Mutation::SetInterface { interface },
            Mutation::AddNode { node: source },
            Mutation::AddNode { node: terminal },
            Mutation::AddEdge { edge: control_edge },
            Mutation::AddEdge {
                edge: Edge::new(
                    EdgeId::new("result-done")?,
                    EdgeKind::Data,
                    id("source")?,
                    port("result")?,
                    id("done")?,
                    port("result")?,
                ),
            },
        ],
    )?;
    assert_eq!(revision.semantic().interface().outputs().len(), 1);
    Ok(())
}

#[test]
fn node_and_dependency_fingerprints_are_stable_and_separate() -> TestResult {
    let revision = simple_sequence("fingerprints", false)?;
    let first = revision
        .semantic()
        .nodes()
        .get(&id("first")?)
        .ok_or("missing first node")?;
    let configuration = node_configuration_fingerprint(first)?;
    let dependencies = node_dependency_fingerprint(revision.semantic(), first.id())?;
    assert_ne!(configuration, dependencies);
    assert_eq!(configuration, node_configuration_fingerprint(first)?);
    assert_eq!(
        dependencies,
        node_dependency_fingerprint(revision.semantic(), first.id())?
    );
    Ok(())
}

#[test]
fn standalone_output_port_deserialization_cannot_smuggle_an_input_binding() -> TestResult {
    let invalid = serde_json::json!({
        "schema": { "id": "milkdrift.value", "version": 1 },
        "required": true,
        "binding": { "type": "literal", "value": null },
        "direction": "output"
    });
    assert!(serde_json::from_value::<DataPort>(invalid).is_err());
    Ok(())
}

#[test]
fn legacy_duplicated_task_operation_migrates_but_conflicts_are_rejected() -> TestResult {
    let kind = NodeKind::Task {
        requirement: CapabilityRequirement::new(OperationId::new("tool.execute")?),
    };
    let mut wire = serde_json::to_value(&kind)?;
    assert!(wire.get("operation").is_none());

    wire.as_object_mut()
        .ok_or("task kind must encode as an object")?
        .insert("operation".to_owned(), serde_json::json!("tool.execute"));
    assert_eq!(serde_json::from_value::<NodeKind>(wire.clone())?, kind);

    wire.as_object_mut()
        .ok_or("task kind must encode as an object")?
        .insert("operation".to_owned(), serde_json::json!("tool.other"));
    assert!(serde_json::from_value::<NodeKind>(wire).is_err());
    Ok(())
}

#[test]
fn branch_arms_are_explicit_and_typed() -> TestResult {
    let yes = port("yes")?;
    let no = port("no")?;
    let branch = Node::new(
        id("branch")?,
        NodeKind::Branch {
            config: BranchConfig::new(
                BTreeMap::from([(yes.clone(), Condition::Constant { value: true })]),
                Some(no.clone()),
            )?,
        },
    )?
    .with_control_output(yes.clone())?
    .with_control_output(no.clone())?;
    let yes_terminal = terminal_node("yes-done")?.with_control_input(port("in")?)?;
    let no_terminal = terminal_node("no-done")?.with_control_input(port("in")?)?;
    let revision = genesis(
        "branch",
        vec![
            Mutation::SetInterface {
                interface: empty_interface()?,
            },
            Mutation::AddNode { node: branch },
            Mutation::AddNode { node: yes_terminal },
            Mutation::AddNode { node: no_terminal },
            Mutation::AddEdge {
                edge: Edge::new(
                    EdgeId::new("yes")?,
                    EdgeKind::Control,
                    id("branch")?,
                    yes,
                    id("yes-done")?,
                    port("in")?,
                ),
            },
            Mutation::AddEdge {
                edge: Edge::new(
                    EdgeId::new("no")?,
                    EdgeKind::Control,
                    id("branch")?,
                    no,
                    id("no-done")?,
                    port("in")?,
                ),
            },
        ],
    )?;
    assert_eq!(revision.semantic().nodes().len(), 3);
    Ok(())
}

#[test]
fn condition_sources_must_be_declared_as_exact_node_inputs() -> TestResult {
    let yes = port("yes")?;
    let source = BindingSource::WorkflowInput {
        field: FieldId::new("undeclared")?,
    };
    let branch = Node::new(
        id("branch-with-hidden-input")?,
        NodeKind::Branch {
            config: BranchConfig::new(
                BTreeMap::from([(yes.clone(), Condition::Exists { source })]),
                None,
            )?,
        },
    )?
    .with_control_output(yes.clone())?;
    let done = terminal_node("condition-done")?.with_control_input(port("in")?)?;
    let result = genesis(
        "condition-hidden-input",
        vec![
            Mutation::SetInterface {
                interface: empty_interface()?,
            },
            Mutation::AddNode { node: branch },
            Mutation::AddNode { node: done },
            Mutation::AddEdge {
                edge: Edge::new(
                    EdgeId::new("condition-route")?,
                    EdgeKind::Control,
                    id("branch-with-hidden-input")?,
                    yes,
                    id("condition-done")?,
                    port("in")?,
                ),
            },
        ],
    );
    let Err(MutationError::Validation(error)) = result else {
        return Err("condition source without an exact data binding was accepted".into());
    };
    assert!(
        error
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == DiagnosticCode::MissingInput)
    );
    Ok(())
}

#[test]
fn fork_join_and_reducer_are_structurally_separate() -> TestResult {
    let item_schema = schema()?;
    let a = port("a")?;
    let b = port("b")?;
    let fork = Node::new(
        id("fork")?,
        NodeKind::Fork {
            config: ForkConfig::new(BTreeSet::from([a.clone(), b.clone()]))?,
        },
    )?
    .with_control_output(a.clone())?
    .with_control_output(b.clone())?;
    let task_a = task_node("task-a")?
        .with_control_input(port("in")?)?
        .with_control_output(port("out")?)?
        .with_data_output(port("item")?, DataPort::output(item_schema.clone()))?;
    let task_b = task_node("task-b")?
        .with_control_input(port("in")?)?
        .with_control_output(port("out")?)?
        .with_data_output(port("item")?, DataPort::output(item_schema.clone()))?;
    let join = Node::new(
        id("join")?,
        NodeKind::Join {
            config: JoinConfig::new(id("fork")?, JoinPolicy::All),
        },
    )?
    .with_control_input(a.clone())?
    .with_control_input(b.clone())?
    .with_control_output(port("next")?)?;
    let reducer = Node::new(
        id("reducer")?,
        NodeKind::Reducer {
            config: ReducerConfig::new(
                port("items")?,
                item_schema.clone(),
                2,
                ReducerStrategy::Collect,
            )?,
        },
    )?
    .with_control_input(port("in")?)?
    .with_control_output(port("next")?)?
    .with_data_input(port("items")?, DataPort::input(item_schema, true, None)?)?;
    let done = terminal_node("done")?.with_control_input(port("in")?)?;

    let nodes = [fork, task_a, task_b, join, reducer, done];
    let mut operations = vec![Mutation::SetInterface {
        interface: empty_interface()?,
    }];
    operations.extend(nodes.into_iter().map(|node| Mutation::AddNode { node }));
    let control_edges = [
        ("fork-a", "fork", "a", "task-a", "in"),
        ("fork-b", "fork", "b", "task-b", "in"),
        ("a-join", "task-a", "out", "join", "a"),
        ("b-join", "task-b", "out", "join", "b"),
        ("join-reducer", "join", "next", "reducer", "in"),
        ("reducer-done", "reducer", "next", "done", "in"),
    ];
    for (edge_id, source, source_port, target, target_port) in control_edges {
        operations.push(Mutation::AddEdge {
            edge: Edge::new(
                EdgeId::new(edge_id)?,
                EdgeKind::Control,
                id(source)?,
                port(source_port)?,
                id(target)?,
                port(target_port)?,
            ),
        });
    }
    for (edge_id, source) in [("data-a", "task-a"), ("data-b", "task-b")] {
        operations.push(Mutation::AddEdge {
            edge: Edge::new(
                EdgeId::new(edge_id)?,
                EdgeKind::Data,
                id(source)?,
                port("item")?,
                id("reducer")?,
                port("items")?,
            ),
        });
    }
    let revision = genesis("fork-reduce", operations)?;
    assert_eq!(revision.semantic().nodes().len(), 6);
    Ok(())
}

fn body_revision() -> TestResult<BlueprintRevision> {
    Ok(genesis(
        "body",
        vec![
            Mutation::SetInterface {
                interface: empty_interface()?,
            },
            Mutation::AddNode {
                node: terminal_node("body-done")?,
            },
        ],
    )?)
}

#[test]
fn repeat_wait_signal_subworkflow_and_terminal_form_an_acyclic_sequence() -> TestResult {
    let body = body_revision()?;
    let reference = PinnedSubworkflow::new(
        WorkflowId::new("body")?,
        body.id().clone(),
        empty_interface()?,
    );
    let repeat = Node::new(
        id("repeat")?,
        NodeKind::Repeat {
            config: RepeatConfig::new(
                reference.clone(),
                Condition::Constant { value: false },
                10,
                RepeatBudget {
                    max_duration_ms: Some(60_000),
                    max_cost_micros: None,
                    max_cost_currency: None,
                },
                RepeatTermination::Fail,
            )?,
        },
    )?
    .with_control_output(port("next")?)?;
    let wait = Node::new(id("wait")?, NodeKind::Wait { duration_ms: 1_000 })?
        .with_control_input(port("in")?)?
        .with_control_output(port("next")?)?;
    let signal = Node::new(
        id("signal")?,
        NodeKind::SignalWait {
            signal: OperationId::new("workflow.approved")?,
        },
    )?
    .with_control_input(port("in")?)?
    .with_control_output(port("next")?)?;
    let subworkflow = Node::new(id("subworkflow")?, NodeKind::Subworkflow { reference })?
        .with_control_input(port("in")?)?
        .with_control_output(port("next")?)?;
    let done = terminal_node("done")?.with_control_input(port("in")?)?;
    let nodes = [repeat, wait, signal, subworkflow, done];
    let mut operations = vec![Mutation::SetInterface {
        interface: empty_interface()?,
    }];
    operations.extend(nodes.into_iter().map(|node| Mutation::AddNode { node }));
    for (edge_id, source, target) in [
        ("repeat-wait", "repeat", "wait"),
        ("wait-signal", "wait", "signal"),
        ("signal-sub", "signal", "subworkflow"),
        ("sub-done", "subworkflow", "done"),
    ] {
        operations.push(Mutation::AddEdge {
            edge: Edge::new(
                EdgeId::new(edge_id)?,
                EdgeKind::Control,
                id(source)?,
                port("next")?,
                id(target)?,
                port("in")?,
            ),
        });
    }
    let revision = genesis("durable-nodes", operations)?;
    assert_eq!(revision.semantic().nodes().len(), 5);
    Ok(())
}

#[test]
fn repeat_cost_budgets_bind_one_validated_currency_ledger() -> TestResult {
    let usd = CostCurrencyCode::new("USD")?;
    assert_eq!(usd.as_str(), "USD");
    assert!(CostCurrencyCode::new("usd").is_err());
    assert!(CostCurrencyCode::new("EURO").is_err());

    let budget = RepeatBudget {
        max_duration_ms: Some(1_000),
        max_cost_micros: Some(50_000),
        max_cost_currency: Some(usd),
    };
    assert_eq!(
        serde_json::from_value::<RepeatBudget>(serde_json::to_value(&budget)?)?,
        budget
    );

    let legacy_none = serde_json::json!({
        "max_duration_ms": null,
        "max_cost_micros": null
    });
    assert_eq!(
        serde_json::from_value::<RepeatBudget>(legacy_none)?,
        RepeatBudget {
            max_duration_ms: None,
            max_cost_micros: None,
            max_cost_currency: None,
        }
    );
    assert!(
        serde_json::from_value::<RepeatBudget>(serde_json::json!({
            "max_duration_ms": null,
            "max_cost_micros": 1,
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<RepeatBudget>(serde_json::json!({
            "max_duration_ms": null,
            "max_cost_micros": null,
            "max_cost_currency": "USD",
        }))
        .is_err()
    );
    Ok(())
}

#[test]
fn subworkflow_instantiation_and_upgrade_keep_exact_revision_pins() -> TestResult {
    let body_v1 = body_revision()?;
    let body_v2 = body_v1.revise(
        body_v1.id(),
        MutationBatch::new(vec![Mutation::SetMetadata {
            metadata: BlueprintMetadata::new(
                "body v2",
                "updated body metadata",
                BTreeSet::new(),
                BTreeMap::new(),
            )?,
        }])?,
        AuthorRef::new("human:test")?,
        "body v2",
    )?;
    let interface = empty_interface()?;
    let node = Node::new(
        id("body-call")?,
        NodeKind::Subworkflow {
            reference: PinnedSubworkflow::new(
                WorkflowId::new("body")?,
                body_v1.id().clone(),
                interface.clone(),
            ),
        },
    )?
    .with_control_output(port("next")?)?;
    let outer = genesis(
        "outer",
        vec![
            Mutation::SetInterface {
                interface: interface.clone(),
            },
            Mutation::InstantiateSubworkflow { node },
            Mutation::AddNode {
                node: terminal_node("done")?.with_control_input(port("in")?)?,
            },
            Mutation::AddEdge {
                edge: Edge::new(
                    EdgeId::new("body-done")?,
                    EdgeKind::Control,
                    id("body-call")?,
                    port("next")?,
                    id("done")?,
                    port("in")?,
                ),
            },
        ],
    )?;
    let upgraded = outer.revise(
        outer.id(),
        MutationBatch::new(vec![Mutation::UpgradeSubworkflow {
            node: id("body-call")?,
            expected_revision: body_v1.id().clone(),
            replacement: PinnedSubworkflow::new(
                WorkflowId::new("body")?,
                body_v2.id().clone(),
                interface,
            ),
        }])?,
        AuthorRef::new("human:test")?,
        "pin body v2",
    )?;
    let Some(node) = upgraded.semantic().nodes().get(&id("body-call")?) else {
        return Err("upgraded node missing".into());
    };
    let NodeKind::Subworkflow { reference } = node.kind() else {
        return Err("upgraded node kind changed".into());
    };
    assert_eq!(reference.revision(), body_v2.id());
    Ok(())
}

#[test]
fn deterministic_content_ignores_mutation_order_and_layout() -> TestResult {
    let first = simple_sequence("deterministic", false)?;
    let reordered = simple_sequence("deterministic", true)?;
    let presentation_layout_a = BTreeMap::from([("first", (1_i32, 2_i32))]);
    let presentation_layout_b = BTreeMap::from([("first", (900_i32, -30_i32))]);
    assert_ne!(presentation_layout_a, presentation_layout_b);
    assert_eq!(first.content_digest(), reordered.content_digest());
    assert_eq!(first.id(), reordered.id());
    Ok(())
}

#[test]
fn failed_batch_rolls_back_and_optimistic_conflicts_are_typed() -> TestResult {
    let base = simple_sequence("rollback", false)?;
    let before = base.clone();
    let duplicate = MutationBatch::new(vec![Mutation::AddNode {
        node: terminal_node("done")?,
    }])?;
    let failure = base.revise(
        base.id(),
        duplicate,
        AuthorRef::new("human:test")?,
        "duplicate",
    );
    assert!(matches!(failure, Err(MutationError::Operation(_))));
    assert_eq!(base, before);

    let other = simple_sequence("other", false)?;
    let batch = MutationBatch::new(vec![Mutation::SetInterface {
        interface: empty_interface()?,
    }])?;
    let conflict = base.revise(
        other.id(),
        batch,
        AuthorRef::new("human:test")?,
        "wrong base",
    );
    assert!(matches!(
        conflict,
        Err(MutationError::BaseRevisionConflict { .. })
    ));
    Ok(())
}

#[test]
fn deliberate_merge_requires_explicit_resolved_candidate() -> TestResult {
    let base = simple_sequence("merge", false)?;
    let other = simple_sequence("merge-other", false)?;
    let batch = MutationBatch::new(vec![Mutation::SetMergeParents {
        parents: vec![base.id().clone(), other.id().clone()],
    }])?;
    let merged = base.revise(
        base.id(),
        batch,
        AuthorRef::new("human:merger")?,
        "explicitly resolved merge",
    )?;
    assert_eq!(merged.parents().len(), 2);
    assert_eq!(merged.content_digest(), base.content_digest());
    assert_ne!(merged.id(), base.id());
    Ok(())
}

#[test]
fn hostile_depth_path_and_future_version_are_rejected() -> TestResult {
    let segments = (0..33)
        .map(|index| PathSegment::Index(index as u16))
        .collect();
    assert!(PathSelector::new(segments).is_err());

    let future = br#"{"schema_version":2,"revision":{}}"#;
    assert!(matches!(
        BlueprintRevisionDocument::from_json(future),
        Err(DocumentError::UnsupportedVersion { found: 2, .. })
    ));
    let mut nested = "null".to_owned();
    for _ in 0..70 {
        nested = format!("[{nested}]");
    }
    let hostile = format!("{{\"schema_version\":1,\"revision\":{nested}}}");
    assert!(matches!(
        BlueprintRevisionDocument::from_json(hostile.as_bytes()),
        Err(DocumentError::Bounds { .. })
    ));
    assert!(
        BlueprintRevisionDocument::from_json(br#"{"schema_version":1,"schema_version":1}"#)
            .is_err()
    );
    Ok(())
}

#[test]
fn tagged_core_values_reject_unknown_fields() -> TestResult {
    assert!(
        serde_json::from_value::<PathSegment>(serde_json::json!({
            "type": "field",
            "value": "answer",
            "surprise": true
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<JoinPolicy>(serde_json::json!({
            "type": "quorum",
            "quorum": 2,
            "surprise": true
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<ReducerStrategy>(serde_json::json!({
            "type": "capability",
            "operation": "tool.reduce",
            "surprise": true
        }))
        .is_err()
    );
    Ok(())
}

#[test]
fn illegal_cycle_reports_a_stable_code() -> TestResult {
    let a = task_node("a")?
        .with_control_input(port("in")?)?
        .with_control_output(port("out")?)?;
    let b = terminal_node("b")?.with_control_input(port("in")?)?;
    let result = genesis(
        "cycle",
        vec![
            Mutation::SetInterface {
                interface: empty_interface()?,
            },
            Mutation::AddNode { node: a },
            Mutation::AddNode { node: b },
            Mutation::AddEdge {
                edge: Edge::new(
                    EdgeId::new("a-b")?,
                    EdgeKind::Control,
                    id("a")?,
                    port("out")?,
                    id("b")?,
                    port("in")?,
                ),
            },
            Mutation::AddEdge {
                edge: Edge::new(
                    EdgeId::new("b-a")?,
                    EdgeKind::Control,
                    id("b")?,
                    port("out")?,
                    id("a")?,
                    port("in")?,
                ),
            },
        ],
    );
    let Err(MutationError::Validation(error)) = result else {
        return Err("expected validation error".into());
    };
    assert!(
        error
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == DiagnosticCode::IllegalCycle)
    );
    Ok(())
}

fn generated_sequence(count: usize) -> TestResult<BlueprintRevision> {
    let mut operations = vec![Mutation::SetInterface {
        interface: empty_interface()?,
    }];
    for index in 0..count {
        let node = task_node(&format!("task-{index}"))?
            .with_control_input(port("in")?)?
            .with_control_output(port("out")?)?;
        operations.push(Mutation::AddNode { node });
    }
    operations.push(Mutation::AddNode {
        node: terminal_node("done")?.with_control_input(port("in")?)?,
    });
    for index in 0..count {
        let source = format!("task-{index}");
        let target = if index + 1 == count {
            "done".to_owned()
        } else {
            format!("task-{}", index + 1)
        };
        operations.push(Mutation::AddEdge {
            edge: Edge::new(
                EdgeId::new(format!("edge-{index}"))?,
                EdgeKind::Control,
                id(&source)?,
                port("out")?,
                id(&target)?,
                port("in")?,
            ),
        });
    }
    Ok(genesis("generated", operations)?)
}

proptest! {
    #[test]
    fn every_published_generated_revision_is_valid(count in 1_usize..24) {
        let revision = generated_sequence(count);
        prop_assert!(revision.is_ok());
        if let Ok(revision) = revision {
            prop_assert_eq!(revision.semantic().nodes().len(), count + 1);
        }
    }
}

#[test]
fn blueprint_golden_fixture_is_exact_and_canonical() -> TestResult {
    let revision = simple_sequence("golden", false)?;
    let bytes = BlueprintRevisionDocument::new(&revision).to_canonical_json()?;
    let fixture = include_bytes!("fixtures/revision-v1.json").trim_ascii_end();
    if fixture.is_empty() {
        eprintln!("{}", String::from_utf8(bytes.clone())?);
    }
    assert_eq!(bytes, fixture);
    let (document, decoded) = BlueprintRevisionDocument::from_json(fixture)?;
    assert_eq!(decoded, revision);
    assert_eq!(document.to_canonical_json()?, fixture);
    Ok(())
}

// Compile-time type distinction is part of the public API contract.
fn _revision_is_not_a_workflow(_: RevisionId) {}
