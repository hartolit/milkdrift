//! Disposable repository and typed ordinary task requests for external evidence.
use milkdrift_control_protocol::tasks::{
    CreateWorkflowRequest, InputBinding, InputValue, NewTaskDto, PathAccess, TaskCapability,
    TaskDefinitionDto, TaskOutputDeclarationDto,
};
use milkdrift_model_provider::EndpointProfile;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
    process::Command,
};
pub(crate) const PROCESS_WORKFLOW: &str = "External process interoperability evidence";
pub(crate) const MODEL_WORKFLOW: &str = "External model interoperability evidence";
pub(crate) struct ModelProfileFacts {
    pub(crate) profile_id: String,
    pub(crate) revision: u64,
    pub(crate) protocol: String,
    pub(crate) model_alias: String,
    pub(crate) endpoint_origin: String,
    pub(crate) digest: String,
    pub(crate) secret_refs: BTreeSet<String>,
}
pub(crate) fn initialize_repository(repository: &Path) -> Result<(String, String), String> {
    fs::create_dir_all(repository).map_err(|error| error.to_string())?;
    fs::write(repository.join(".gitignore"), "/target/\n").map_err(|error| error.to_string())?;
    fs::write(
        repository.join("calculator.rs"),
        "pub fn add(a: i32, b: i32) -> i32 { a - b }\n",
    )
    .map_err(|error| error.to_string())?;
    fs::write(
        repository.join("test_calculator.rs"),
        "mod calculator;\n#[test]\nfn addition() { assert_eq!(calculator::add(2, 3), 5); }\n",
    )
    .map_err(|error| error.to_string())?;
    git(repository, &["init", "-q"])?;
    git(repository, &["config", "user.name", "Milkdrift Evidence"])?;
    git(
        repository,
        &["config", "user.email", "evidence@milkdrift.invalid"],
    )?;
    git(
        repository,
        &["add", ".gitignore", "calculator.rs", "test_calculator.rs"],
    )?;
    git(
        repository,
        &["commit", "-q", "-m", "initial disposable fixture"],
    )?;
    Ok((
        git(repository, &["rev-parse", "HEAD"])?,
        git(repository, &["rev-parse", "HEAD^{tree}"])?,
    ))
}

pub(crate) fn git(repository: &Path, arguments: &[&str]) -> Result<String, String> {
    let output = milkdrift_evidence::application::run_command(
        Command::new("git").args(arguments).current_dir(repository),
        None,
        std::time::Duration::from_secs(10),
    )
    .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "git {} exited {}: {}",
            arguments.join(" "),
            output.status.code().unwrap_or(-1),
            output.stderr.trim()
        ));
    }
    Ok(output.stdout.trim().to_owned())
}

pub(crate) fn model_profile_facts(bytes: &[u8]) -> Result<ModelProfileFacts, String> {
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
        digest: format!("b3_{}", blake3::hash(bytes)),
        secret_refs,
    })
}

pub(crate) fn definition(
    capability: &str,
    title: &str,
    instructions: &str,
    repository: Option<&Path>,
) -> TaskDefinitionDto {
    TaskDefinitionDto {
        title: title.into(),
        description: title.into(),
        instructions: instructions.into(),
        capability: TaskCapability {
            id: capability.into(),
            operation: if repository.is_some() {
                "process.execute"
            } else {
                "model.generate"
            }
            .into(),
        },
        inputs: BTreeMap::new(),
        outputs: BTreeMap::new(),
        dependencies: vec![],
        required: true,
        corrective_of: None,
        paths: repository
            .map(|path| {
                vec![PathAccess {
                    path: path.to_string_lossy().into_owned(),
                    write: true,
                }]
            })
            .unwrap_or_default(),
    }
}
pub(crate) fn output(
    definition: &mut TaskDefinitionDto,
    name: &str,
    media: Option<&str>,
    max_bytes: u64,
) {
    definition.outputs.insert(
        name.into(),
        TaskOutputDeclarationDto {
            required: true,
            media_type: media.map(str::to_owned),
            max_bytes,
        },
    );
}
pub(crate) fn text(value: &str) -> InputBinding {
    InputBinding::Inline {
        value: InputValue::Text { text: value.into() },
    }
}
pub(crate) fn process_request(capability: &str, repository: &Path) -> CreateWorkflowRequest {
    let prompt = "Inspect this disposable Rust repository. Fix calculator::add so the existing Rust test passes. Create the ignored target/ directory, compile test_calculator.rs with rustc --test and direct its -o output into target/, then run that executable. Keep source changes bounded, do not commit, and report the result.";
    let mut coding = definition(capability, "Initial coding", prompt, Some(repository));
    coding.inputs.insert("prompt".into(), text(prompt));
    output(&mut coding, "diff", None, 1_048_576);
    let mut verify = definition(
        "evidence-verifier-weak",
        "Controlled verification",
        "Run the independent verification with the declared controlled check failure.",
        Some(repository),
    );
    verify.dependencies.push("coding".into());
    output(
        &mut verify,
        "verification_result",
        Some("application/json"),
        262_144,
    );
    output(
        &mut verify,
        "verification_logs",
        Some("text/plain"),
        1_048_576,
    );
    output(
        &mut verify,
        "verification_acceptance",
        Some("text/plain"),
        4_096,
    );
    let mut review = definition(
        "evidence-reviewer",
        "Independent review",
        "Read the settled verification outcome and review the need for a corrective task.",
        Some(repository),
    );
    review.dependencies.push("verify".into());
    review.inputs.insert(
        "verification".into(),
        InputBinding::SettledOutcome {
            task: "verify".into(),
        },
    );
    output(&mut review, "review", Some("application/json"), 524_288);
    output(
        &mut review,
        "remediation_proposal",
        Some("application/json"),
        524_288,
    );
    CreateWorkflowRequest {
        request_id: "external-process-create".into(),
        title: PROCESS_WORKFLOW.into(),
        description: "Controlled failure, independent review and explicit corrective future work"
            .into(),
        tasks: vec![
            NewTaskDto {
                alias: "coding".into(),
                definition: coding,
            },
            NewTaskDto {
                alias: "verify".into(),
                definition: verify,
            },
            NewTaskDto {
                alias: "review".into(),
                definition: review,
            },
        ],
        inputs: BTreeMap::new(),
        finish_when_settled: false,
        limits: None,
    }
}
pub(crate) fn model_request(capability: &str, repository: &Path) -> CreateWorkflowRequest {
    let mut tasks = Vec::new();
    for (alias, payload) in [
        (
            "evidence-a",
            "Architecture evidence: add must compute a plus b",
        ),
        (
            "evidence-b",
            "Verification evidence: Rust test expects add(2,3)=5",
        ),
        ("unselected", "UNSELECTED_PRIVATE_EVIDENCE"),
    ] {
        let mut source = definition(
            "evidence-source",
            alias,
            "Publish the exact named evidence",
            Some(repository),
        );
        source.inputs.insert("payload".into(), text(payload));
        if alias == "evidence-b" {
            source.dependencies.push("evidence-a".into());
        }
        if alias == "unselected" {
            source.dependencies.push("evidence-b".into());
        }
        output(&mut source, "evidence", Some("text/plain"), 65_536);
        tasks.push(NewTaskDto {
            alias: alias.into(),
            definition: source,
        });
    }
    let mut model = definition(
        capability,
        "Model analysis",
        "Using only the explicitly selected evidence, respond exactly MILKDRIFT_EVIDENCE_OK and nothing else.",
        None,
    );
    for name in ["evidence-a", "evidence-b"] {
        model.dependencies.push(name.into());
        model.inputs.insert(
            name.into(),
            InputBinding::TaskOutput {
                task: name.into(),
                output: "evidence".into(),
                optional: false,
            },
        );
    }
    model.inputs.insert(
        "release".into(),
        InputBinding::Supplied {
            name: "release".into(),
            optional: false,
        },
    );
    for (name, media, max_bytes) in [
        (
            "model_response",
            "application/vnd.milkdrift.model-response.v1+json",
            1_048_576,
        ),
        ("final_text", "text/plain", 524_288),
        (
            "provider_metadata",
            "application/vnd.milkdrift.model-provider-metadata.v1+json",
            262_144,
        ),
    ] {
        output(&mut model, name, Some(media), max_bytes);
    }
    tasks.push(NewTaskDto {
        alias: "model".into(),
        definition: model,
    });
    CreateWorkflowRequest {
        request_id: "external-model-create".into(),
        title: MODEL_WORKFLOW.into(),
        description: "Explicit immutable input selection across a durable pre-entry wait".into(),
        tasks,
        inputs: BTreeMap::new(),
        finish_when_settled: true,
        limits: None,
    }
}
