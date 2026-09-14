//! The actual daemon exposes shared proposal controls while unsafe execution stays closed.
use super::*;
use milkdrift_control::{
    ClaimedStopCondition, ProposalApplicationPolicy, ProposalId, ProposalProvenance,
    WorkflowProposal, WorkflowProposalDocument,
};
use milkdrift_control_client::{BearerCredential, ClientConfig, ControlClient};
use milkdrift_control_protocol::{
    Command, CommandRequest, DaemonState, ProposalDecision, ProtocolVersion,
};
use std::{
    fs,
    net::{Ipv4Addr, TcpListener},
    process::{Child, Command as Process, Stdio},
    time::Instant,
};

struct ProcessGuard(Child);
impl Drop for ProcessGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn request(
    id: &str,
    sequence: Option<u64>,
    revision: Option<String>,
    command: Command,
) -> CommandRequest {
    CommandRequest {
        protocol: ProtocolVersion::CURRENT,
        command_id: id.to_owned(),
        expected_sequence: sequence,
        expected_revision: revision,
        reason: "review and reconcile blocked context".to_owned(),
        evidence: Vec::new(),
        command,
    }
}

async fn launch(config: &Path, endpoint: &url::Url) -> TestResult<(ProcessGuard, ControlClient)> {
    let executable = std::env::current_exe()?
        .parent()
        .and_then(Path::parent)
        .ok_or("target directory")?
        .join(format!("milkdrift-daemon{}", std::env::consts::EXE_SUFFIX));
    let mut process = ProcessGuard(
        Process::new(executable)
            .arg("--recovery")
            .arg("--config")
            .arg(config)
            .env("MILKDRIFT_TOKEN", "recovery-test-credential")
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()?,
    );
    let mut client_config = ClientConfig::new(endpoint.clone());
    client_config.safe_query_retries = 0;
    client_config.request_timeout = Duration::from_secs(3);
    let client = ControlClient::new(
        client_config,
        BearerCredential::new("recovery-test-credential")?,
    )?;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Ok(health) = client.health().await {
            assert_eq!(health.state, DaemonState::Recovery);
            assert!(health.live);
            assert!(!health.ready);
            assert_eq!(health.active_effects, 0);
            assert!(!health.peer_executions.enabled);
            break;
        }
        assert!(process.0.try_wait()?.is_none(), "recovery daemon exited");
        assert!(Instant::now() < deadline, "recovery startup deadline");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(client.readiness().await.is_err());
    Ok((process, client))
}

#[tokio::test]
async fn recovery_binary_authorizes_reviews_replays_and_applies_repair_without_execution()
-> TestResult {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/operator/daemon.toml");
    let text = fs::read_to_string(source)?
        .replace("operator-starter", "context-enforcement")
        .replace("human:operator", "human:structured-runtime-test")
        .replace("grant:operator", "grant:structured-runtime-test")
        .replace(
            "dangerous_allow_broad_authority = false",
            "dangerous_allow_broad_authority = true",
        )
        .replace(
            "maximum_side_effect = \"read_only\"",
            "maximum_side_effect = \"unknown\"",
        )
        .replace(
            "type = \"only\"\nvalues = [\"operator-process\"]",
            "type = \"any\"",
        )
        .replace(
            "values = [\"process.execute\"]",
            "values = [\"model.generate\"]",
        )
        .replace(
            "type = \"only\"\nvalues = [\"local-process\"]",
            "type = \"any\"",
        );
    let configuration: toml::Value = toml::from_str(&text)?;
    let configured = &configuration["actors"][0]["authority"];
    let resources: milkdrift_authority::ResourceScope =
        configured["resources"].clone().try_into()?;
    let budget: AuthorityBudget = configured["budget"].clone().try_into()?;
    let grant = milkdrift_control::AuthorityPreset::Controller
        .template(
            GrantId::new("grant:structured-runtime-test")?,
            1,
            ActorRef::new("human:structured-runtime-test")?,
            resources.workflow_run.clone(),
            resources.capability.clone(),
            budget,
        )
        .resources(resources)
        .validity(
            milkdrift_authority::BoundaryTimeMillis::new(0),
            milkdrift_authority::BoundaryTimeMillis::new(4_102_444_800_000),
        )
        .build()?;
    let claim = CommandAuthorityClaim::new(grant.identity().clone(), 1, grant.digest()?, 0)?;
    let (harness, run, invocation) =
        legacy_schedule_authorized(LegacyCase::DisclosureLoss, &claim)?;
    let old = harness
        .store
        .revision(
            harness
                .runtime
                .projection(&run)?
                .revision()
                .ok_or("revision")?,
        )?
        .ok_or("revision")?;
    let original_history = harness.runtime.history(&run)?;
    let saved =
        ContextManifestDocument::new(manifest(&harness.store, &invocation)?).to_canonical_json()?;
    let attempt = manifest(&harness.store, &invocation)?.attempt().to_string();
    let sequence = harness.store.head(&run)?;
    let mut new_policy = serde_json::to_value(policy("fresh", false, true)?)?;
    new_policy["budget"]["max_bytes"] = json!(100_000);
    let proposal = WorkflowProposalDocument::new(WorkflowProposal::new(
        ProposalId::new("recovery-context-proposal")?,
        ActorRef::new("human:structured-runtime-test")?,
        ProposalProvenance::Direct,
        old.semantic().workflow().clone(),
        Some(run.clone()),
        old.id().clone(),
        old.content_digest().clone(),
        Some(sequence),
        MutationBatch::new(vec![Mutation::ReplaceNode {
            node: work(serde_json::from_value(new_policy)?)?,
        }])?,
        "Select fresh bounded context for replacement work",
        None,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        ProposalApplicationPolicy::RequireApproval,
        None,
        ClaimedStopCondition::Continue,
    )?);
    let directory = harness.close();
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let address = listener.local_addr()?;
    let endpoint = url::Url::parse(&format!("http://{address}/"))?;
    let config_dir = tempfile::tempdir()?;
    let config = config_dir.path().join("daemon.toml");
    let text = text
        .replace(
            "data_root = \"./data\"",
            &format!(
                "data_root = {}",
                serde_json::to_string(&directory.path().to_string_lossy())?
            ),
        )
        .replace("127.0.0.1:9734", &address.to_string());
    fs::write(&config, text)?;
    drop(listener);
    let (process, client) = launch(&config, &endpoint).await?;
    assert_eq!(client.run(run.as_str()).await?.sequence, sequence.get());
    let read = client.attempt(run.as_str(), &attempt).await?;
    assert_eq!(read.context_access, "recovery_redacted");
    assert!(read.context.is_none());
    let unauthenticated = ControlClient::new(
        ClientConfig::new(endpoint.clone()),
        BearerCredential::new("wrong-credential")?,
    )?;
    assert!(unauthenticated.run(run.as_str()).await.is_err());
    let submit = request(
        "recovery-submit",
        Some(sequence.get()),
        Some(old.id().to_string()),
        Command::SubmitProposal {
            document: serde_json::from_slice(&proposal.to_canonical_json()?)?,
        },
    );
    let submitted = client.submit(&submit).await?;
    let proposed = submitted.value["proposed_revision"]
        .as_str()
        .ok_or("proposed revision")?
        .to_owned();
    let digest = proposal.proposal().digest().as_str().to_owned();
    let apply_command = Command::ApplyProposal {
        run_id: run.to_string(),
        proposal_id: "recovery-context-proposal".to_owned(),
        proposal_digest: digest.clone(),
        proposed_revision: proposed.clone(),
    };
    assert!(
        client
            .submit(&request(
                "recovery-unapproved",
                Some(client.run(run.as_str()).await?.sequence),
                Some(proposed.clone()),
                apply_command.clone()
            ))
            .await
            .is_err()
    );
    client
        .submit(&request(
            "recovery-approve",
            Some(client.run(run.as_str()).await?.sequence),
            Some(proposed.clone()),
            Command::DecideProposal {
                run_id: run.to_string(),
                proposal_id: "recovery-context-proposal".to_owned(),
                proposal_digest: digest,
                proposed_revision: proposed.clone(),
                decision_id: "recovery-reviewed".to_owned(),
                decision: ProposalDecision::Approve,
            },
        ))
        .await?;
    let apply = request(
        "recovery-apply",
        Some(client.run(run.as_str()).await?.sequence),
        Some(proposed.clone()),
        apply_command,
    );
    let applied = client.submit(&apply).await?;
    assert_eq!(
        client.run(run.as_str()).await?.revision_id,
        Some(proposed.clone())
    );
    let repaired_sequence = client.run(run.as_str()).await?.sequence;
    assert!(
        client
            .submit(&request(
                "recovery-resume-denied",
                Some(repaired_sequence),
                None,
                Command::ResumeRun {
                    run_id: run.to_string()
                }
            ))
            .await
            .is_err()
    );
    drop(process);
    let (process, client) = launch(&config, &endpoint).await?;
    let replay = client.submit(&apply).await?;
    assert!(replay.replayed);
    assert_eq!(replay.value, applied.value);
    let mut conflict = apply.clone();
    conflict.reason.push_str(" changed");
    assert!(client.submit(&conflict).await.is_err());
    assert_eq!(client.run(run.as_str()).await?.sequence, repaired_sequence);
    drop(process);
    let (store, _, executor, runtime) =
        open_closed_runtime_at(directory.path(), "binary-repaired", NOW, 64)?;
    runtime.initialize_startup()?;
    for _ in 0..16 {
        runtime_tick(&runtime)?;
        if runtime.projection(&run)?.is_completed() {
            break;
        }
    }
    assert!(runtime.projection(&run)?.is_completed());
    assert_eq!(executor.entry_count(), 1);
    assert_eq!(
        &runtime.history(&run)?[..original_history.len()],
        original_history
    );
    assert_eq!(
        saved,
        ContextManifestDocument::new(manifest(&store, &invocation)?).to_canonical_json()?
    );
    Ok(())
}
