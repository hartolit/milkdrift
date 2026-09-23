//! Attached-model contract evidence through the real serving ledger; no container/isolation claim.
mod support;
use milkdrift_capability::{managed::*, *};
use milkdrift_capability_host::{conformance::*, managed::*, *};
use milkdrift_managed_linux::*;
use milkdrift_persistence::{managed::*, *};
use milkdrift_redb_store::RedbStore;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::Arc,
    time::Duration,
};
use support::Result;

#[test]
fn attached_model_adapter_passes_shared_conformance_with_durable_holds() -> Result {
    if !cfg!(target_os = "linux") {
        return Ok(());
    }
    run_adapter_conformance(|scenario| -> Result<AdapterConformanceCase> {
        let directory = tempfile::tempdir()?;
        let quadlet = directory.path().join("quadlets");
        std::fs::create_dir(&quadlet)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))?;
            std::fs::set_permissions(&quadlet, std::fs::Permissions::from_mode(0o700))?;
        }
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let base = format!("http://{}/v1", listener.local_addr()?);
        let recipe = LinuxRecipe {
            schema_version: 2,
            name: ManagedName::new("recipe")?,
            worker_image: format!("sha256:{}", "a".repeat(64)),
            worker_network: WorkerNetwork::None,
            worker_limits: milkdrift_managed_linux::ContainerLimits {
                memory_bytes: 536_870_912,
                cpu_percent: 100,
                pids: 64,
                temporary_bytes: 67_108_864,
            },
            task_timeout_ms: 10_000,
            output_bytes: 65_536,
            minimum_free_bytes: 1,
            data_disposition: DataDisposition::Preserve,
            model_service: ModelService::Attached {
                api_base: base,
                model_alias: "mock-model".to_owned(),
                endpoint_limits: milkdrift_model_provider::EndpointLimits {
                    connect_timeout_ms: 1000,
                    request_timeout_ms: 3000,
                    idle_timeout_ms: 3000,
                    max_headers: 32,
                    max_header_bytes: 8192,
                    max_request_bytes: 65_536,
                    max_response_bytes: 65_536,
                    max_stream_line_bytes: 8192,
                    max_stream_event_bytes: 16_384,
                    max_fragment_bytes: 1024,
                },
                billing: milkdrift_model_provider::BillingTerms::Unbilled {
                    source: "local deterministic fixture, no provider charge".to_owned(),
                },
                token_limits: milkdrift_model_provider::ModelTokenLimits::ByteBpe {
                    template_tokens_per_message: 0,
                    template_tokens_per_request: 0,
                    maximum_input_tokens: 8192,
                    maximum_output_tokens: 4096,
                    output_control: milkdrift_model_provider::OutputTokenControl::MaxTokens,
                    source: "deterministic bounded fixture contract; no real-model qualification"
                        .to_owned(),
                },
            },
        };
        let path = directory.path().join("recipe.json");
        std::fs::write(&path, serde_json::to_vec(&recipe)?)?;
        let platform = Arc::new(LinuxManagedPlatform::new(LinuxManagerConfig {
            state_root: directory.path().to_path_buf(),
            systemd_directory: quadlet.clone(),
            quadlet_directory: quadlet,
            recipes: vec![path],
        })?);
        let install = ManagedName::new("conformance")?;
        let setup = platform.plan(
            &install,
            &recipe.reference()?,
            &format!("b3_{}", "1".repeat(64)),
            1,
        )?;
        let store = Arc::new(RedbStore::open(directory.path().join("store"))?);
        let request = ManagedRequest {
            schema_version: milkdrift_capability::managed::MANAGED_SCHEMA_VERSION,
            command: ManagedName::new("apply")?,
            installation: install.clone(),
            expected_version: 0,
            action: ManagedAction::Apply {
                recipe: recipe.reference()?,
            },
        };
        let mut authority = support::caller()?;
        authority.operation = milkdrift_authority::AuthorityOperation::AdministerCapabilities;
        authority.resources.capability = Some(CapabilityId::new("managed.conformance")?);
        authority.resources.capability_operation = Some(OperationId::new("resource.apply")?);
        let identity = format!("b3_{}", "2".repeat(64));
        store.begin_managed_change(
            &request,
            &support::decision(authority)?,
            &ManagedChange {
                entry_authorization: None,
                identity: identity.clone(),
                candidate: setup.clone(),
                generation: 1,
                steps: vec![ManagedStep::Verify],
                removing: false,
                running: false,
            },
        )?;
        store.advance_managed_change(
            &install,
            &identity,
            0,
            ManagedObservation {
                digest: identity.clone(),
                summary: "fixture attached service; no OS evidence".to_owned(),
                running: false,
            },
        )?;
        let descriptor = setup
            .capabilities
            .iter()
            .find(|d| *d.category() == CapabilityCategory::Model)
            .ok_or("model missing")?
            .clone();
        let task = milkdrift_model::ModelTaskRequest::new(
            vec![milkdrift_model::Message::new(
                milkdrift_model::MessageRole::User,
                vec![milkdrift_model::ContentPart::Text {
                    text: "complete".to_owned(),
                }],
                None,
            )?],
            Vec::new(),
            None,
            milkdrift_model::SessionSelection::Fresh,
            None,
            8,
            false,
            Default::default(),
        )?;
        let task = milkdrift_model::ModelTaskRequestDocument::new(task);
        let (invocation, context, record) = support::entered(
            &store,
            &descriptor,
            "model-contract",
            milkdrift_model::MODEL_TASK_INPUT_NAME,
            serde_json::to_value(task)?,
        )?;
        let data = Arc::new(StoreInvocationDataAccess::new(
            store.clone(),
            directory.path().join("temporary"),
            ArtifactReadAuthority::PublicOnly,
        )?);
        let adapter = Arc::new(ManagedModelAdapter::new(
            platform,
            store.clone(),
            setup,
            Arc::new(InMemorySecretResolver::new()),
            data,
        )?);
        let server = scenario.executes().then(|| std::thread::spawn(move || -> std::result::Result<(), String> {
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            let mut stream = loop { match listener.accept() { Ok((stream, _)) => break stream, Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && std::time::Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)), Err(e) => return Err(e.to_string()) } };
            stream.set_read_timeout(Some(Duration::from_secs(5))).map_err(|e| e.to_string())?;
            let mut bytes = vec![];
            loop { let mut chunk = [0; 4096]; let n = stream.read(&mut chunk).map_err(|e| e.to_string())?; if n == 0 { return Err("closed request".to_owned()); } bytes.extend_from_slice(&chunk[..n]);
                if bytes.len() > 131_072 { return Err("request too large".to_owned()); }
                if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&bytes[..end]);
                    let len = head.lines().find_map(|l| l.to_lowercase().strip_prefix("content-length:").and_then(|v| v.trim().parse::<usize>().ok())).ok_or("length missing")?;
                    if bytes.len() >= end + 4 + len { break; }
                }
            }
            if !bytes.starts_with(b"POST /v1/chat/completions ") { return Err("wrong endpoint".to_owned()); }
            let body = r#"{"id":"r","model":"mock-model","choices":[{"message":{"content":"complete"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1}}"#;
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).map_err(|e| e.to_string())
        }));
        Ok(AdapterConformanceCase::new(
            adapter,
            descriptor,
            invocation,
            context,
            AdapterConformanceExpectations {
                start_replay: StartReplayExpectation::Idempotent,
                available_while_draining: false,
                available_after_shutdown: false,
                unknown_cancellation: UnknownCancellationExpectation::NegativeAcknowledgement,
            },
        )?
        .with_serving_allowance(record.request.limits.clone())
        .with_cleanup(move || {
            if let Some(server) = server {
                server.join().map_err(|_| "server panicked")??;
            }
            let id = managed_use_id(&record.managed_invocation().map_err(|e| e.to_string())?);
            let usage = store
                .managed_use(&id)
                .map_err(|e| e.to_string())?
                .ok_or("hold missing")?;
            if (scenario == ConformanceScenario::Execution
                || scenario == ConformanceScenario::HostDrain)
                && !matches!(usage.phase, ManagedUsePhase::Quiescent { .. })
            {
                return Err("response did not prove request quiescence".to_owned());
            }
            if scenario == ConformanceScenario::ReporterFailure {
                let claim = record.phase.claim().ok_or("claim absent")?;
                store
                    .mark_peer_uncertain(
                        &record.caller,
                        &record.execution,
                        &claim.worker,
                        claim.generation,
                        103,
                        "fixture reporter response loss",
                    )
                    .map_err(|e| e.to_string())?;
                let archived = store
                    .archive_peer_executions(&PeerRetentionRequest {
                        terminal_before_or_at: TimestampMillis::new(104),
                        archived_at: TimestampMillis::new(105),
                        limit: PageSize::new(16).map_err(|e| e.to_string())?,
                    })
                    .map_err(|e| e.to_string())?;
                if archived.archived != 1
                    || store.managed_use(&id).map_err(|e| e.to_string())?.as_ref() != Some(&usage)
                {
                    return Err(
                        "serving compaction changed unresolved managed ownership".to_owned()
                    );
                }
            }
            store.verify_managed_integrity().map_err(|e| e.to_string())
        })
        .with_keepalive(directory))
    })?;
    Ok(())
}
