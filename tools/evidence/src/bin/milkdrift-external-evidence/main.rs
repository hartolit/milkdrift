//! Qualify a real coding agent and model endpoint through ordinary daemon operations.
//!
//! Operator profiles supply external resources; generated workflows exercise verification,
//! remediation, selected context, and settled-boundary restart. The report owner validates
//! exact source/scenario evidence and redaction. Fixture mode tests this harness and always
//! remains non-qualifying, even when every scenario assertion succeeds.

mod profiles;
mod report;
mod workflows;

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write as _,
    net::{Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    process::ExitCode,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use clap::{ArgAction, Parser, ValueEnum};
use milkdrift_authority::{
    AccessMode, ArtifactAuthorityScope, AuthorityBudget, BoundaryTimeMillis,
    CapabilityAuthorityScopeBuilder, DaemonAuthorityScope, FilesystemScope, LayoutAuthorityScope,
    NetworkProfileRef, NetworkScope, PeerAuthorityScope, ResourceScope, SecretRef, Selection,
    WorkflowRunScope, WorkspaceAuthorityScope,
};
use milkdrift_blueprint::WorkflowId;
use milkdrift_capability::{CapabilityId, OperationId, SideEffectClass};
use milkdrift_control_client::{ClientError, ControlClient};
use milkdrift_control_protocol::{Command, CommandRequest, ProtocolVersion, decode_json};
use milkdrift_daemon::{
    ActorBindingConfig, ActorGrantConfig, AdapterConfig, ApplicationReceiptConfig,
    AuthorityPresetConfig, DaemonConfig, ModelProfileConfig, PeerHostConfig, RuntimeHostConfig,
    SecretSourceConfig, ShutdownConfig, ShutdownEffectPolicy,
};
use milkdrift_evidence::application::{DaemonLaunch, application_binary, reserve_endpoint};
use milkdrift_model::ModelResponseDocument;
use milkdrift_prompt_sequence::{
    PromptSource, RemediationProposalSpec, build_remediation_proposal,
};
use milkdrift_workspace::ArtifactSensitivity;
use profiles::{AgentProfile, GeneratedProfiles, generated_profiles, prepare_agent_profile};
use report::{
    ArtifactEvidence, EvidenceReport, MilkdriftEvidence, PlatformEvidence, REPORT_SCHEMA_VERSION,
    RestartEvidence, ScenarioEvidence, ValidationEvidence, write_report,
};
use serde_json::{Value, json};
use workflows::{MODEL_RUN, MODEL_WORKFLOW, PROCESS_RUN, PROCESS_WORKFLOW};

type HarnessResult<T = ()> = Result<T, String>;

fn client_error(error: ClientError) -> String {
    match error {
        ClientError::Api(envelope) => format!(
            "daemon API {:?}: {} ({:?})",
            envelope.code, envelope.message, envelope.details
        ),
        other => other.to_string(),
    }
}

#[derive(Parser, Debug)]
#[command(
    name = "milkdrift-external-evidence",
    about = "Operator-driven, redacted real process/model interoperability evidence"
)]
struct Arguments {
    /// Built daemon executable; defaults to the sibling Cargo-profile binary.
    #[arg(long)]
    daemon: Option<PathBuf>,
    /// Operator-supplied byte-pinned real coding-agent process profile.
    #[arg(long)]
    agent_profile: Option<PathBuf>,
    /// Operator-supplied non-secret model endpoint profile.
    #[arg(long)]
    model_profile: Option<PathBuf>,
    /// Capability identity to register for the endpoint profile.
    #[arg(long, default_value = "external-evidence-model")]
    model_capability: String,
    /// Selected evidence output directory; repository target/ is allowed.
    #[arg(long)]
    output: PathBuf,
    /// One direct argument passed to the exact agent executable for version evidence.
    #[arg(long, action=ArgAction::Append, allow_hyphen_values=true)]
    agent_version_arg: Vec<String>,
    /// Map `secret:ref=env:VARIABLE` or `secret:ref=file:/absolute/path` without values.
    #[arg(long, action=ArgAction::Append)]
    secret_source: Vec<String>,
    /// Run deterministic local process/mock endpoint harness validation.
    #[arg(long)]
    fixture: bool,
    /// Required acknowledgement that fixture evidence is non-qualifying.
    #[arg(long)]
    allow_fixture: bool,
    /// Hermetic contract-test fault injection; never accepted as qualifying evidence.
    #[arg(long, value_enum, requires = "fixture", hide = true)]
    fixture_failure: Option<FixtureFailure>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum FixtureFailure {
    Process,
    Model,
}

struct MockEndpoint {
    address: SocketAddr,
    requests: Arc<AtomicUsize>,
    request_lines: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    task: Option<std::thread::JoinHandle<()>>,
}

impl Drop for MockEndpoint {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(task) = self.task.take() {
            let _ = task.join();
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let arguments = Arguments::parse();
    match execute(arguments).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("milkdrift-external-evidence: {error}");
            ExitCode::from(1)
        }
    }
}

async fn execute(arguments: Arguments) -> HarnessResult {
    if arguments.fixture && !arguments.allow_fixture {
        return Err("--fixture requires --allow-fixture and remains non-qualifying".to_owned());
    }
    let output = validate_output_path(&arguments.output)?;
    fs::create_dir_all(&output).map_err(|error| error.to_string())?;
    if fs::read_dir(&output)
        .map_err(|error| error.to_string())?
        .next()
        .is_some()
    {
        return Err("selected output directory must be empty".to_owned());
    }
    let report_path = output.join("report.json");
    let session_root = output.join("session");
    let repository = session_root.join("repository");
    fs::create_dir_all(&session_root).map_err(|error| error.to_string())?;
    let now = unix_millis()?;
    let operator_secrets = forbidden_secret_values(&arguments.secret_source)?;
    let (milkdrift_commit, milkdrift_tree, dirty) = milkdrift_git_facts()?;
    let mut report = EvidenceReport {
        schema_version: REPORT_SCHEMA_VERSION,
        generated_at_unix_ms: now,
        platform: PlatformEvidence {
            os: std::env::consts::OS.to_owned(),
            architecture: std::env::consts::ARCH.to_owned(),
            build_target: rust_build_target()?,
        },
        milkdrift: MilkdriftEvidence {
            starting_commit: milkdrift_commit,
            starting_tree: milkdrift_tree,
            workspace_version: env!("CARGO_PKG_VERSION").to_owned(),
            dirty_at_start: dirty,
        },
        configuration_digest: None,
        qualifying: false,
        fixture_mode: arguments.fixture,
        process: ScenarioEvidence::pending("process scenario has not run"),
        model: ScenarioEvidence::pending("model scenario has not run"),
        validation: Vec::new(),
        redactions: vec![
            "secret values and authorization headers".to_owned(),
            "complete prompts and model/process outputs".to_owned(),
            "private repository file contents".to_owned(),
            "endpoint paths, queries, fragments, and user information".to_owned(),
        ],
        failure_reason: None,
    };
    let result = run_scenarios(
        &arguments,
        &output,
        &session_root,
        &repository,
        &mut report,
        &operator_secrets,
    )
    .await;
    if let Err(error) = &result {
        report.failure_reason = Some(error.clone());
    }
    report.qualifying = report.process.qualifying && report.model.qualifying;
    write_report(&report_path, &report, &operator_secrets)?;
    redaction_check(&report_path, &operator_secrets)?;
    result
}

async fn run_scenarios(
    arguments: &Arguments,
    output: &Path,
    session_root: &Path,
    repository: &Path,
    report: &mut EvidenceReport,
    operator_secrets: &[Vec<u8>],
) -> HarnessResult {
    let (repository_initial_commit, repository_initial_tree) =
        workflows::initialize_repository(repository)?;
    let agent = prepare_agent_profile(
        arguments.agent_profile.as_deref(),
        arguments.fixture,
        repository,
        session_root,
        &arguments.agent_version_arg,
    )?;
    let helpers = generated_profiles(repository, session_root)?;
    let mock = if arguments.fixture {
        Some(start_mock_endpoint().await?)
    } else {
        None
    };
    let model_profile_path = if let Some(mock) = &mock {
        write_fixture_model_profile(session_root, mock.address)?
    } else {
        arguments
            .model_profile
            .clone()
            .ok_or_else(|| "--model-profile is required".to_owned())?
    };
    let model_bytes =
        fs::read(&model_profile_path).map_err(|error| format!("model profile read: {error}"))?;
    let model_facts = workflows::model_profile_facts(&model_bytes)?;
    let sources = secret_sources(arguments, session_root, &agent, &model_facts.secret_refs)?;
    let (config, configuration_digest, process_token, model_token) = configuration(
        arguments
            .daemon
            .clone()
            .map(Ok)
            .unwrap_or_else(|| application_binary("milkdrift-daemon"))
            .map_err(|error| error.to_string())?,
        session_root,
        repository,
        &agent,
        &helpers,
        &model_profile_path,
        &arguments.model_capability,
        &model_facts,
        sources,
    )?;
    report.configuration_digest = Some(configuration_digest);
    if !arguments.fixture && report.milkdrift.dirty_at_start {
        return Err(
            "qualifying real evidence requires a clean Milkdrift checkout so commit/tree identify the tested source"
                .to_owned(),
        );
    }
    report.validation.extend([
        ValidationEvidence {
            command: "process profile schema + executable identity validation".to_owned(),
            exit_status: 0,
        },
        ValidationEvidence {
            command: "model endpoint profile schema validation".to_owned(),
            exit_status: 0,
        },
        ValidationEvidence {
            command: "daemon configuration validation before storage open".to_owned(),
            exit_status: 0,
        },
    ]);

    if arguments.fixture_failure == Some(FixtureFailure::Process) {
        let reason = "fixture-injected process scenario failure".to_owned();
        report.process = ScenarioEvidence::failed(reason.clone());
        return Err(reason);
    }

    report.process = match run_process_scenario(
        &config,
        &process_token,
        &agent,
        &repository_initial_commit,
        &repository_initial_tree,
        repository,
        arguments.fixture,
    )
    .await
    {
        Ok(evidence) => evidence,
        Err(error) => {
            report.process = ScenarioEvidence::failed(error.clone());
            return Err(error);
        }
    };
    if arguments.fixture_failure == Some(FixtureFailure::Model) {
        let reason = "fixture-injected model scenario failure".to_owned();
        report.model = ScenarioEvidence::failed(reason.clone());
        return Err(reason);
    }
    if let Some(mock) = &mock {
        let stream = tokio::net::TcpStream::connect(mock.address)
            .await
            .map_err(|error| {
                format!(
                    "fixture model endpoint stopped before model scenario (task_finished={}): {error}",
                    mock.task.as_ref().is_none_or(std::thread::JoinHandle::is_finished)
                )
            })?;
        drop(stream);
    }
    report.model = match run_model_scenario(
        &config,
        &model_token,
        &arguments.model_capability,
        &model_facts,
        mock.as_ref().map(|value| value.requests.clone()),
        mock.as_ref().map(|value| value.request_lines.clone()),
        arguments.fixture,
    )
    .await
    {
        Ok(evidence) => evidence,
        Err(error) => {
            report.model = ScenarioEvidence::failed(error.clone());
            return Err(error);
        }
    };
    if let Some(mut mock) = mock {
        mock.stop.store(true, Ordering::SeqCst);
        if let Some(task) = mock.task.take() {
            task.join()
                .map_err(|_| "fixture model endpoint thread panicked".to_owned())?;
        }
    }
    let mut forbidden = operator_secrets.to_vec();
    forbidden.push(process_token.as_bytes().to_vec());
    forbidden.push(model_token.as_bytes().to_vec());
    let temporary = output.join("report.preflight.json");
    write_report(&temporary, report, &forbidden)?;
    redaction_check(&temporary, &forbidden)?;
    fs::remove_file(temporary).map_err(|error| error.to_string())?;
    Ok(())
}

#[allow(clippy::too_many_arguments)] // Harness assembly keeps independently audited paths, profiles, and secrets explicit.
fn configuration(
    executable: PathBuf,
    session_root: &Path,
    repository: &Path,
    agent: &AgentProfile,
    helpers: &GeneratedProfiles,
    model_profile_path: &Path,
    model_capability: &str,
    model: &workflows::ModelProfileFacts,
    mut secret_sources: BTreeMap<String, SecretSourceConfig>,
) -> HarnessResult<(DaemonLaunch, String, String, String)> {
    let now = unix_millis()?;
    let process_token = format!("process-{}-{now}", std::process::id());
    let model_token = format!("model-{}-{now}", std::process::id());
    let process_token_path = session_root.join("process.token");
    let model_token_path = session_root.join("model.token");
    milkdrift_evidence::application::write_private(&process_token_path, process_token.as_bytes())
        .map_err(|error| error.to_string())?;
    milkdrift_evidence::application::write_private(&model_token_path, model_token.as_bytes())
        .map_err(|error| error.to_string())?;
    secret_sources.insert(
        "credential:external-process".to_owned(),
        SecretSourceConfig::File {
            path: process_token_path,
        },
    );
    secret_sources.insert(
        "credential:external-model".to_owned(),
        SecretSourceConfig::File {
            path: model_token_path,
        },
    );
    let process_caps = BTreeSet::from([
        agent.capability.clone(),
        "evidence-verifier-weak".to_owned(),
        "evidence-verifier-good".to_owned(),
        "evidence-reviewer".to_owned(),
        "milkdrift-workflow-control".to_owned(),
    ]);
    let model_caps = BTreeSet::from([
        model_capability.to_owned(),
        "evidence-source".to_owned(),
        "milkdrift-workflow-control".to_owned(),
    ]);
    let actor = |credential: &str,
                 actor: &str,
                 grant: &str,
                 workflow: &str,
                 capabilities: &BTreeSet<String>,
                 model_profile: Option<&str>|
     -> HarnessResult<ActorBindingConfig> {
        Ok(ActorBindingConfig {
            credential_ref: credential.to_owned(),
            actor: actor.to_owned(),
            grant_id: grant.to_owned(),
            grant_revision: 1,
            revocation_generation: 0,
            preset: AuthorityPresetConfig::Controller,
            authority: explicit_grant(
                workflow,
                capabilities,
                model_profile,
                session_root,
                repository,
                agent,
                helpers,
                model,
                &secret_sources,
            )?,
            enabled: true,
        })
    };
    let actors = vec![
        actor(
            "credential:external-process",
            "human:external-process",
            "grant:external-process",
            PROCESS_WORKFLOW,
            &process_caps,
            None,
        )?,
        actor(
            "credential:external-model",
            "human:external-model",
            "grant:external-model",
            MODEL_WORKFLOW,
            &model_caps,
            Some(&model.profile_id),
        )?,
    ];
    let config = DaemonConfig {
        schema_version: milkdrift_daemon::DAEMON_CONFIG_SCHEMA_VERSION,
        data_root: session_root.join("data"),
        bind: reserve_endpoint().map_err(|error| error.to_string())?,
        secret_sources,
        actors,
        runtime: RuntimeHostConfig {
            maintenance_interval_ms: 10,
            ..RuntimeHostConfig::default()
        },
        adapters: AdapterConfig {
            process_profiles: vec![
                agent.path.clone(),
                helpers.weak_verifier.clone(),
                helpers.good_verifier.clone(),
                helpers.reviewer.clone(),
                helpers.evidence_source.clone(),
            ],
            model_profiles: vec![ModelProfileConfig {
                capability_id: model_capability.to_owned(),
                profile: model_profile_path.to_owned(),
            }],
        },
        peers: PeerHostConfig::default(),
        shutdown: ShutdownConfig {
            deadline_ms: 30_000,
            effect_policy: ShutdownEffectPolicy::Retain,
        },
        application_receipts: ApplicationReceiptConfig {
            hot_receipt_bound: 1_000,
            archive_batch_size: 64,
        },
        security_audit_record_bound: 2_000,
    };
    let configuration_digest = config
        .clone()
        .validate(session_root)
        .map_err(|error| error.to_string())?
        .normalized_digest()
        .to_owned();
    let launch = DaemonLaunch::write(executable, session_root, &config)
        .map_err(|error| error.to_string())?;
    Ok((launch, configuration_digest, process_token, model_token))
}

#[allow(clippy::too_many_arguments)] // Evidence grants spell out every bounded resource input at the audit boundary.
fn explicit_grant(
    workflow: &str,
    capabilities: &BTreeSet<String>,
    model_profile: Option<&str>,
    session_root: &Path,
    repository: &Path,
    agent: &AgentProfile,
    helpers: &GeneratedProfiles,
    model: &workflows::ModelProfileFacts,
    configured_secrets: &BTreeMap<String, SecretSourceConfig>,
) -> HarnessResult<ActorGrantConfig> {
    let now = unix_millis()?;
    let identities = capabilities
        .iter()
        .map(|value| CapabilityId::new(value).map_err(|error| error.to_string()))
        .collect::<Result<BTreeSet<_>, _>>()?;
    let mut operations = BTreeSet::from([
        OperationId::new("process.execute").map_err(|error| error.to_string())?,
        OperationId::new("workflow.inspect").map_err(|error| error.to_string())?,
    ]);
    if model_profile.is_some() {
        operations.insert(OperationId::new("model.generate").map_err(|error| error.to_string())?);
    }
    let builder = CapabilityAuthorityScopeBuilder::new(SideEffectClass::Unknown)
        .only_capabilities(identities)
        .map_err(|error| error.to_string())?
        .only_operations(operations)
        .map_err(|error| error.to_string())?;
    // The same model workflow also contains local evidence-source tasks whose requirements
    // intentionally have no provider profile. The model task itself pins the exact profile and
    // the report verifies that frozen identity through attempt provenance.
    let session_root = session_root
        .canonicalize()
        .map_err(|error| format!("session root canonicalization: {error}"))?;
    let repository = repository
        .canonicalize()
        .map_err(|error| format!("repository root canonicalization: {error}"))?;
    let mut filesystem = vec![
        FilesystemScope::from_canonical_host_path(
            &session_root,
            BTreeSet::from([AccessMode::Read, AccessMode::Write, AccessMode::Execute]),
        )
        .map_err(|error| error.to_string())?,
        FilesystemScope::from_canonical_host_path(
            &repository,
            BTreeSet::from([AccessMode::Read, AccessMode::Write, AccessMode::Execute]),
        )
        .map_err(|error| error.to_string())?,
    ];
    // A real agent need not share the interpreter used by the generated evidence helpers.
    for executable in [&agent.canonical_executable, &helpers.canonical_executable] {
        let scope = FilesystemScope::from_canonical_host_path(
            executable
                .parent()
                .ok_or_else(|| "process executable has no parent".to_owned())?,
            BTreeSet::from([AccessMode::Execute]),
        )
        .map_err(|error| error.to_string())?;
        if !filesystem.contains(&scope) {
            filesystem.push(scope);
        }
    }
    let network = if model_profile.is_some() {
        let destination = model
            .endpoint_origin
            .split_once("://")
            .map(|(_, value)| value.to_owned())
            .ok_or_else(|| "model endpoint origin is invalid".to_owned())?;
        NetworkScope::new(
            BTreeSet::from([NetworkProfileRef::new(model.profile_id.clone())
                .map_err(|error| error.to_string())?]),
            BTreeSet::from([destination]),
        )
        .map_err(|error| error.to_string())?
    } else {
        NetworkScope::new(BTreeSet::new(), BTreeSet::new()).map_err(|error| error.to_string())?
    };
    let required_secrets = if model_profile.is_some() {
        &model.secret_refs
    } else {
        &agent.secret_refs
    };
    let secrets = required_secrets
        .iter()
        .filter(|key| configured_secrets.contains_key(*key))
        .map(|key| SecretRef::new(key).map_err(|error| error.to_string()))
        .collect::<Result<BTreeSet<_>, _>>()?;
    let artifacts = ArtifactAuthorityScope::new(
        Selection::any(),
        BTreeSet::from([
            ArtifactSensitivity::Public,
            ArtifactSensitivity::Internal,
            ArtifactSensitivity::Restricted,
        ]),
    )
    .map_err(|error| error.to_string())?;
    Ok(ActorGrantConfig {
        resources: ResourceScope {
            workflow_run: WorkflowRunScope::Workflow {
                workflow: WorkflowId::new(workflow).map_err(|error| error.to_string())?,
            },
            capability: builder.build(),
            filesystem,
            network,
            secrets,
            artifacts,
            layouts: LayoutAuthorityScope::none(),
            peers: PeerAuthorityScope::none(),
            daemon: DaemonAuthorityScope {
                readiness: true,
                detailed_health: true,
                own_authority: true,
                configuration: false,
                audit: false,
            },
            workspace: WorkspaceAuthorityScope::dangerous_all_in_run(),
        },
        budget: AuthorityBudget {
            cost_minor: Some(1_000_000_000),
            duration_ms: Some(604_800_000),
            invocations: Some(10_000),
            artifact_bytes: Some(16 * 1_073_741_824),
            units: Some(1_000_000_000),
            concurrency: Some(32),
        },
        valid_from: BoundaryTimeMillis::new(now.saturating_sub(60_000)),
        valid_until: BoundaryTimeMillis::new(now.saturating_add(86_400_000)),
        // Dynamic artifact/workspace identities require explicit run-wide acknowledgement;
        // workflow, capability, operation, filesystem, network, secret, time, and budgets remain exact.
        dangerous_allow_broad_authority: true,
    })
}

fn secret_sources(
    arguments: &Arguments,
    session_root: &Path,
    agent: &AgentProfile,
    model_refs: &BTreeSet<String>,
) -> HarnessResult<BTreeMap<String, SecretSourceConfig>> {
    let mut result = BTreeMap::new();
    for mapping in &arguments.secret_source {
        let (reference, source) = mapping
            .split_once('=')
            .ok_or_else(|| "secret source mapping requires reference=source".to_owned())?;
        if let Some(variable) = source.strip_prefix("env:") {
            if std::env::var_os(variable).is_none() {
                return Err(format!(
                    "required secret environment variable {variable} is not set"
                ));
            }
            result.insert(
                reference.to_owned(),
                SecretSourceConfig::Environment {
                    variable: variable.to_owned(),
                },
            );
        } else if let Some(path) = source.strip_prefix("file:") {
            result.insert(
                reference.to_owned(),
                SecretSourceConfig::File {
                    path: PathBuf::from(path),
                },
            );
        } else {
            return Err("secret source must use env: or file:".to_owned());
        }
    }
    for required in agent.secret_refs.iter().chain(model_refs) {
        if !result.contains_key(required) {
            return Err(format!(
                "missing --secret-source mapping for required reference {required}"
            ));
        }
    }
    if result.is_empty() {
        // Daemon config requires at least one source; this private unused sentinel carries no provider value.
        let sentinel = session_root.join("unused-secret-sentinel");
        milkdrift_evidence::application::write_private(&sentinel, b"unused")
            .map_err(|error| error.to_string())?;
        result.insert(
            "secret:unused-evidence-sentinel".to_owned(),
            SecretSourceConfig::File { path: sentinel },
        );
    }
    Ok(result)
}

async fn start_mock_endpoint() -> HarnessResult<MockEndpoint> {
    let requests = Arc::new(AtomicUsize::new(0));
    let request_lines = Arc::new(Mutex::new(Vec::new()));
    let listener =
        std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|error| error.to_string())?;
    let address = listener.local_addr().map_err(|error| error.to_string())?;
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = stop.clone();
    let thread_requests = requests.clone();
    let thread_request_lines = request_lines.clone();
    let task = std::thread::spawn(move || {
        while !thread_stop.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let _ =
                        serve_mock_connection(&mut stream, &thread_requests, &thread_request_lines);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(_) => break,
            }
        }
    });
    Ok(MockEndpoint {
        address,
        requests,
        request_lines,
        stop,
        task: Some(task),
    })
}

fn serve_mock_connection(
    stream: &mut std::net::TcpStream,
    requests: &AtomicUsize,
    request_lines: &Mutex<Vec<String>>,
) -> std::io::Result<()> {
    let request = milkdrift_evidence::http_fixture::read_request(stream)?;
    let request_line = request.lines().next().unwrap_or_default().to_owned();
    {
        let mut lines = request_lines
            .lock()
            .map_err(|_| std::io::Error::other("fixture request log is poisoned"))?;
        if lines.len() == 64 {
            return Err(std::io::Error::other("fixture request log exceeds bound"));
        }
        lines.push(request_line.clone());
    }
    let exact_path = request_line == "POST /v1/chat/completions HTTP/1.1";
    if !exact_path {
        stream.write_all(
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )?;
        return Ok(());
    }
    requests.fetch_add(1, Ordering::SeqCst);
    let body = concat!(
        "data: {\"id\":\"fixture-response\",\"model\":\"fixture-model\",\"choices\":[{\"delta\":{\"content\":\"{\\\"ok\\\":\"},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"fixture-response\",\"model\":\"fixture-model\",\"choices\":[{\"delta\":{\"content\":\"true}\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":19,\"completion_tokens\":4}}\n\n",
        "data: [DONE]\n\n"
    );
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes())
}

fn write_fixture_model_profile(session_root: &Path, address: SocketAddr) -> HarnessResult<PathBuf> {
    let value = json!({
        "schema_version":1,
        "identity":"fixture-external-model",
        "revision":1,
        "protocol":{"type":"open_ai_compatible","path":"v1/chat/completions"},
        "base_url":format!("http://{address}"),
        "model":"fixture-model",
        "auth":{"type":"no_auth"},
        "limits":{"connect_timeout_ms":2000,"request_timeout_ms":10000,"idle_timeout_ms":5000,"max_headers":64,"max_header_bytes":16384,"max_request_bytes":1048576,"max_response_bytes":1048576,"max_stream_line_bytes":65536,"max_stream_event_bytes":131072,"max_fragment_bytes":4096},
        "redirect":"deny","tls":"web_pki_roots","proxy":"disabled",
        "features":["streaming","structured_output","system_role"],
        "max_concurrent":1,"local_development":true,"allowed_hosts":["127.0.0.1"],
        "trust_zones":["external-evidence-fixture"],"provider_options":{}
    });
    let path = session_root.join("model-profile.fixture.json");
    fs::write(
        &path,
        serde_json::to_vec(&value).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    // Product parser validation happens before daemon startup.
    milkdrift_model_provider::EndpointProfile::from_json(
        &fs::read(&path).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    Ok(path)
}

fn validate_output_path(path: &Path) -> HarnessResult<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()
            .map_err(|error| error.to_string())?
            .join(path)
    };
    if absolute
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err("output path must not contain parent traversal".to_owned());
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let mut ancestor = absolute.as_path();
    let mut suffix = Vec::new();
    while !ancestor.exists() {
        suffix.push(
            ancestor
                .file_name()
                .ok_or_else(|| "output path has no existing ancestor".to_owned())?
                .to_owned(),
        );
        ancestor = ancestor
            .parent()
            .ok_or_else(|| "output path has no existing ancestor".to_owned())?;
    }
    let mut resolved = ancestor.canonicalize().map_err(|error| error.to_string())?;
    for component in suffix.into_iter().rev() {
        resolved.push(component);
    }
    let target = root
        .join("target")
        .canonicalize()
        .map_err(|error| format!("repository target directory is unavailable: {error}"))?;
    if resolved.starts_with(&root) && !resolved.starts_with(target) {
        return Err("evidence output inside tracked source paths is forbidden; use target/ or an external directory".to_owned());
    }
    Ok(resolved)
}

fn milkdrift_git_facts() -> HarnessResult<(String, String, bool)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    Ok((
        workflows::git(&root, &["rev-parse", "HEAD"])?,
        workflows::git(&root, &["rev-parse", "HEAD^{tree}"])?,
        !workflows::git(&root, &["status", "--porcelain"])?.is_empty(),
    ))
}

fn rust_build_target() -> HarnessResult<String> {
    let output = milkdrift_evidence::application::run_command(
        std::process::Command::new("rustc").arg("-vV"),
        None,
        Duration::from_secs(10),
    )
    .map_err(|error| format!("rustc build-target query failed: {error}"))?;
    if !output.status.success() {
        return Err("rustc build-target query failed".to_owned());
    }
    output
        .stdout
        .lines()
        .find_map(|line| line.strip_prefix("host: ").map(str::to_owned))
        .filter(|target| !target.is_empty())
        .ok_or_else(|| "rustc build-target query omitted the host triple".to_owned())
}

fn redaction_check(path: &Path, forbidden: &[Vec<u8>]) -> HarnessResult {
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    if forbidden
        .iter()
        .any(|value| !value.is_empty() && bytes.windows(value.len()).any(|window| window == value))
    {
        return Err("redaction validation found a secret value in the report".to_owned());
    }
    let text = String::from_utf8(bytes).map_err(|error| error.to_string())?;
    for forbidden_key in ["authorization", "full_prompt", "full_output", "environment"] {
        if text
            .to_ascii_lowercase()
            .contains(&format!("\"{forbidden_key}\""))
        {
            return Err(format!(
                "redaction validation found forbidden key {forbidden_key}"
            ));
        }
    }
    serde_json::from_str::<Value>(&text).map_err(|error| error.to_string())?;
    Ok(())
}

fn forbidden_secret_values(mappings: &[String]) -> HarnessResult<Vec<Vec<u8>>> {
    let mut values = Vec::new();
    for mapping in mappings {
        let (_reference, source) = mapping
            .split_once('=')
            .ok_or_else(|| "secret source mapping requires reference=source".to_owned())?;
        let value = if let Some(variable) = source.strip_prefix("env:") {
            std::env::var_os(variable)
                .ok_or_else(|| {
                    format!("required secret environment variable {variable} is not set")
                })?
                .as_encoded_bytes()
                .to_vec()
        } else if let Some(path) = source.strip_prefix("file:") {
            fs::read(path).map_err(|error| format!("secret source file read: {error}"))?
        } else {
            return Err("secret source must use env: or file:".to_owned());
        };
        if !value.is_empty() {
            values.push(value);
        }
    }
    Ok(values)
}

fn unix_millis() -> HarnessResult<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system clock precedes the Unix epoch".to_owned())
        .and_then(|duration| {
            u64::try_from(duration.as_millis())
                .map_err(|_| "system clock exceeds the timestamp representation".to_owned())
        })
}

mod scenarios;

use scenarios::{run_model_scenario, run_process_scenario};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grants_cover_separate_helper_executables_without_extra_access() -> HarnessResult {
        let root = tempfile::tempdir().map_err(|error| error.to_string())?;
        for directory in ["agent", "python", "session", "repository"] {
            fs::create_dir(root.path().join(directory)).map_err(|error| error.to_string())?;
        }
        let root = root
            .path()
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let agent = AgentProfile {
            path: root.join("agent-profile.json"),
            capability: "test-agent".to_owned(),
            canonical_executable: root.join("agent/agent.exe"),
            content_digest: String::new(),
            size_bytes: 1,
            version_output: String::new(),
            output_names: vec!["diff".to_owned()],
            secret_refs: BTreeSet::new(),
        };
        let mut helpers = GeneratedProfiles {
            canonical_executable: root.join("python/python.exe"),
            weak_verifier: root.join("weak.json"),
            good_verifier: root.join("good.json"),
            reviewer: root.join("reviewer.json"),
            evidence_source: root.join("source.json"),
        };
        let model = workflows::ModelProfileFacts {
            profile_id: "test-model".to_owned(),
            revision: 1,
            protocol: "open_ai_compatible".to_owned(),
            model_alias: "test-model".to_owned(),
            endpoint_origin: "http://127.0.0.1:8080".to_owned(),
            streaming: true,
            structured_output: false,
            secret_refs: BTreeSet::new(),
        };
        let execute = BTreeSet::from([AccessMode::Execute]);
        let helper_scope = FilesystemScope::from_canonical_host_path(&root.join("python"), execute)
            .map_err(|error| error.to_string())?;
        for same_executable in [false, true] {
            if same_executable {
                helpers
                    .canonical_executable
                    .clone_from(&agent.canonical_executable);
            }
            for model_profile in [None, Some(model.profile_id.as_str())] {
                let grant = explicit_grant(
                    PROCESS_WORKFLOW,
                    &BTreeSet::from([agent.capability.clone()]),
                    model_profile,
                    &root.join("session"),
                    &root.join("repository"),
                    &agent,
                    &helpers,
                    &model,
                    &BTreeMap::new(),
                )?;
                assert_eq!(
                    grant.resources.filesystem.len(),
                    if same_executable { 3 } else { 4 }
                );
                assert_eq!(
                    grant.resources.filesystem.contains(&helper_scope),
                    !same_executable
                );
            }
        }
        Ok(())
    }
}
