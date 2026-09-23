//! Governed method authoring and indirect structural refusal through public readers.

use milkdrift_blueprint::{
    AdaptationScope, AuthorRef, BindingSource, BlueprintRevision, BlueprintRevisionDocument,
    DataPort, Edge, EdgeId, EdgeKind, GoverningAgreement, Mutation, MutationBatch, Node, NodeId,
    NodeKind, PortId, SchemaRef, TerminalOutcome, WorkflowId, validate_agreement_adoption,
};
use milkdrift_capability::{BoundedJson, CapabilityRequirement, OperationId, SchemaId};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn requirement() -> TestResult<CapabilityRequirement> {
    Ok(CapabilityRequirement::new(OperationId::new(
        "tool.investigate",
    )?))
}

fn task(name: &str, instruction: &str) -> TestResult<Node> {
    Ok(Node::new(
        NodeId::new(name)?,
        NodeKind::task_direct_inputs(requirement()?)?,
    )?
    .with_control_input(PortId::new("in")?)?
    .with_control_output(PortId::new("out")?)?
    .with_data_input(
        PortId::new("instruction")?,
        DataPort::input(
            SchemaRef::new(SchemaId::new("example.instruction")?, 1)?,
            true,
            Some(BindingSource::Literal {
                value: BoundedJson::new(serde_json::json!(instruction))?,
            }),
        )?,
    )?)
}

fn edge(name: &str, from: &str, to: &str) -> TestResult<Edge> {
    Ok(Edge::new(
        EdgeId::new(name)?,
        EdgeKind::Control,
        NodeId::new(from)?,
        PortId::new("out")?,
        NodeId::new(to)?,
        PortId::new("in")?,
    ))
}

fn baseline() -> TestResult<BlueprintRevision> {
    let entry = Node::new(NodeId::new("entry")?, NodeKind::Wait { duration_ms: 1 })?
        .with_control_output(PortId::new("out")?)?;
    let done = Node::new(
        NodeId::new("done")?,
        NodeKind::Terminal {
            outcome: TerminalOutcome::Success,
        },
    )?
    .with_control_input(PortId::new("in")?)?;
    Ok(BlueprintRevision::genesis(
        WorkflowId::new("governed-method")?,
        MutationBatch::new(vec![
            Mutation::AddNode { node: entry },
            Mutation::AddNode {
                node: task("repair.begin", "implement the supplied requirements")?,
            },
            Mutation::AddNode {
                node: task("repair.end", "freeze the resulting candidate")?,
            },
            Mutation::AddNode {
                node: task("verify", "trusted checks")?,
            },
            Mutation::AddNode { node: done },
            Mutation::AddEdge {
                edge: edge("entry", "entry", "repair.begin")?,
            },
            Mutation::AddEdge {
                edge: edge("internal", "repair.begin", "repair.end")?,
            },
            Mutation::AddEdge {
                edge: edge("verify", "repair.end", "verify")?,
            },
            Mutation::AddEdge {
                edge: edge("done", "verify", "done")?,
            },
        ])?,
        AuthorRef::new("human:operator")?,
        "initial method",
    )?)
}

fn govern(base: &BlueprintRevision) -> TestResult<BlueprintRevision> {
    let agreement = GoverningAgreement::seal(
        NodeId::new("test-agreement")?,
        base,
        AdaptationScope::new("repair.".to_owned(), 4, vec![requirement()?])?,
        format!("b3_{}", "a".repeat(64)),
    )?;
    revise(
        base,
        vec![Mutation::SetAgreement {
            agreement: Some(agreement),
        }],
    )
}

fn revise(base: &BlueprintRevision, changes: Vec<Mutation>) -> TestResult<BlueprintRevision> {
    Ok(base.revise(
        base.id(),
        MutationBatch::new(changes)?,
        AuthorRef::new("agent:repair")?,
        "prospective repair",
    )?)
}

#[test]
fn useful_investigation_and_repair_preserve_the_agreement_and_roundtrip() -> TestResult {
    let governed = govern(&baseline()?)?;
    let repaired = revise(
        &governed,
        vec![
            Mutation::ReplaceNode {
                node: task(
                    "repair.begin",
                    "retain the failed candidate; repair authenticated mutations",
                )?,
            },
            Mutation::AddNode {
                node: task(
                    "repair.investigate",
                    "inspect the selected failure evidence",
                )?,
            },
            Mutation::ReplaceEdge {
                edge: edge("internal", "repair.begin", "repair.investigate")?,
            },
            Mutation::AddEdge {
                edge: edge("investigated", "repair.investigate", "repair.end")?,
            },
        ],
    )?;
    validate_agreement_adoption(&governed, &repaired)?;
    assert_eq!(
        governed.semantic().agreement(),
        repaired.semantic().agreement()
    );
    assert_ne!(governed.id(), repaired.id());
    let bytes = BlueprintRevisionDocument::new(&repaired).to_canonical_json()?;
    let (document, decoded) = BlueprintRevisionDocument::from_json(&bytes)?;
    assert_eq!(decoded, repaired);
    assert_eq!(document.to_canonical_json()?, bytes);
    Ok(())
}

#[test]
fn protected_nodes_dependencies_terminal_and_scope_cannot_change() -> TestResult {
    let governed = govern(&baseline()?)?;
    let cases = vec![
        vec![Mutation::ReplaceNode {
            node: task("verify", "trust the agent's report")?,
        }],
        vec![Mutation::ReplaceEdge {
            edge: edge("verify", "repair.begin", "verify")?,
        }],
        vec![Mutation::ReplaceNode {
            node: Node::new(
                NodeId::new("done")?,
                NodeKind::Terminal {
                    outcome: TerminalOutcome::Failure,
                },
            )?
            .with_control_input(PortId::new("in")?)?,
        }],
        vec![Mutation::AddNode {
            node: task("outside", "same edit in the enclosing scope")?,
        }],
        vec![
            Mutation::ReplaceNode {
                node: Node::new(
                    NodeId::new("repair.end")?,
                    NodeKind::Terminal {
                        outcome: TerminalOutcome::Success,
                    },
                )?
                .with_control_input(PortId::new("in")?)?,
            },
            Mutation::RemoveEdge {
                edge: EdgeId::new("verify")?,
            },
        ],
    ];
    for changes in cases {
        assert!(revise(&governed, changes).is_err());
    }
    let ungoverned = revise(&governed, vec![Mutation::SetAgreement { agreement: None }])?;
    assert!(validate_agreement_adoption(&governed, &ungoverned).is_err());
    assert!(validate_agreement_adoption(&ungoverned, &governed).is_err());
    let different = govern(&baseline()?)?;
    assert_eq!(
        different.semantic().agreement(),
        governed.semantic().agreement()
    );
    Ok(())
}

#[test]
fn indirect_data_rewiring_and_new_capability_requirements_refuse() -> TestResult {
    let governed = govern(&baseline()?)?;
    let changed_requirement = Node::new(
        NodeId::new("repair.begin")?,
        NodeKind::task_direct_inputs(CapabilityRequirement::new(OperationId::new(
            "resource.manage",
        )?))?,
    )?
    .with_control_input(PortId::new("in")?)?
    .with_control_output(PortId::new("out")?)?;
    assert!(
        revise(
            &governed,
            vec![Mutation::ReplaceNode {
                node: changed_requirement
            }]
        )
        .is_err()
    );
    let modified_input = task("verify", "unchanged instruction")?.with_data_input(
        PortId::new("forged-report")?,
        DataPort::input(
            SchemaRef::new(SchemaId::new("example.report")?, 1)?,
            true,
            Some(BindingSource::Literal {
                value: BoundedJson::new(serde_json::json!({"accepted": true}))?,
            }),
        )?,
    )?;
    assert!(
        revise(
            &governed,
            vec![Mutation::ReplaceNode {
                node: modified_input
            }]
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn agreement_readers_refuse_identity_version_and_unknown_field_changes() -> TestResult {
    let governed = govern(&baseline()?)?;
    let original =
        serde_json::to_value(governed.semantic().agreement().ok_or("missing agreement")?)?;
    for (key, value) in [
        ("schema_version", serde_json::json!(2)),
        (
            "effect_policy",
            serde_json::json!(format!("b3_{}", "b".repeat(64))),
        ),
        (
            "digest",
            serde_json::json!(format!("b3_{}", "0".repeat(64))),
        ),
        ("extra", serde_json::json!(true)),
    ] {
        let mut changed = original.clone();
        changed[key] = value;
        assert!(serde_json::from_value::<GoverningAgreement>(changed).is_err());
    }
    assert!(AdaptationScope::new("repair".to_owned(), 4, vec![requirement()?]).is_err());
    assert!(AdaptationScope::new("repair.".to_owned(), 0, vec![requirement()?]).is_err());
    assert!(AdaptationScope::new("repair.".to_owned(), 4, vec![]).is_err());
    Ok(())
}
