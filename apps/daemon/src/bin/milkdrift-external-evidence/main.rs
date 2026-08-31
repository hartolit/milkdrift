//! Operator-driven real process and model interoperability evidence harness.

mod profiles;
mod report;
mod workflows;

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read as _, Write as _},
    net::{IpAddr, Ipv4Addr, SocketAddr},
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
    NetworkProfileRef, NetworkScope, PeerAuthorityScope, ResourceScope, SecretRef,
    WorkflowRunScope, WorkspaceAuthorityScope,
};
use milkdrift_blueprint::{BlueprintRevisionDocument, WorkflowId};
use milkdrift_capability::{CapabilityId, OperationId, SideEffectClass};
use milkdrift_control_client::{BearerCredential, ClientConfig, ClientError, ControlClient};
use milkdrift_control_protocol::{Command, CommandRequest, ProtocolVersion, decode_json};
use milkdrift_daemon::{
    ActorBindingConfig, ActorGrantConfig, AdapterConfig, ApplicationReceiptConfig,
    AuthorityPresetConfig, DaemonConfig, DaemonHost, ModelProfileConfig, PeerHostConfig,
    RuntimeHostConfig, SecretSourceConfig, ShutdownConfig, ShutdownEffectPolicy,
    ValidatedDaemonConfig, serve,
};
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
use tokio::{sync::oneshot, task::JoinHandle};
use url::Url;
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

struct RunningDaemon {
    endpoint: Url,
    stop: oneshot::Sender<()>,
    task: JoinHandle<Result<(), milkdrift_daemon::HostError>>,
}

impl RunningDaemon {
    fn client(&self, token: &str) -> HarnessResult<ControlClient> {
        let mut config = ClientConfig::new(self.endpoint.clone());
        config.safe_query_retries = 0;
        config.retry_delay = Duration::from_millis(20);
        ControlClient::new(
            config,
            BearerCredential::new(token).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())
    }

    async fn stop(self) -> HarnessResult {
        let _ = self.stop.send(());
        tokio::time::timeout(Duration::from_secs(30), self.task)
            .await
            .map_err(|_| "daemon graceful shutdown timed out".to_owned())?
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())
    }
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
    let now = unix_millis();
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
    let result = run_scenarios(&arguments, &output, &session_root, &repository, &mut report).await;
    if let Err(error) = &result {
        report.failure_reason = Some(error.clone());
    }
    report.qualifying = report.process.qualifying && report.model.qualifying;
    write_report(&report_path, &report)?;
    redaction_check(&report_path, &[])?;
    result
}

async fn run_scenarios(
    arguments: &Arguments,
    output: &Path,
    session_root: &Path,
    repository: &Path,
    report: &mut EvidenceReport,
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
    let forbidden = [process_token.as_str(), model_token.as_str()];
    let temporary = output.join("report.preflight.json");
    write_report(&temporary, report)?;
    redaction_check(&temporary, &forbidden)?;
    fs::remove_file(temporary).map_err(|error| error.to_string())?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn configuration(
    session_root: &Path,
    repository: &Path,
    agent: &AgentProfile,
    helpers: &GeneratedProfiles,
    model_profile_path: &Path,
    model_capability: &str,
    model: &workflows::ModelProfileFacts,
    mut secret_sources: BTreeMap<String, SecretSourceConfig>,
) -> HarnessResult<(ValidatedDaemonConfig, String, String, String)> {
    let process_token = format!("process-{}-{}", std::process::id(), unix_millis());
    let model_token = format!("model-{}-{}", std::process::id(), unix_millis());
    let process_token_path = session_root.join("process.token");
    let model_token_path = session_root.join("model.token");
    profiles::secure_file(&process_token_path, process_token.as_bytes())?;
    profiles::secure_file(&model_token_path, model_token.as_bytes())?;
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
        schema_version: 7,
        data_root: session_root.join("data"),
        bind: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
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
    }
    .validate(session_root)
    .map_err(|error| error.to_string())?;
    let configuration_bytes =
        serde_json::to_vec(&config.document).map_err(|error| error.to_string())?;
    let configuration_digest = format!("b3_{}", blake3::hash(&configuration_bytes).to_hex());
    Ok((config, configuration_digest, process_token, model_token))
}

#[allow(clippy::too_many_arguments)]
fn explicit_grant(
    workflow: &str,
    capabilities: &BTreeSet<String>,
    model_profile: Option<&str>,
    session_root: &Path,
    repository: &Path,
    agent: &AgentProfile,
    model: &workflows::ModelProfileFacts,
    configured_secrets: &BTreeMap<String, SecretSourceConfig>,
) -> HarnessResult<ActorGrantConfig> {
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
    let mut filesystem = vec![
        FilesystemScope::new(
            session_root
                .to_str()
                .ok_or_else(|| "session root is not UTF-8".to_owned())?,
            BTreeSet::from([AccessMode::Read, AccessMode::Write, AccessMode::Execute]),
        )
        .map_err(|error| error.to_string())?,
        FilesystemScope::new(
            repository
                .to_str()
                .ok_or_else(|| "repository root is not UTF-8".to_owned())?,
            BTreeSet::from([AccessMode::Read, AccessMode::Write, AccessMode::Execute]),
        )
        .map_err(|error| error.to_string())?,
        FilesystemScope::new("/usr/bin", BTreeSet::from([AccessMode::Execute]))
            .map_err(|error| error.to_string())?,
    ];
    if model_profile.is_none() {
        filesystem.push(
            FilesystemScope::new(
                agent
                    .canonical_executable
                    .parent()
                    .and_then(Path::to_str)
                    .ok_or_else(|| "agent executable parent is not UTF-8".to_owned())?,
                BTreeSet::from([AccessMode::Execute]),
            )
            .map_err(|error| error.to_string())?,
        );
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
        BTreeSet::new(),
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
        valid_from: BoundaryTimeMillis::new(unix_millis().saturating_sub(60_000)),
        valid_until: BoundaryTimeMillis::new(unix_millis().saturating_add(86_400_000)),
        // Dynamic artifact/workspace identities require explicit run-wide acknowledgement;
        // workflow, capability, operation, filesystem, network, secret, time, and budgets remain exact.
        dangerous_allow_broad_authority: true,
    })
}

async fn start(config: ValidatedDaemonConfig) -> HarnessResult<RunningDaemon> {
    let host = DaemonHost::start(config).map_err(|error| error.to_string())?;
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|error| error.to_string())?;
    let address = listener.local_addr().map_err(|error| error.to_string())?;
    let endpoint = Url::parse(&format!("http://{address}/")).map_err(|error| error.to_string())?;
    let (stop, stopped) = oneshot::channel();
    let task = tokio::spawn(serve(listener, host, async move {
        let _ = stopped.await;
    }));
    Ok(RunningDaemon {
        endpoint,
        stop,
        task,
    })
}

async fn run_process_scenario(
    config: &ValidatedDaemonConfig,
    token: &str,
    agent: &AgentProfile,
    initial_commit: &str,
    initial_tree: &str,
    repository: &Path,
    fixture: bool,
) -> HarnessResult<ScenarioEvidence> {
    let sequence = workflows::process_sequence(&agent.capability, &agent.output_names)?;
    let sequence_value = serde_json::to_value(&sequence).map_err(|error| error.to_string())?;
    let daemon = start(config.clone()).await?;
    let client = daemon.client(token)?;
    client.readiness().await.map_err(client_error)?;
    let imported = client
        .submit(&request(
            "external-process-import",
            None,
            Command::ImportPromptSequence {
                document: sequence_value,
            },
        ))
        .await
        .map_err(client_error)?;
    let revision = json_string(&imported.value, "revision_id")?;
    client
        .submit(&request(
            "external-process-start",
            None,
            Command::StartRun {
                run_id: PROCESS_RUN.to_owned(),
                workflow_id: PROCESS_WORKFLOW.to_owned(),
                revision_id: revision.clone(),
            },
        ))
        .await
        .map_err(client_error)?;
    let before_restart = wait_for_run(&client, PROCESS_RUN, |state| {
        [
            "stage-repair-coding",
            "stage-repair-verification",
            "stage-repair-review",
            "stage-repair-approval",
        ]
        .iter()
        .all(|expected| {
            state.nodes.iter().any(|node| {
                node.node_id == *expected
                    && (*expected == "stage-repair-approval" || node.latest_attempt_id.is_some())
            })
        })
    })
    .await?;
    let original_agent_attempt = attempt_for_node(&before_restart, "stage-repair-coding")?;
    let weak_attempt = attempt_for_node(&before_restart, "stage-repair-verification")?;
    let reviewer_attempt = attempt_for_node(&before_restart, "stage-repair-review")?;
    daemon.stop().await?;

    let daemon = start(config.clone()).await?;
    let client = daemon.client(token)?;
    let recovered = client
        .run(PROCESS_RUN)
        .await
        .map_err(|error| error.to_string())?;
    if recovered.sequence != before_restart.sequence
        || !recovered
            .nodes
            .iter()
            .any(|node| node.node_id == "stage-repair-approval")
    {
        return Err("process run did not recover the durable review boundary".to_owned());
    }
    client
        .submit(&request(
            "external-process-pause",
            Some(recovered.sequence),
            Command::PauseRun {
                run_id: PROCESS_RUN.to_owned(),
            },
        ))
        .await
        .map_err(client_error)?;
    let paused = client
        .run(PROCESS_RUN)
        .await
        .map_err(|error| error.to_string())?;
    let revision_read = client
        .revision(&revision)
        .await
        .map_err(|error| error.to_string())?;
    let base_value = revision_read
        .document
        .as_ref()
        .ok_or_else(|| "process base revision document is absent".to_owned())?;
    let (_, base) = BlueprintRevisionDocument::from_json(
        &serde_json::to_vec(base_value).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let mut good_verification = sequence.sequence().stages[0].verification.clone();
    good_verification.profile.capability =
        CapabilityId::new("evidence-verifier-good").map_err(|error| error.to_string())?;
    let proposal = build_remediation_proposal(
        &sequence,
        &base,
        RemediationProposalSpec {
            run: milkdrift_workspace::RunId::new(PROCESS_RUN).map_err(|error| error.to_string())?,
            observed_sequence: milkdrift_persistence::RunSequence::new(paused.sequence),
            proposal: milkdrift_control::ProposalId::new("proposal-external-remediation-1")
                .map_err(|error| error.to_string())?,
            proposer: milkdrift_authority::ActorRef::new("human:external-process")
                .map_err(|error| error.to_string())?,
            stage_id: "repair".to_owned(),
            generation: 1,
            prompt: PromptSource::InlineMarkdown {
                content: "In a fresh process, inspect the current accepted repository state, make only any remaining bounded repair needed for the unittest, run it, and do not commit.\n"
                    .to_owned(),
            },
            verification_override: Some(good_verification),
        },
    )
    .map_err(|error| error.to_string())?;
    let proposal_digest = proposal.proposal().digest().as_str().to_owned();
    let proposal_value = decode_json(
        &proposal
            .to_canonical_json()
            .map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let mut submit = request(
        "external-process-proposal",
        Some(paused.sequence),
        Command::SubmitProposal {
            document: proposal_value,
        },
    );
    submit.expected_revision = Some(revision.clone());
    let submitted = client
        .submit(&submit)
        .await
        .map_err(|error| error.to_string())?;
    let proposed_revision = json_string(&submitted.value, "proposed_revision")?;
    let decision_boundary = client
        .run(PROCESS_RUN)
        .await
        .map_err(|error| error.to_string())?
        .sequence;
    let mut approve = request(
        "external-process-approve",
        Some(decision_boundary),
        Command::DecideProposal {
            run_id: PROCESS_RUN.to_owned(),
            proposal_id: "proposal-external-remediation-1".to_owned(),
            proposal_digest: proposal_digest.clone(),
            proposed_revision: proposed_revision.clone(),
            decision_id: "decision-external-remediation-1".to_owned(),
            decision: milkdrift_control_protocol::ProposalDecision::Approve,
        },
    );
    approve.expected_revision = Some(proposed_revision.clone());
    client
        .submit(&approve)
        .await
        .map_err(|error| error.to_string())?;
    let apply_boundary = client
        .run(PROCESS_RUN)
        .await
        .map_err(|error| error.to_string())?
        .sequence;
    let mut apply = request(
        "external-process-apply",
        Some(apply_boundary),
        Command::ApplyProposal {
            run_id: PROCESS_RUN.to_owned(),
            proposal_id: "proposal-external-remediation-1".to_owned(),
            proposal_digest,
            proposed_revision: proposed_revision.clone(),
        },
    );
    apply.expected_revision = Some(proposed_revision.clone());
    client
        .submit(&apply)
        .await
        .map_err(|error| error.to_string())?;
    let signal_boundary = client
        .run(PROCESS_RUN)
        .await
        .map_err(|error| error.to_string())?
        .sequence;
    client
        .submit(&request(
            "external-process-signal",
            Some(signal_boundary),
            Command::SignalRun {
                run_id: PROCESS_RUN.to_owned(),
                signal_id: "signal-external-remediation-1".to_owned(),
                signal_type: "sequence.approved".to_owned(),
                correlation: None,
                broadcast: false,
                payload: json!({"proposal":"proposal-external-remediation-1","revision":proposed_revision}),
            },
        ))
        .await
        .map_err(client_error)?;
    let resume_boundary = client
        .run(PROCESS_RUN)
        .await
        .map_err(|error| error.to_string())?
        .sequence;
    client
        .submit(&request(
            "external-process-resume",
            Some(resume_boundary),
            Command::ResumeRun {
                run_id: PROCESS_RUN.to_owned(),
            },
        ))
        .await
        .map_err(|error| error.to_string())?;
    let completed = wait_for_run(&client, PROCESS_RUN, |state| {
        state.terminal.as_deref() == Some("succeeded")
    })
    .await?;
    for node in &completed.nodes {
        if (node.node_id.contains("coding")
            || node.node_id.contains("verification")
            || node.node_id.contains("review"))
            && node.attempt_count != 1
        {
            return Err(format!(
                "process node {} executed more than once",
                node.node_id
            ));
        }
    }
    let remediation_attempt = attempt_for_node(&completed, "stage-repair-remediation-1-coding")?;
    let good_attempt = attempt_for_node(&completed, "stage-repair-remediation-1-verification")?;
    let agent_read = client
        .attempt(PROCESS_RUN, &original_agent_attempt)
        .await
        .map_err(|error| error.to_string())?;
    let agent_provenance = agent_read
        .capability_provenance
        .as_ref()
        .ok_or_else(|| "agent attempt omitted capability provenance".to_owned())?;
    if agent_provenance.implementation_content_digest.as_deref()
        != Some(agent.content_digest.as_str())
        || agent_provenance.implementation_size_bytes != Some(agent.size_bytes)
        || agent_provenance.process_profile_digest.is_none()
        || agent_provenance.execution_policy_digest.is_none()
        || agent_provenance.configured_path_digest.is_none()
        || agent_provenance.canonical_path_digest.is_none()
    {
        return Err("agent attempt omitted exact executable/profile provenance".to_owned());
    }
    let good_read = client
        .attempt(PROCESS_RUN, &good_attempt)
        .await
        .map_err(|error| error.to_string())?;
    let weak_read = client
        .attempt(PROCESS_RUN, &weak_attempt)
        .await
        .map_err(client_error)?;
    let reviewer_read = client
        .attempt(PROCESS_RUN, &reviewer_attempt)
        .await
        .map_err(client_error)?;
    let remediation_read = client
        .attempt(PROCESS_RUN, &remediation_attempt)
        .await
        .map_err(client_error)?;
    let required_artifacts = [
        "verification_result",
        "verification_logs",
        "verification_pass",
    ];
    if required_artifacts
        .iter()
        .any(|name| !good_read.outputs.iter().any(|output| output.name == *name))
    {
        return Err("good verification omitted exact diff/test/log/pass artifacts".to_owned());
    }
    let process_reads = [
        ("initial_coding", &agent_read),
        ("controlled_verification", &weak_read),
        ("independent_review", &reviewer_read),
        ("remediation_coding", &remediation_read),
        ("final_verification", &good_read),
    ];
    let mut invocation_ids = BTreeSet::new();
    let mut invocations = Vec::new();
    let mut artifacts = Vec::new();
    for (role, read) in process_reads {
        let invocation = read
            .invocation_id
            .as_ref()
            .ok_or_else(|| format!("{role} attempt omitted its invocation identity"))?;
        if !invocation_ids.insert(invocation.clone()) {
            return Err("process attempts reused an external invocation identity".to_owned());
        }
        let provenance = read
            .capability_provenance
            .as_ref()
            .ok_or_else(|| format!("{role} attempt omitted capability provenance"))?;
        invocations.push(json!({
            "role":role,
            "attempt_id":read.attempt_id,
            "invocation_id":invocation,
            "capability_id":read.capability_id,
            "descriptor_revision":read.descriptor_revision,
            "snapshot_digest":provenance.snapshot_digest,
            "process_profile_digest":provenance.process_profile_digest,
            "context_manifest_digest":read.context.as_ref().map(|context| context.digest.clone()),
            "terminal":read.terminal,
            "uncertain":read.uncertain,
        }));
        artifacts.extend(read.outputs.iter().map(|output| ArtifactEvidence {
            artifact_id: output.artifact.artifact_id.clone(),
            digest: output.artifact.digest.clone(),
            size: output.artifact.size,
            content_type: output.artifact.content_type.clone(),
            role: format!("{role}:{}", output.name),
        }));
    }
    let final_commit = workflows::git(repository, &["rev-parse", "HEAD"])?;
    let final_tree = workflows::git(repository, &["rev-parse", "HEAD^{tree}"])?;
    let dirty_diff = workflows::git(repository, &["diff", "--binary", "HEAD"])?;
    if final_commit == initial_commit && dirty_diff.is_empty() {
        return Err("real coding agent produced neither a commit nor a dirty diff".to_owned());
    }
    daemon.stop().await?;
    Ok(ScenarioEvidence {
        qualifying: !fixture,
        outcome: "succeeded".to_owned(),
        profile: json!({
            "capability":agent.capability,
            "executable_configured_path_digest":agent_provenance.configured_path_digest,
            "executable_canonical_path_digest":agent_provenance.canonical_path_digest,
            "executable_content_digest":agent.content_digest,
            "executable_size_bytes":agent.size_bytes,
            "version_output":agent.version_output,
            "profile_digest":agent_provenance.process_profile_digest,
            "policy_digest":agent_provenance.execution_policy_digest,
            "fixture_rejected_for_qualification":fixture,
        }),
        commands: vec![
            "external-process-import".to_owned(),
            "external-process-start".to_owned(),
            "external-process-pause".to_owned(),
            "external-process-proposal".to_owned(),
            "external-process-approve".to_owned(),
            "external-process-apply".to_owned(),
            "external-process-signal".to_owned(),
            "external-process-resume".to_owned(),
        ],
        runs: vec![PROCESS_RUN.to_owned()],
        revisions: vec![revision, proposed_revision],
        attempts: vec![
            original_agent_attempt,
            weak_attempt,
            reviewer_attempt,
            remediation_attempt,
            good_attempt,
        ],
        proposals: vec!["proposal-external-remediation-1".to_owned()],
        artifacts,
        restart_boundaries: vec![RestartEvidence {
            boundary: "after controlled verification failure and independent review".to_owned(),
            sequence_before: before_restart.sequence,
            sequence_after: recovered.sequence,
            recovered_state: "awaiting remediation approval".to_owned(),
            duplicate_attempts: false,
        }],
        facts: json!({
            "repository_initial_commit":initial_commit,
            "repository_initial_tree":initial_tree,
            "repository_final_commit":final_commit,
            "repository_final_tree":final_tree,
            "dirty_diff_digest":format!("b3_{}",blake3::hash(dirty_diff.as_bytes())),
            "dirty_diff_bytes":dirty_diff.len(),
            "controlled_failure":"orchestration verifier gate intentionally omitted verification_pass",
            "fresh_agent_attempts":2,
            "distinct_process_invocations":invocation_ids.len(),
            "process_invocations":invocations,
            "terminal_sequence":completed.sequence,
        }),
        failure_reason: None,
    })
}

async fn run_model_scenario(
    config: &ValidatedDaemonConfig,
    token: &str,
    model_capability: &str,
    profile: &workflows::ModelProfileFacts,
    fixture_requests: Option<Arc<AtomicUsize>>,
    fixture_request_lines: Option<Arc<Mutex<Vec<String>>>>,
    fixture: bool,
) -> HarnessResult<ScenarioEvidence> {
    let daemon = start(config.clone()).await?;
    let client = daemon.client(token)?;
    let blueprint = workflows::model_revision(model_capability, profile)?;
    let imported = client
        .submit(&request(
            "external-model-import",
            None,
            Command::ImportBlueprint {
                document: blueprint,
            },
        ))
        .await
        .map_err(client_error)?;
    let revision = json_string(&imported.value, "revision_id")?;
    client
        .submit(&request(
            "external-model-start",
            None,
            Command::StartRun {
                run_id: MODEL_RUN.to_owned(),
                workflow_id: MODEL_WORKFLOW.to_owned(),
                revision_id: revision.clone(),
            },
        ))
        .await
        .map_err(client_error)?;
    let before_restart = wait_for_run(&client, MODEL_RUN, |state| {
        state
            .nodes
            .iter()
            .any(|node| node.node_id == "model-release")
    })
    .await?;
    if fixture_requests
        .as_ref()
        .is_some_and(|count| count.load(Ordering::SeqCst) != 0)
    {
        return Err("fixture model endpoint was entered before release boundary".to_owned());
    }
    daemon.stop().await?;

    let daemon = start(config.clone()).await?;
    let client = daemon.client(token)?;
    let recovered = client
        .run(MODEL_RUN)
        .await
        .map_err(|error| error.to_string())?;
    if recovered.sequence != before_restart.sequence
        || !recovered
            .nodes
            .iter()
            .any(|node| node.node_id == "model-release")
    {
        return Err("model workflow did not recover the pre-entry release boundary".to_owned());
    }
    client
        .submit(&request(
            "external-model-release",
            Some(recovered.sequence),
            Command::SignalRun {
                run_id: MODEL_RUN.to_owned(),
                signal_id: "signal-external-model-release".to_owned(),
                signal_type: "evidence.model.release".to_owned(),
                correlation: None,
                broadcast: false,
                payload: json!({"approved":true}),
            },
        ))
        .await
        .map_err(|error| error.to_string())?;
    let completed = wait_for_run(&client, MODEL_RUN, |state| {
        state.terminal.as_deref() == Some("succeeded")
            || state.nodes.iter().any(|node| {
                node.node_id == "model"
                    && (node.state.contains("uncertain")
                        || node.state.contains("rejected")
                        || node.state.contains("failed"))
            })
    })
    .await?;
    if completed.terminal.as_deref() != Some("succeeded") {
        let health = client.health().await.map_err(client_error)?;
        let failed_attempt = attempt_for_node(&completed, "model")?;
        let failed_read = client
            .attempt(MODEL_RUN, &failed_attempt)
            .await
            .map_err(client_error)?;
        return Err(format!(
            "model attempt did not complete; fixture_requests={}; fixture_request_lines={:?}; worker_failure={:?}; attempt_state={}; progress={}; terminal={:?}; outputs={:?}",
            fixture_requests
                .as_ref()
                .map_or(0, |count| count.load(Ordering::SeqCst)),
            fixture_request_lines
                .as_ref()
                .and_then(|lines| lines.lock().ok().map(|lines| lines.clone()))
                .unwrap_or_default(),
            health.last_failure,
            failed_read.state,
            failed_read.progress_observations,
            failed_read.terminal,
            failed_read
                .outputs
                .iter()
                .map(|output| output.name.clone())
                .collect::<Vec<_>>()
        ));
    }
    let model_attempt = attempt_for_node(&completed, "model")?;
    let attempt = client
        .attempt(MODEL_RUN, &model_attempt)
        .await
        .map_err(|error| error.to_string())?;
    if completed
        .nodes
        .iter()
        .find(|node| node.node_id == "model")
        .is_none_or(|node| node.attempt_count != 1)
    {
        return Err("model external entry was duplicated".to_owned());
    }
    let context = attempt
        .context
        .as_ref()
        .ok_or_else(|| "model attempt omitted authorized context manifest".to_owned())?;
    let denied_attempt = attempt_for_node(&completed, "evidence-denied")?;
    let denied = client
        .attempt(MODEL_RUN, &denied_attempt)
        .await
        .map_err(client_error)?;
    let denied_artifact = denied
        .outputs
        .iter()
        .find(|output| output.name == "evidence")
        .ok_or_else(|| "denied evidence task omitted its artifact".to_owned())?;
    let entries = serde_json::to_string(&context.entries).map_err(|error| error.to_string())?;
    let omissions = serde_json::to_string(&context.omissions).map_err(|error| error.to_string())?;
    if !entries.contains("evidence-a")
        || !entries.contains("evidence-b")
        || entries.contains(&denied_artifact.artifact.artifact_id)
        || !omissions.contains(&denied_artifact.artifact.artifact_id)
    {
        return Err("frozen context manifest selected/omitted the wrong evidence".to_owned());
    }
    if profile.streaming && attempt.progress_observations == 0 {
        return Err("streaming profile produced no durable fragment observation".to_owned());
    }
    let provenance = attempt
        .capability_provenance
        .as_ref()
        .ok_or_else(|| "model attempt omitted capability provenance".to_owned())?;
    if provenance.model_profile_digest.is_none()
        || provenance.model_profile_revision != Some(profile.revision)
        || provenance.provider_protocol.as_deref() != Some(profile.protocol.as_str())
        || provenance.model_alias.as_deref() != Some(profile.model_alias.as_str())
        || provenance.endpoint_origin.as_deref() != Some(profile.endpoint_origin.as_str())
    {
        return Err(
            "model attempt omitted exact profile/protocol/model/origin provenance".to_owned(),
        );
    }
    let response_output = attempt
        .outputs
        .iter()
        .find(|output| output.name == "model_response")
        .ok_or_else(|| "model response artifact is absent".to_owned())?;
    let provider_output = attempt
        .outputs
        .iter()
        .find(|output| output.name == "provider_metadata")
        .ok_or_else(|| "provider metadata artifact is absent".to_owned())?;
    let response_bytes = client
        .artifact_range(
            &response_output.artifact.artifact_id,
            0,
            response_output.artifact.size.saturating_sub(1),
        )
        .await
        .map_err(|error| error.to_string())?
        .bytes;
    let response = ModelResponseDocument::from_json(&response_bytes)
        .map_err(|error| error.to_string())?
        .body()
        .clone();
    let provider_bytes = client
        .artifact_range(
            &provider_output.artifact.artifact_id,
            0,
            provider_output.artifact.size.saturating_sub(1),
        )
        .await
        .map_err(client_error)?
        .bytes;
    let provider_metadata: Value =
        serde_json::from_slice(&provider_bytes).map_err(|error| error.to_string())?;
    if provider_metadata
        .as_object()
        .is_none_or(serde_json::Map::is_empty)
    {
        return Err("provider metadata artifact is empty or malformed".to_owned());
    }
    let provider_identity = provider_metadata.as_object().is_some_and(|metadata| {
        metadata.values().any(|value| {
            value
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|identity| !identity.is_empty())
                && value
                    .get("model")
                    .and_then(Value::as_str)
                    .is_some_and(|model| !model.is_empty())
        })
    });
    if !provider_identity {
        return Err("provider metadata omitted response identity or model".to_owned());
    }
    let structured_ok = response
        .structured()
        .and_then(|value| value.value().get("ok"))
        .and_then(Value::as_bool)
        == Some(true);
    let parseable_ok = response.text().trim() == "MILKDRIFT_EVIDENCE_OK";
    if profile.structured_output && !structured_ok || !profile.structured_output && !parseable_ok {
        return Err("model response did not satisfy the bounded parseable contract".to_owned());
    }
    if fixture_requests
        .as_ref()
        .is_some_and(|count| count.load(Ordering::SeqCst) != 1)
    {
        return Err("fixture endpoint observed a duplicate or missing remote request".to_owned());
    }
    if fixture_request_lines.as_ref().is_some_and(|lines| {
        lines.lock().map_or(true, |lines| {
            lines.as_slice() != ["POST /v1/chat/completions HTTP/1.1"]
        })
    }) {
        return Err("fixture endpoint observed the wrong request target".to_owned());
    }
    let artifacts = attempt
        .outputs
        .iter()
        .map(|output| ArtifactEvidence {
            artifact_id: output.artifact.artifact_id.clone(),
            digest: output.artifact.digest.clone(),
            size: output.artifact.size,
            content_type: output.artifact.content_type.clone(),
            role: output.name.clone(),
        })
        .collect::<Vec<_>>();
    let finish_reason = format!("{:?}", response.finish_reason()).to_lowercase();
    if finish_reason == "unknown" {
        return Err("provider response omitted a recognized finish reason".to_owned());
    }
    let usage = response.usage();
    if usage.input_units.is_none()
        && usage.output_units.is_none()
        && usage.cached_input_units.is_none()
        && usage.cost_micros.is_none()
    {
        return Err("provider response omitted all usage facts".to_owned());
    }
    let durable_usage = attempt
        .usage
        .as_ref()
        .ok_or_else(|| "model terminal evidence omitted durable usage facts".to_owned())?;
    if durable_usage.input_units != usage.input_units
        || durable_usage.output_units != usage.output_units
    {
        return Err("durable attempt usage differs from the committed response".to_owned());
    }
    let context_digest = context.digest.clone();
    let selected_count = context.entries.len();
    let omitted_count = context.omissions.len();
    daemon.stop().await?;
    Ok(ScenarioEvidence {
        qualifying: !fixture,
        outcome: "succeeded".to_owned(),
        profile: json!({
            "profile_digest":provenance.model_profile_digest,
            "profile_revision":profile.revision,
            "provider_protocol":profile.protocol,
            "model_alias":profile.model_alias,
            "endpoint_origin":profile.endpoint_origin,
            "fixture_rejected_for_qualification":fixture,
        }),
        commands: vec![
            "external-model-import".to_owned(),
            "external-model-start".to_owned(),
            "external-model-release".to_owned(),
        ],
        runs: vec![MODEL_RUN.to_owned()],
        revisions: vec![revision],
        attempts: vec![model_attempt],
        proposals: Vec::new(),
        artifacts,
        restart_boundaries: vec![RestartEvidence {
            boundary: "durable signal wait before model adapter entry".to_owned(),
            sequence_before: before_restart.sequence,
            sequence_after: recovered.sequence,
            recovered_state: "model request unreleased; no external entry".to_owned(),
            duplicate_attempts: false,
        }],
        facts: json!({
            "context_manifest_digest":context_digest,
            "selected_count":selected_count,
            "omitted_count":omitted_count,
            "denied_context_artifact_digest":denied_artifact.artifact.digest,
            "streaming_observations":attempt.progress_observations,
            "streaming_bytes":attempt.progress_bytes,
            "finish_reason":finish_reason,
            "usage":{"input_units":usage.input_units,"output_units":usage.output_units,"cached_input_units":usage.cached_input_units,"duration_ms":durable_usage.duration_ms,"cost_micros":usage.cost_micros,"currency":usage.currency},
            "provider_metadata_artifact_digest":provider_output.artifact.digest,
            "terminal":"succeeded",
            "uncertain":attempt.uncertain,
            "capability_generation":attempt.descriptor_revision,
            "invocation_id":attempt.invocation_id,
        }),
        failure_reason: None,
    })
}

async fn wait_for_run(
    client: &ControlClient,
    run: &str,
    predicate: impl Fn(&milkdrift_control_protocol::RunRead) -> bool,
) -> HarnessResult<milkdrift_control_protocol::RunRead> {
    let mut last = None;
    for _ in 0..1_200 {
        let state = client.run(run).await.map_err(|error| error.to_string())?;
        if predicate(&state) {
            return Ok(state);
        }
        if state.lifecycle == "terminal" {
            return Err(format!(
                "run {run} reached unexpected terminal state: {}",
                serde_json::to_string(&state).map_err(|error| error.to_string())?
            ));
        }
        last = Some(state);
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    Err(format!(
        "run {run} did not reach the expected bounded state; last={}",
        serde_json::to_string(&last).map_err(|error| error.to_string())?
    ))
}

fn attempt_for_node(
    run: &milkdrift_control_protocol::RunRead,
    node: &str,
) -> HarnessResult<String> {
    run.nodes
        .iter()
        .find(|value| value.node_id == node)
        .and_then(|value| value.latest_attempt_id.clone())
        .ok_or_else(|| format!("node {node} has no attempt"))
}

fn request(command_id: &str, sequence: Option<u64>, command: Command) -> CommandRequest {
    CommandRequest {
        protocol: ProtocolVersion::CURRENT,
        command_id: command_id.to_owned(),
        expected_sequence: sequence,
        expected_revision: None,
        reason: "external interoperability evidence harness".to_owned(),
        evidence: Vec::new(),
        command,
    }
}

fn json_string(value: &Value, name: &str) -> HarnessResult<String> {
    value
        .get(name)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("command response omitted {name}"))
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
        profiles::secure_file(&sentinel, b"unused")?;
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
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
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
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 8_192];
    let header_end = loop {
        let read = stream.read(&mut buffer)?;
        if read == 0 {
            return Ok(());
        }
        bytes.extend_from_slice(&buffer[..read]);
        if bytes.len() > 2 * 1_048_576 {
            return Err(std::io::Error::other("fixture request exceeded bound"));
        }
        if let Some(position) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
    };
    let headers = std::str::from_utf8(&bytes[..header_end])
        .map_err(|_| std::io::Error::other("fixture request headers are not UTF-8"))?
        .to_owned();
    let content_length = headers
        .lines()
        .find_map(|line| {
            line.split_once(':').and_then(|(name, value)| {
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
        })
        .unwrap_or(0);
    while bytes.len().saturating_sub(header_end) < content_length {
        let read = stream.read(&mut buffer)?;
        if read == 0 {
            return Err(std::io::Error::other("fixture request body was truncated"));
        }
        bytes.extend_from_slice(&buffer[..read]);
    }
    let request_line = headers.lines().next().unwrap_or_default().to_owned();
    if let Ok(mut lines) = request_lines.lock() {
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
    let command = |arguments: &[&str]| -> HarnessResult<String> {
        let output = std::process::Command::new("/usr/bin/git")
            .args(arguments)
            .current_dir(&root)
            .output()
            .map_err(|error| error.to_string())?;
        if !output.status.success() {
            return Err(format!("git {} failed", arguments.join(" ")));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    };
    Ok((
        command(&["rev-parse", "HEAD"])?,
        command(&["rev-parse", "HEAD^{tree}"])?,
        !command(&["status", "--porcelain"])?.is_empty(),
    ))
}

fn rust_build_target() -> HarnessResult<String> {
    let output = std::process::Command::new("/usr/bin/rustc")
        .arg("-vV")
        .output()
        .map_err(|error| format!("rustc build-target query failed: {error}"))?;
    if !output.status.success() {
        return Err("rustc build-target query failed".to_owned());
    }
    String::from_utf8(output.stdout)
        .map_err(|error| error.to_string())?
        .lines()
        .find_map(|line| line.strip_prefix("host: ").map(str::to_owned))
        .filter(|target| !target.is_empty())
        .ok_or_else(|| "rustc build-target query omitted the host triple".to_owned())
}

fn redaction_check(path: &Path, forbidden: &[&str]) -> HarnessResult {
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    let text = String::from_utf8(bytes).map_err(|error| error.to_string())?;
    if forbidden
        .iter()
        .any(|value| !value.is_empty() && text.contains(value))
    {
        return Err("redaction validation found a secret value in the report".to_owned());
    }
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

fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
        .unwrap_or(0)
}
