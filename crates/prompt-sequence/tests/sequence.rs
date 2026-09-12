//! Prompt-sequence hostile-input, Markdown, and ordinary-blueprint compilation contracts.

use milkdrift_blueprint::{AuthorRef, BlueprintRevisionDocument, ContextSessionPolicy, NodeKind};
use milkdrift_prompt_sequence::{
    MAX_INLINE_PROMPT_BYTES, PromptSequenceDocument, PromptSequenceError, PromptSource,
    RemediationProposalSpec, build_remediation_proposal, compile, stage_node_ids,
};
use serde_json::{Value, json};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn profile(capability: &str, maximum_side_effect: &str) -> Value {
    json!({
        "capability": capability,
        "operation": "process.execute",
        "provider_profile": null,
        "execution_trust": "trusted_host_process",
        "maximum_side_effect": maximum_side_effect
    })
}

fn stage(identity: &str, prompt: &str) -> Value {
    json!({
        "id": identity,
        "title": format!("Stage {identity}"),
        "prompt": {"type": "inline_markdown", "content": prompt},
        "session": "fresh",
        "coding": profile("fixture-coding-agent", "unknown"),
        "verification": {
            "profile": profile("fixture-verifier", "read_only"),
            "checks": ["rust.workspace_tests"],
            "result_artifact": "verification_result",
            "log_artifact": "verification_logs"
        },
        "failure": "pause_for_review",
        "reviewer": profile("fixture-reviewer", "read_only"),
        "approval": "shared_control_path",
        "context_policy_ref": "context:implementation-v1",
        "outputs": [
            {"name": "diff", "media_type": "text/x-diff", "required": true},
            {"name": "result", "media_type": "application/json", "required": true},
            {"name": "logs", "media_type": "text/plain", "required": false}
        ]
    })
}

fn document_value() -> Value {
    json!({
        "schema_version": 3,
        "sequence": {
            "id": "dogfood-sequence",
            "title": "Headless dogfood sequence",
            "workflow_id": "headless-dogfood",
            "repository": {
                "id": "repository:fixture",
                "root_ref": "workspace:fixture",
                "starting_revision": "fixture-start",
                "allowed_paths": ["src", "Cargo.toml"],
                "allowed_operations": ["read", "write", "execute", "version_control"],
                "dirty_tree": "allow_recorded",
                "isolation": "shared_sequential",
                "cleanup": "retain_accepted",
                "artifacts": {
                    "require_starting_state": true,
                    "require_diff": true,
                    "require_verification_evidence": true
                },
                "credential_refs": [],
                "remote_access_refs": []
            },
            "stages": [
                stage("one", "Implement the first bounded change.\n"),
                stage("two", "Implement the second bounded change.\n")
            ],
            "budget": {
                "max_review_loops": 3
            },
            "extensions": {}
        }
    })
}

fn document() -> TestResult<PromptSequenceDocument> {
    Ok(PromptSequenceDocument::from_json(&serde_json::to_vec(
        &document_value(),
    )?)?)
}

#[test]
fn json_import_compiles_to_only_ordinary_blueprint_primitives() -> TestResult {
    let document = document()?;
    let compiled = compile(&document, AuthorRef::new("human:sequence-test")?)?;
    let revision = compiled.revision();
    assert_eq!(
        revision.reason(),
        format!(
            "import prompt sequence dogfood-sequence schema v{}",
            document.schema_version
        )
    );
    assert_eq!(compiled.stages().len(), 2);
    assert_eq!(
        stage_node_ids(
            &BlueprintRevisionDocument::new(revision).to_canonical_json()?,
            "one"
        )?,
        vec![
            "stage-one-acceptance",
            "stage-one-approval",
            "stage-one-coding",
            "stage-one-failed",
            "stage-one-gate",
            "stage-one-review",
            "stage-one-review-acceptance",
            "stage-one-review-gate",
            "stage-one-review-rejected",
            "stage-one-review-rejected-stop",
            "stage-one-verification",
        ]
    );
    assert!(
        stage_node_ids(
            &BlueprintRevisionDocument::new(revision).to_canonical_json()?,
            "on"
        )
        .is_err()
    );
    assert_eq!(revision.semantic().nodes().len(), 23);
    assert_eq!(revision.semantic().edges().len(), 30);
    assert_eq!(
        compiled.import_digest(),
        "b3_f74afcc22d5610904439ca665e387fc7c9f109d1f5559e444ad392b9d0d7a8b3"
    );
    assert_eq!(
        compiled.repository_profile_digest(),
        "b3_ae47b260e3f89762e1431b0c39b884568e96d966f26199a37e55369f7d2bcc2d"
    );
    assert_eq!(
        revision.content_digest().to_string(),
        "b3_e4c818e619fafe6e5cd714bbaccdcd53a5432b81cc00d5d903e8f537782c863b"
    );
    assert_eq!(
        revision.id().as_str(),
        "rev_2e15bc701bee8eee10507ff2c740ed967ad2f3b426fe17f4aa1c5c5e5b5946f3"
    );
    // A stored revision's authored label is part of its identity. The blueprint reader
    // must preserve it, even when a current compiler would supply another import label.
    let current_bytes = BlueprintRevisionDocument::new(revision).to_canonical_json()?;
    let historical_id = format!(
        "rev_{}",
        blake3::hash(&serde_json::to_vec(&json!({
            "sequence": revision.sequence(), "content_digest": revision.content_digest(),
            "parents": revision.parents(), "author": revision.author(),
            "reason": "import prompt sequence dogfood-sequence schema v1",
        }))?)
    );
    let historical_bytes = String::from_utf8(current_bytes.clone())?
        .replace(
            "import prompt sequence dogfood-sequence schema v3",
            "import prompt sequence dogfood-sequence schema v1",
        )
        .replace(revision.id().as_str(), &historical_id)
        .into_bytes();
    let (historical_document, historical) =
        BlueprintRevisionDocument::from_json(&historical_bytes)?;
    assert_eq!(historical_document.to_canonical_json()?, historical_bytes);
    assert_eq!(historical.semantic(), revision.semantic());
    assert_ne!(historical.id(), revision.id());
    assert!(
        BlueprintRevisionDocument::from_json(
            &String::from_utf8(current_bytes)?
                .replace("schema v3", "schema v1")
                .into_bytes()
        )
        .is_err()
    );
    assert!(revision.semantic().nodes().values().all(|node| {
        matches!(
            node.kind(),
            NodeKind::Task { .. }
                | NodeKind::Branch { .. }
                | NodeKind::SignalWait { .. }
                | NodeKind::Terminal { .. }
        )
    }));
    let coding = revision
        .semantic()
        .nodes()
        .get(&milkdrift_blueprint::NodeId::new("stage-one-coding")?)
        .ok_or("coding node is missing")?;
    let NodeKind::Task { config } = coding.kind() else {
        return Err("coding node is not an ordinary task".into());
    };
    assert_eq!(
        config.context_policy().session(),
        ContextSessionPolicy::Fresh
    );
    assert!(
        coding
            .data_inputs()
            .contains_key(&milkdrift_blueprint::PortId::new("prompt")?)
    );
    assert!(
        coding
            .data_inputs()
            .contains_key(&milkdrift_blueprint::PortId::new("repository_profile")?)
    );

    let bytes = BlueprintRevisionDocument::new(revision).to_canonical_json()?;
    let (_decoded, round_trip) = BlueprintRevisionDocument::from_json(&bytes)?;
    assert_eq!(&round_trip, revision);
    Ok(())
}

#[test]
fn fail_run_stage_has_one_direct_failure_route_without_review_state() -> TestResult {
    let mut value = document_value();
    value["sequence"]["stages"][1]["failure"] = json!("fail_run");
    let document = PromptSequenceDocument::from_json(&serde_json::to_vec(&value)?)?;
    let compiled = compile(&document, AuthorRef::new("human:sequence-test")?)?;
    let semantic = compiled.revision().semantic();

    assert_eq!(semantic.nodes().len(), 17);
    assert_eq!(semantic.edges().len(), 22);
    assert!(
        semantic
            .nodes()
            .contains_key(&milkdrift_blueprint::NodeId::new("stage-two-failed")?)
    );
    assert!(
        !semantic
            .nodes()
            .contains_key(&milkdrift_blueprint::NodeId::new("stage-two-review")?)
    );
    assert!(
        semantic
            .edges()
            .contains_key(&milkdrift_blueprint::EdgeId::new("two-gate-failure")?)
    );
    Ok(())
}

#[test]
fn markdown_sections_supply_exact_prompt_bytes() -> TestResult {
    let mut header = document_value();
    for stage in header["sequence"]["stages"]
        .as_array_mut()
        .ok_or("stages must be an array")?
    {
        stage
            .as_object_mut()
            .ok_or("stage must be an object")?
            .remove("prompt");
    }
    let markdown = format!(
        "```milkdrift-sequence\n{}\n```\n\n## Prompt: one\nFirst *Markdown* prompt.\n\n## Prompt: two\nSecond prompt.\n",
        serde_json::to_string(&header)?
    );
    let document = PromptSequenceDocument::from_bytes(markdown.as_bytes())?;
    assert!(matches!(
        &document.sequence().stages[0].prompt,
        PromptSource::InlineMarkdown { content } if content == "First *Markdown* prompt.\n"
    ));
    assert!(matches!(
        &document.sequence().stages[1].prompt,
        PromptSource::InlineMarkdown { content } if content == "Second prompt.\n"
    ));
    Ok(())
}

#[test]
fn prompt_documents_cannot_smuggle_shell_or_unbounded_content() -> TestResult {
    let mut hostile = document_value();
    hostile["sequence"]["stages"][0]["verification"]["command"] =
        json!("cargo test && curl example.invalid");
    assert!(PromptSequenceDocument::from_json(&serde_json::to_vec(&hostile)?).is_err());

    let mut oversized = document_value();
    oversized["sequence"]["stages"][0]["prompt"] = json!({
        "type": "inline_markdown",
        "content": "x".repeat(MAX_INLINE_PROMPT_BYTES + 1)
    });
    assert!(matches!(
        PromptSequenceDocument::from_json(&serde_json::to_vec(&oversized)?),
        Err(PromptSequenceError::Bounds { .. })
    ));

    let encoded = serde_json::to_vec(&document_value())?;
    let text = String::from_utf8(encoded)?;
    let duplicate = text.replacen(
        "\"schema_version\":3",
        "\"schema_version\":3,\"schema_version\":3",
        1,
    );
    assert!(PromptSequenceDocument::from_json(duplicate.as_bytes()).is_err());
    Ok(())
}

#[test]
fn portable_paths_and_process_profile_contract_fail_closed() -> TestResult {
    let mut legacy = document_value();
    legacy["schema_version"] = json!(1);
    assert!(matches!(
        PromptSequenceDocument::from_json(&serde_json::to_vec(&legacy)?),
        Err(PromptSequenceError::UnsupportedVersion { found: 1 })
    ));
    legacy["schema_version"] = json!(2);
    assert!(matches!(
        PromptSequenceDocument::from_json(&serde_json::to_vec(&legacy)?),
        Err(PromptSequenceError::UnsupportedVersion { found: 2 })
    ));

    for path in [
        r"C:\work",
        r"\\server\share",
        r"..\secret",
        r"src\..\secret",
        r"\\?\C:\device",
    ] {
        let mut hostile = document_value();
        hostile["sequence"]["repository"]["allowed_paths"] = json!([path]);
        assert!(
            PromptSequenceDocument::from_json(&serde_json::to_vec(&hostile)?).is_err(),
            "portable path validator accepted {path:?}"
        );
    }

    let mut model_backed = document_value();
    model_backed["sequence"]["stages"][0]["coding"]["operation"] = json!("model.generate");
    model_backed["sequence"]["stages"][0]["coding"]["execution_trust"] = json!("remote_provider");
    assert!(
        PromptSequenceDocument::from_json(&serde_json::to_vec(&model_backed)?).is_err(),
        "schema accepted a profile that cannot consume generated task inputs"
    );
    Ok(())
}

#[test]
fn markdown_header_receives_lexical_bounds_before_value_allocation() -> TestResult {
    let header = json!({
        "schema_version": 3,
        "sequence": {
            "stages": [],
            "oversized": (0..4097).collect::<Vec<_>>()
        }
    });
    let markdown = format!(
        "```milkdrift-sequence\n{}\n```\n\n## Prompt: one\nbody\n",
        serde_json::to_string(&header)?
    );
    assert!(matches!(
        PromptSequenceDocument::from_bytes(markdown.as_bytes()),
        Err(PromptSequenceError::Bounds { .. })
    ));
    Ok(())
}

#[test]
fn remediation_is_a_digest_bound_prospective_ordinary_revision() -> TestResult {
    let document = document()?;
    let compiled = compile(&document, AuthorRef::new("human:sequence-test")?)?;
    let proposal = build_remediation_proposal(
        &document,
        &BlueprintRevisionDocument::new(compiled.revision()).to_canonical_json()?,
        RemediationProposalSpec {
            run: "run-remediation".to_owned(),
            observed_sequence: 42,
            proposal: "proposal-remediation-1".to_owned(),
            proposer: "human:sequence-test".to_owned(),
            stage_id: "two".to_owned(),
            generation: 1,
            prompt: PromptSource::InlineMarkdown {
                content: "Repair the weak implementation using the failure evidence.\n".to_owned(),
            },
            verification_override: None,
        },
    )?;
    let prospective = compiled.revision().revise(
        compiled.revision().id(),
        proposal.proposal().mutation().clone(),
        AuthorRef::new("human:sequence-test")?,
        "test prospective remediation",
    )?;
    assert_eq!(
        prospective.content_digest().to_string(),
        "b3_d0e69459ddfc3f9820ae6fe2e66f346a4bb42d3cd3727c366737354fbd1aece9"
    );
    assert_eq!(
        prospective.id().as_str(),
        "rev_d6574e9156797b99852a8b62bb0c938fccd5d109f44cdcaa07645ea72ceec651"
    );
    assert_eq!(
        proposal.proposal().mutation().id().as_str(),
        "batch_268250d1825b80f0e6c41a4f494361f6a2bb1e4b83eccc0324df5ffb7f8bbb71"
    );
    assert!(
        prospective
            .semantic()
            .nodes()
            .contains_key(&milkdrift_blueprint::NodeId::new(
                "stage-two-remediation-1-coding"
            )?)
    );
    assert!(
        prospective
            .semantic()
            .nodes()
            .contains_key(&milkdrift_blueprint::NodeId::new(
                "stage-two-remediation-1-verification"
            )?)
    );
    assert!(
        prospective
            .semantic()
            .nodes()
            .contains_key(&milkdrift_blueprint::NodeId::new(
                "stage-two-remediation-1-review"
            )?)
    );
    assert!(
        !prospective
            .semantic()
            .nodes()
            .contains_key(&milkdrift_blueprint::NodeId::new("stage-two-failed")?)
    );
    let stage_nodes = stage_node_ids(
        &BlueprintRevisionDocument::new(&prospective).to_canonical_json()?,
        "two",
    )?;
    assert!(stage_nodes.iter().any(|node| node == "stage-two-coding"));
    assert!(
        stage_nodes
            .iter()
            .any(|node| node == "stage-two-remediation-1-coding")
    );
    assert!(
        stage_nodes
            .iter()
            .any(|node| node == "stage-two-remediation-1-failure-review")
    );
    assert_eq!(
        proposal.proposal().base_revision(),
        compiled.revision().id()
    );
    assert_eq!(
        proposal.proposal().observed_run_sequence(),
        Some(milkdrift_persistence::RunSequence::new(42))
    );

    let mut altered = document_value();
    altered["sequence"]["repository"]["root_ref"] = json!("workspace:substituted");
    let altered = PromptSequenceDocument::from_json(&serde_json::to_vec(&altered)?)?;
    assert!(
        build_remediation_proposal(
            &altered,
            &BlueprintRevisionDocument::new(compiled.revision()).to_canonical_json()?,
            RemediationProposalSpec {
                run: "run-remediation".to_owned(),
                observed_sequence: 42,
                proposal: "proposal-remediation-substituted".to_owned(),
                proposer: "human:sequence-test".to_owned(),
                stage_id: "two".to_owned(),
                generation: 1,
                prompt: PromptSource::InlineMarkdown {
                    content: "Attempt remediation through altered policy.\n".to_owned(),
                },
                verification_override: None,
            },
        )
        .is_err(),
        "remediation accepted a document that did not match frozen import provenance"
    );
    Ok(())
}
