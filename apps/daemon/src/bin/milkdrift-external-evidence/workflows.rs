use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
    process::Command,
};

use milkdrift_blueprint::{
    AuthorRef, BindingSource, BlueprintRevision, BlueprintRevisionDocument, ContextBudget,
    ContextCategory, ContextOrdering, ContextSemanticRole, ContextSessionPolicy, ContextTruncation,
    DataPort, Edge, EdgeId, EdgeKind, Mutation, MutationBatch, Node, NodeId, NodeKind, PortId,
    SchemaRef, TaskConfig, TaskContextPolicy, TerminalOutcome, WorkflowId,
};
use milkdrift_capability::{
    BoundedJson, CapabilityId, CapabilityRequirement, ExecutionTrustClass, OperationId,
    ProviderProfileRef, SchemaId, SideEffectClass, StreamingMode,
};
use milkdrift_model::{
    ContentPart, MODEL_TASK_INPUT_NAME, Message, MessageRole, ModelTaskRequest,
    ModelTaskRequestDocument, SessionSelection, StructuredOutput,
};
use milkdrift_model_provider::EndpointProfile;
use milkdrift_prompt_sequence::PromptSequenceDocument;
use serde_json::{Value, json};

pub const PROCESS_WORKFLOW: &str = "external-evidence-process";
pub const PROCESS_RUN: &str = "run-external-evidence-process";
pub const MODEL_WORKFLOW: &str = "external-evidence-model";
pub const MODEL_RUN: &str = "run-external-evidence-model";

pub struct ModelProfileFacts {
    pub profile_id: String,
    pub revision: u64,
    pub protocol: String,
    pub model_alias: String,
    pub endpoint_origin: String,
    pub streaming: bool,
    pub structured_output: bool,
    pub secret_refs: BTreeSet<String>,
}

pub fn initialize_repository(repository: &Path) -> Result<(String, String), String> {
    fs::create_dir_all(repository).map_err(|error| error.to_string())?;
    fs::write(
        repository.join("calculator.py"),
        "def add(a, b):\n    return a - b\n",
    )
    .map_err(|error| error.to_string())?;
    fs::write(
        repository.join("test_calculator.py"),
        "import unittest\nfrom calculator import add\n\nclass CalculatorTest(unittest.TestCase):\n    def test_add(self):\n        self.assertEqual(add(2, 3), 5)\n\nif __name__ == '__main__':\n    unittest.main()\n",
    )
    .map_err(|error| error.to_string())?;
    git(repository, &["init", "-q"])?;
    git(repository, &["config", "user.name", "Milkdrift Evidence"])?;
    git(
        repository,
        &["config", "user.email", "evidence@milkdrift.invalid"],
    )?;
    git(repository, &["add", "calculator.py", "test_calculator.py"])?;
    git(
        repository,
        &["commit", "-q", "-m", "initial disposable fixture"],
    )?;
    Ok((
        git(repository, &["rev-parse", "HEAD"])?,
        git(repository, &["rev-parse", "HEAD^{tree}"])?,
    ))
}

pub fn git(repository: &Path, arguments: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(repository)
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "git {} exited {}: {}",
            arguments.join(" "),
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

pub fn process_sequence(
    agent_capability: &str,
    agent_outputs: &[String],
) -> Result<PromptSequenceDocument, String> {
    let outputs = agent_outputs
        .iter()
        .map(|name| json!({"name":name,"media_type":"application/octet-stream","required":name == "diff"}))
        .collect::<Vec<_>>();
    let value = json!({
        "schema_version":2,
        "sequence":{
            "id":PROCESS_WORKFLOW,
            "title":"External process interoperability evidence",
            "workflow_id":PROCESS_WORKFLOW,
            "repository":{
                "id":"repository:external-evidence",
                "root_ref":"workspace:disposable-external-evidence",
                "starting_revision":"harness-recorded-initial-commit",
                "allowed_paths":["calculator.py","test_calculator.py"],
                "allowed_operations":["read","write","execute","version_control"],
                "dirty_tree":"allow_recorded",
                "isolation":"shared_sequential",
                "cleanup":"retain_accepted",
                "artifacts":{"require_starting_state":true,"require_diff":true,"require_verification_evidence":true},
                "credential_refs":[],"remote_access_refs":[]
            },
            "stages":[{
                "id":"repair",
                "title":"Repair the disposable calculator",
                "prompt":{"type":"inline_markdown","content":"Inspect this disposable Python repository. Fix calculator.add so the existing unittest passes. Keep the change bounded, run the test, do not commit, and report the result.\n"},
                "session":"fresh",
                "coding":{"capability":agent_capability,"operation":"process.execute","provider_profile":null,"execution_trust":"trusted_host_process","maximum_side_effect":"unknown"},
                "verification":{
                    "profile":{"capability":"evidence-verifier-weak","operation":"process.execute","provider_profile":null,"execution_trust":"trusted_host_process","maximum_side_effect":"read_only"},
                    "checks":["python.unittest","git.diff"],
                    "success_artifact":"verification_pass",
                    "result_artifact":"verification_result",
                    "log_artifact":"verification_logs"
                },
                "failure":"pause_for_review",
                "reviewer":{"capability":"evidence-reviewer","operation":"process.execute","provider_profile":null,"execution_trust":"trusted_host_process","maximum_side_effect":"read_only"},
                "approval":"shared_control_path",
                "context_policy_ref":"context:external-evidence-process-v1",
                "outputs":outputs
            }],
            "budget":{"max_review_loops":2},
            "extensions":{"org.milkdrift/external-evidence":{"controlled_verifier_fault":true}}
        }
    });
    PromptSequenceDocument::from_json(
        &serde_json::to_vec(&value).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())
}

pub fn model_profile_facts(bytes: &[u8]) -> Result<ModelProfileFacts, String> {
    let profile = EndpointProfile::from_json(bytes).map_err(|error| error.to_string())?;
    let value: Value = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    let profile_id = profile.identity().as_str().to_owned();
    let revision = value
        .get("revision")
        .and_then(Value::as_u64)
        .ok_or_else(|| "model profile revision is absent".to_owned())?;
    let protocol = value
        .get("protocol")
        .and_then(|value| value.get("type"))
        .and_then(Value::as_str)
        .ok_or_else(|| "model protocol is absent".to_owned())?
        .to_owned();
    let model_alias = value
        .get("model")
        .and_then(Value::as_str)
        .ok_or_else(|| "model alias is absent".to_owned())?
        .to_owned();
    let base = url::Url::parse(
        value
            .get("base_url")
            .and_then(Value::as_str)
            .ok_or_else(|| "model base URL is absent".to_owned())?,
    )
    .map_err(|error| error.to_string())?;
    let host = match base.host() {
        Some(url::Host::Ipv6(address)) => format!("[{address}]"),
        Some(url::Host::Ipv4(address)) => address.to_string(),
        Some(url::Host::Domain(name)) => name.to_owned(),
        None => return Err("model endpoint host is absent".to_owned()),
    };
    let port = base
        .port()
        .map(|value| format!(":{value}"))
        .unwrap_or_default();
    let endpoint_origin = format!("{}://{host}{port}", base.scheme());
    let features = value
        .get("features")
        .and_then(Value::as_array)
        .ok_or_else(|| "model features are absent".to_owned())?;
    let has = |feature: &str| features.iter().any(|value| value.as_str() == Some(feature));
    let mut secret_refs = BTreeSet::new();
    if let Some(secret) = value
        .get("auth")
        .and_then(|auth| auth.get("secret"))
        .and_then(Value::as_str)
    {
        secret_refs.insert(secret.to_owned());
    }
    Ok(ModelProfileFacts {
        profile_id,
        revision,
        protocol,
        model_alias,
        endpoint_origin,
        streaming: has("streaming"),
        structured_output: has("structured_output"),
        secret_refs,
    })
}

pub fn model_revision(
    model_capability: &str,
    profile: &ModelProfileFacts,
) -> Result<Value, String> {
    let mut operations = vec![
        Mutation::AddNode {
            node: evidence_node(
                "evidence-a",
                "architecture evidence: add must compute a plus b",
            )?,
        },
        Mutation::AddNode {
            node: evidence_node(
                "evidence-b",
                "verification evidence: unittest expects add(2,3)=5",
            )?
            .with_control_input(port("in")?)
            .map_err(|e| e.to_string())?,
        },
        Mutation::AddNode {
            node: evidence_node(
                "evidence-denied",
                "irrelevant candidate: weather is outside the requested causal policy",
            )?
            .with_control_input(port("in")?)
            .map_err(|e| e.to_string())?,
        },
        Mutation::AddNode {
            node: Node::new(
                node("model-release")?,
                NodeKind::SignalWait {
                    signal: OperationId::new("evidence.model.release")
                        .map_err(|e| e.to_string())?,
                },
            )
            .map_err(|e| e.to_string())?
            .with_control_input(port("in")?)
            .map_err(|e| e.to_string())?
            .with_control_output(port("next")?)
            .map_err(|e| e.to_string())?,
        },
        Mutation::AddNode {
            node: model_node(model_capability, profile)?
                .with_control_input(port("in")?)
                .map_err(|e| e.to_string())?,
        },
        Mutation::AddNode {
            node: Node::new(
                node("done")?,
                NodeKind::Terminal {
                    outcome: TerminalOutcome::Success,
                },
            )
            .map_err(|e| e.to_string())?
            .with_control_input(port("in")?)
            .map_err(|e| e.to_string())?,
        },
    ];
    for (id, source, target) in [
        ("a-b", "evidence-a", "evidence-b"),
        ("b-denied", "evidence-b", "evidence-denied"),
        ("denied-release", "evidence-denied", "model-release"),
        ("release-model", "model-release", "model"),
        ("model-done", "model", "done"),
    ] {
        operations.push(Mutation::AddEdge {
            edge: Edge::new(
                EdgeId::new(id).map_err(|e| e.to_string())?,
                EdgeKind::Control,
                node(source)?,
                port("next")?,
                node(target)?,
                port("in")?,
            ),
        });
    }
    let revision = BlueprintRevision::genesis(
        WorkflowId::new(MODEL_WORKFLOW).map_err(|e| e.to_string())?,
        MutationBatch::new(operations).map_err(|e| e.to_string())?,
        AuthorRef::new("human:external-evidence").map_err(|e| e.to_string())?,
        "real external model interoperability evidence",
    )
    .map_err(|e| e.to_string())?;
    let bytes = BlueprintRevisionDocument::new(&revision)
        .to_canonical_json()
        .map_err(|e| e.to_string())?;
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}

fn evidence_node(identity: &str, payload: &str) -> Result<Node, String> {
    let requirement =
        CapabilityRequirement::new(OperationId::new("process.execute").map_err(|e| e.to_string())?)
            .exact(CapabilityId::new("evidence-source").map_err(|e| e.to_string())?)
            .execution_trust(ExecutionTrustClass::TrustedHostProcess)
            .maximum_side_effect(SideEffectClass::ReadOnly);
    let config = TaskConfig::new(requirement, TaskContextPolicy::default())
        .and_then(|config| {
            config.with_output_context_roles(BTreeSet::from([ContextSemanticRole::Evidence]))
        })
        .map_err(|e| e.to_string())?;
    let payload = BoundedJson::new(Value::String(payload.to_owned())).map_err(|e| e.to_string())?;
    let input = DataPort::input(
        schema("milkdrift.evidence-payload")?,
        true,
        Some(BindingSource::Literal { value: payload }),
    )
    .map_err(|e| e.to_string())?;
    Node::new(node(identity)?, NodeKind::Task { config })
        .map_err(|e| e.to_string())?
        .with_control_output(port("next")?)
        .map_err(|e| e.to_string())?
        .with_data_input(port("payload")?, input)
        .map_err(|e| e.to_string())?
        .with_data_output(
            port("evidence")?,
            DataPort::output(schema("milkdrift.artifact-reference")?),
        )
        .map_err(|e| e.to_string())
}

fn model_node(model_capability: &str, profile: &ModelProfileFacts) -> Result<Node, String> {
    let mut requirement =
        CapabilityRequirement::new(OperationId::new("model.generate").map_err(|e| e.to_string())?)
            .exact(CapabilityId::new(model_capability).map_err(|e| e.to_string())?)
            .provider_profile(
                ProviderProfileRef::new(&profile.profile_id).map_err(|e| e.to_string())?,
            )
            .maximum_side_effect(SideEffectClass::None);
    if profile.streaming {
        requirement = requirement.streaming(StreamingMode::OutputFragments);
    }
    let policy = TaskContextPolicy::new(
        false,
        None,
        BTreeSet::from([node("evidence-a")?, node("evidence-b")?]),
        BTreeSet::new(),
        BTreeSet::new(),
        BTreeSet::from([
            ContextCategory::RawProgress,
            ContextCategory::ToolTrace,
            ContextCategory::VerboseCommandOutput,
            ContextCategory::PriorPrompt,
        ]),
        None,
        ContextBudget::default(),
        ContextOrdering::default(),
        ContextTruncation::default(),
        ContextSessionPolicy::Fresh,
        true,
    )
    .map_err(|e| e.to_string())?;
    let config = TaskConfig::new(requirement, policy).map_err(|e| e.to_string())?;
    let task = model_task(profile)?;
    let task_value: Value = serde_json::from_slice(
        &ModelTaskRequestDocument::new(task)
            .to_canonical_json()
            .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let mut result = Node::new(node("model")?, NodeKind::Task { config })
        .map_err(|e| e.to_string())?
        .with_control_output(port("next")?)
        .map_err(|e| e.to_string())?
        .with_data_input(
            port(MODEL_TASK_INPUT_NAME)?,
            DataPort::input(
                schema("milkdrift.model-task")?,
                true,
                Some(BindingSource::Literal {
                    value: BoundedJson::new(task_value).map_err(|e| e.to_string())?,
                }),
            )
            .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    for output in [
        "model_response",
        "final_text",
        "structured_output",
        "tool_calls",
        "provider_metadata",
    ] {
        result = result
            .with_data_output(
                port(output)?,
                DataPort::output(schema("milkdrift.artifact-reference")?),
            )
            .map_err(|e| e.to_string())?;
    }
    Ok(result)
}

fn model_task(profile: &ModelProfileFacts) -> Result<ModelTaskRequest, String> {
    let structured = profile
        .structured_output
        .then(|| {
            StructuredOutput::new(
                "milkdrift_evidence",
                BoundedJson::new(json!({
                    "type":"object",
                    "properties":{"ok":{"type":"boolean"}},
                    "required":["ok"],
                    "additionalProperties":false
                }))
                .map_err(|e| e.to_string())?,
                true,
            )
            .map_err(|e| e.to_string())
        })
        .transpose()?;
    ModelTaskRequest::new(
        vec![Message::new(
            MessageRole::User,
            vec![ContentPart::Text {
                text: if structured.is_some() {
                    "Using only the selected evidence, return JSON with ok=true. Do not repeat the evidence."
                        .to_owned()
                } else {
                    "Using only the selected evidence, respond exactly MILKDRIFT_EVIDENCE_OK and nothing else."
                        .to_owned()
                },
            }],
            None,
        )
        .map_err(|e| e.to_string())?],
        Vec::new(),
        structured,
        SessionSelection::Fresh,
        None,
        64,
        profile.streaming,
        BTreeMap::new(),
    )
    .map_err(|e| e.to_string())
}

fn node(value: &str) -> Result<NodeId, String> {
    NodeId::new(value).map_err(|e| e.to_string())
}

fn port(value: &str) -> Result<PortId, String> {
    PortId::new(value).map_err(|e| e.to_string())
}

fn schema(value: &str) -> Result<SchemaRef, String> {
    SchemaRef::new(SchemaId::new(value).map_err(|e| e.to_string())?, 1).map_err(|e| e.to_string())
}
