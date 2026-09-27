//! Archive at each authorization boundary after the real serving workers have stopped.
use super::*;
use milkdrift_capability::{AdmissionBound, AdmissionUnit};
use milkdrift_capability_host::{
    InvocationDataAccess, MaterializationLimits, ServingError, StoreInvocationDataAccess,
};
use milkdrift_persistence::ArtifactReadAuthority;

const CONTENT: &[u8] = b"retained output across archival";

#[derive(Default)]
struct ClockState {
    measuring: bool,
    calls: usize,
    archive_at: Option<usize>,
    archived: u32,
}

struct ArchivingClock {
    store: Arc<RedbStore>,
    timestamp: u64,
    state: Mutex<ClockState>,
}

impl ArchivingClock {
    fn arm(&self, archive_at: Option<usize>) -> TestResult {
        *self.state.lock().map_err(|_| "clock poisoned")? = ClockState {
            measuring: true,
            archive_at,
            ..ClockState::default()
        };
        Ok(())
    }

    fn observed(&self) -> TestResult<(usize, u32)> {
        let state = self.state.lock().map_err(|_| "clock poisoned")?;
        Ok((state.calls, state.archived))
    }
}

impl PeerClock for ArchivingClock {
    fn now_unix_ms(&self) -> Result<u64, PeerClockError> {
        let archive = {
            let mut state = self.state.lock().map_err(|_| PeerClockError::Unavailable)?;
            if state.measuring {
                state.calls += 1;
            }
            let archive = state.archive_at == Some(state.calls);
            if archive {
                state.archive_at = None;
            }
            archive
        };
        if archive {
            let page = self
                .store
                .archive_peer_executions(&PeerRetentionRequest {
                    terminal_before_or_at: TimestampMillis::new(self.timestamp),
                    archived_at: TimestampMillis::new(self.timestamp),
                    limit: PageSize::new(1).map_err(|_| PeerClockError::Unavailable)?,
                })
                .map_err(|_| PeerClockError::Unavailable)?;
            self.state
                .lock()
                .map_err(|_| PeerClockError::Unavailable)?
                .archived += page.archived;
        }
        Ok(self.timestamp)
    }
}

struct OutputAdapter {
    delegate: TerminalAdapter,
    data: StoreInvocationDataAccess,
    output: Arc<Mutex<Option<InvocationArtifactReference>>>,
    terminal_lists_output: bool,
}

impl CapabilityAdapter for OutputAdapter {
    fn accepts_direct_inputs(&self) -> bool {
        true
    }

    fn admission_envelope(
        &self,
        _invocation: &AdapterInvocation<'_>,
    ) -> Result<InvocationAdmissionEnvelope, AdapterError> {
        Ok(InvocationAdmissionEnvelope::new(
            AdmissionUnit::Unknown,
            AdmissionBound::NotApplicable,
            AdmissionBound::NotApplicable,
            AdmissionBound::Bounded(1024),
            AdmissionBound::NotApplicable,
        ))
    }

    fn authority_requirements(&self) -> CapabilityExecutionRequirements {
        self.delegate.authority_requirements()
    }

    fn start(&self) -> Result<(), AdapterError> {
        self.delegate.start()
    }

    fn execute(
        &self,
        invocation: &AdapterInvocation<'_>,
        reporter: &dyn AdapterReporter,
    ) -> Result<(), AdapterError> {
        let error =
            |value: &dyn std::fmt::Display| AdapterError::external_failure(value.to_string());
        let reference = self
            .data
            .publish_bytes(
                invocation
                    .context()
                    .ok_or_else(|| AdapterError::rejected("serving context absent"))?,
                invocation.request(),
                "result",
                "text/plain",
                CONTENT,
                MaterializationLimits {
                    max_files: 1,
                    max_file_bytes: 1024,
                    max_total_bytes: 1024,
                    max_path_bytes: 256,
                    max_directory_depth: 4,
                    chunk_bytes: 1024,
                },
            )
            .map_err(|e| error(&e))?;
        *self
            .output
            .lock()
            .map_err(|_| AdapterError::external_failure("output mutex poisoned"))? =
            Some(reference.clone());
        reporter.invocation(
            InvocationEvent::new(
                invocation.request().invocation().clone(),
                1,
                InvocationEventKind::Output {
                    name: "result".to_owned(),
                    reference: reference.clone(),
                },
            )
            .map_err(|e| error(&e))?,
        )?;
        reporter.invocation(
            InvocationEvent::new(
                invocation.request().invocation().clone(),
                2,
                InvocationEventKind::Terminal {
                    terminal: InvocationTerminal::new(
                        TerminalStatus::Success,
                        if self.terminal_lists_output {
                            vec![reference]
                        } else {
                            Vec::new()
                        },
                        None,
                        None,
                        SideEffectClass::ReadOnly,
                    )
                    .map_err(|e| error(&e))?,
                },
            )
            .map_err(|e| error(&e))?,
        )
    }

    fn cancel(
        &self,
        request: &CancellationRequest,
    ) -> Result<CancellationAcknowledgement, AdapterError> {
        self.delegate.cancel(request)
    }

    fn health(&self, timestamp: u64) -> Result<CapabilityObservation, AdapterError> {
        self.delegate.health(timestamp)
    }

    fn begin_drain(&self) -> Result<(), AdapterError> {
        self.delegate.begin_drain()
    }

    fn shutdown(&self) -> Result<(), AdapterError> {
        self.delegate.shutdown()
    }
}

struct OutputFixture {
    _root: tempfile::TempDir,
    store: Arc<RedbStore>,
    host: CapabilityHost,
    service: Arc<PeerService>,
    clock: Arc<ArchivingClock>,
    actor: ActorRef,
    peer: PeerId,
    execution: PeerExecutionId,
    reference: InvocationArtifactReference,
}

impl OutputFixture {
    fn new(read_output: bool, peer_call: bool) -> TestResult<Self> {
        let root = tempfile::tempdir()?;
        let store = Arc::new(RedbStore::open(root.path())?);
        let clock = Arc::new(ArchivingClock {
            store: store.clone(),
            timestamp: now(),
            state: Mutex::new(ClockState::default()),
        });
        let actor = ActorRef::new("output-client")?;
        let peer = PeerId::new("output-peer")?;
        let target = PeerId::new("output-host")?;
        let output = Arc::new(Mutex::new(None));
        let (host, descriptor) = host_with_adapter(Arc::new(OutputAdapter {
            delegate: TerminalAdapter {
                capability: CapabilityId::new("test-capability")?,
                delay: Duration::ZERO,
                active: Arc::new(AtomicUsize::new(0)),
                maximum: Arc::new(AtomicUsize::new(0)),
                calls: Arc::new(AtomicUsize::new(0)),
                requirements: CapabilityExecutionRequirements::default(),
            },
            data: StoreInvocationDataAccess::new(
                store.clone(),
                root.path().join("execution-output"),
                ArtifactReadAuthority::PublicOnly,
            )?,
            output: output.clone(),
            // Peer coverage checks output rows retained only by output_observations.
            terminal_lists_output: !peer_call,
        }))?;
        let mut config = server_config(peer.clone(), target.clone(), 1, 2)?;
        if !read_output {
            config.relationships[0]
                .authority
                .actions
                .remove(&PeerAction::ArtifactDownload);
            config.relationships[0]
                .authority
                .actions
                .remove(&PeerAction::ArtifactUpload);
        }
        let mut policy = client_policy(std::slice::from_ref(&actor))?;
        if read_output {
            let previous = &policy.grants[0];
            let mut operations = previous.operations().clone();
            operations.insert(AuthorityOperation::ReadCapabilityOutput);
            policy.grants[0] =
                AuthorityGrantBuilder::new(previous.identity().clone(), 1, actor.clone())
                    .operations(operations)
                    .resources(previous.resources().clone())
                    .budget(previous.budget())
                    .build()?;
        }
        let artifacts = Arc::new(CorePeerArtifactStore::new(
            store.clone(),
            1024,
            2048,
            clock.clone(),
        )?);
        let service = PeerService::with_clients(
            config,
            host.clone(),
            store.clone(),
            artifacts,
            None,
            Some(policy.clone()),
            clock.clone(),
        )?;
        service.recover(32)?;
        let accepted = if peer_call {
            let catalog = service.catalog(&peer)?;
            service.invoke(
                &peer,
                request(
                    &peer,
                    &target,
                    &descriptor,
                    catalog.generation,
                    catalog.digest,
                    "output-request",
                    "output-invocation",
                )?,
            )?
        } else {
            let catalog = service.client_catalog(&actor)?;
            service.invoke_client(
                &actor,
                &DirectInvocationRequest {
                    host: target.clone(),
                    request_id: PeerRequestId::new("output-request")?,
                    catalog_generation: catalog.generation,
                    catalog_digest: catalog.digest,
                    selection: ResolvedCapabilitySnapshot::from_descriptor(
                        &descriptor,
                        &OperationId::new("test.execute")?,
                    )?,
                    request: InvocationRequest::new(
                        InvocationId::new("output-invocation")?,
                        descriptor.identity().clone(),
                        OperationId::new("test.execute")?,
                        None,
                        None,
                        Vec::new(),
                        BTreeMap::new(),
                    )?,
                    limits: policy.execution_limits,
                    deadline_unix_ms: clock.timestamp + 30_000,
                },
            )?
        };
        let InvocationAcceptance::Accepted { execution, .. } = accepted else {
            return Err(format!("output request refused: {accepted:?}").into());
        };
        let caller = milkdrift_peer_protocol::ServingCaller {
            host: target,
            principal: if peer_call {
                ServingPrincipal::Peer { peer: peer.clone() }
            } else {
                ServingPrincipal::Client {
                    actor: actor.clone(),
                }
            },
        };
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if matches!(
                store.peer_execution(&caller, &execution)?,
                Some(PeerExecutionSnapshot::Hot(record))
                    if matches!(record.phase, PeerExecutionPhase::Terminal { .. })
            ) {
                break;
            }
            if std::time::Instant::now() >= deadline {
                return Err("real output adapter did not reach terminal".into());
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert!(service.shutdown_workers(Duration::from_secs(5)).clean);
        let reference = output
            .lock()
            .map_err(|_| "output mutex poisoned")?
            .clone()
            .ok_or("adapter did not publish output")?;
        Ok(Self {
            _root: root,
            store,
            host,
            service,
            clock,
            actor,
            peer,
            execution,
            reference,
        })
    }

    fn read_output(&self) -> TestResult {
        let result = self.service.client_output(
            &self.actor,
            &self.execution,
            self.reference.identity(),
            0,
            1024,
        )?;
        assert_eq!(result.execution, self.execution);
        assert_eq!(result.bytes, CONTENT);
        assert_eq!(
            result.metadata.reference().artifact().as_str(),
            self.reference.identity()
        );
        assert!(result.complete);
        Ok(())
    }

    fn finish(self) -> TestResult {
        self.store.verify_peer_execution_integrity()?;
        self.host.shutdown()?;
        Ok(())
    }
}

#[test]
fn client_terminal_output_survives_archival_during_authorization() -> TestResult {
    let baseline = OutputFixture::new(true, false)?;
    baseline.clock.arm(None)?;
    baseline.read_output()?;
    let (boundaries, _) = baseline.clock.observed()?;
    assert!((2..=16).contains(&boundaries));
    baseline.finish()?;
    for boundary in 1..=boundaries {
        let fixture = OutputFixture::new(true, false)?;
        fixture.clock.arm(Some(boundary))?;
        fixture
            .read_output()
            .map_err(|error| format!("archival boundary {boundary}: {error}"))?;
        assert_eq!(
            fixture.clock.observed()?.1,
            1,
            "boundary {boundary} was not exercised"
        );
        fixture.read_output()?;
        fixture.finish()?;
    }
    Ok(())
}

#[test]
fn client_observation_archival_never_discloses_ungranted_outputs() -> TestResult {
    let baseline = OutputFixture::new(false, false)?;
    baseline.clock.arm(None)?;
    assert!(matches!(
        baseline
            .service
            .client_observations(&baseline.actor, &baseline.execution, 0, 8),
        Err(ServingError::Unauthorized(_))
    ));
    let (boundaries, _) = baseline.clock.observed()?;
    assert!((2..=16).contains(&boundaries));
    baseline.finish()?;
    for boundary in 1..=boundaries {
        let fixture = OutputFixture::new(false, false)?;
        fixture.clock.arm(Some(boundary))?;
        let page = fixture
            .service
            .client_observations(&fixture.actor, &fixture.execution, 0, 8);
        assert!(
            matches!(page, Err(ServingError::Unauthorized(_))),
            "boundary {boundary} disclosed: {page:?}"
        );
        assert_eq!(
            fixture.clock.observed()?.1,
            1,
            "boundary {boundary} was not exercised"
        );
        assert!(matches!(
            fixture
                .service
                .client_observations(&fixture.actor, &fixture.execution, 0, 8),
            Err(ServingError::Unauthorized(_))
        ));
        fixture.finish()?;
    }
    Ok(())
}

#[test]
fn peer_archived_streamed_outputs_keep_artifact_read_permission() -> TestResult {
    let fixture = OutputFixture::new(false, true)?;
    assert!(matches!(
        fixture
            .service
            .observations(&fixture.peer, &fixture.execution, 0, 8),
        Err(ServingError::Unauthorized(_))
    ));
    assert_eq!(
        fixture
            .store
            .archive_peer_executions(&PeerRetentionRequest {
                terminal_before_or_at: TimestampMillis::new(fixture.clock.timestamp),
                archived_at: TimestampMillis::new(fixture.clock.timestamp),
                limit: PageSize::new(1)?,
            })?
            .archived,
        1
    );
    let page = fixture
        .service
        .observations(&fixture.peer, &fixture.execution, 0, 8);
    assert!(
        matches!(page, Err(ServingError::Unauthorized(_))),
        "archived streamed output disclosed: {page:?}"
    );
    fixture.finish()?;
    Ok(())
}
