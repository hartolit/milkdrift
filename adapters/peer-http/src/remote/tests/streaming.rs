//! A continuously productive peer still needs the origin's durable lease.
use super::*;
use milkdrift_peer_protocol::{InvocationLookup, ObservationHistory, ObservationPage};

struct LeaseReporter {
    clock: Arc<ControlledClock>,
    expires: Arc<AtomicU64>,
    seen: AtomicU64,
    refuse: bool,
}

impl AdapterReporter for LeaseReporter {
    fn invocation(&self, event: InvocationEvent) -> Result<(), AdapterError> {
        if self.clock.now.load(Ordering::SeqCst) >= self.expires.load(Ordering::SeqCst) {
            return Err(AdapterError::external_failure("origin lease expired"));
        }
        assert_eq!(
            event.sequence(),
            self.seen.fetch_add(1, Ordering::SeqCst) + 1
        );
        Ok(())
    }

    fn heartbeat(&self) -> Result<(), AdapterError> {
        if self.clock.now.load(Ordering::SeqCst) >= self.expires.load(Ordering::SeqCst) {
            return Err(AdapterError::external_failure(
                "cannot renew an expired origin lease",
            ));
        }
        if self.refuse && self.clock.now.load(Ordering::SeqCst) >= 190 {
            return Err(AdapterError::external_failure("renewal refused"));
        }
        self.expires.store(
            self.clock.now.load(Ordering::SeqCst) + 100,
            Ordering::SeqCst,
        );
        Ok(())
    }
}

fn streaming_case(
    refuse: bool,
    delayed_submission: bool,
    lost_submission_replies: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = remote_case(ConformanceScenario::Lifecycle)?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let adapter = Arc::get_mut(&mut fixture.adapter).ok_or("adapter already shared")?;
    adapter.client = PeerHttpClient::new(PeerClientConfig {
        endpoint: Url::parse(&format!("http://{}/", listener.local_addr()?))?,
        local_peer: adapter.client.local_peer().clone(),
        expected_remote_peer: adapter.client.remote_peer().clone(),
        session: SessionId::new("streaming-client")?,
        versions: ProtocolVersionRange::default(),
        bearer_credential: adapter.relationship.bearer_credential.clone(),
        insecure_loopback: InsecureLoopbackMode::AllowInsecureLoopbackDevelopment,
        request_timeout: Duration::from_secs(2),
        observation_poll_interval: Duration::from_millis(1),
    })?;
    let clock = Arc::new(ControlledClock::new(100));
    adapter.clock = clock.clone();
    let remote = adapter.client.remote_peer().clone();
    // Input cleanup may have consumed most of the current lease before submission starts.
    let expires = Arc::new(AtomicU64::new(if delayed_submission { 110 } else { 200 }));
    let reporter = LeaseReporter {
        clock: clock.clone(),
        expires: expires.clone(),
        seen: AtomicU64::new(0),
        refuse,
    };
    let server = thread::spawn(move || -> Result<(), String> {
        let accept = || -> Result<TcpStream, String> {
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            loop {
                match listener.accept() {
                    Ok((stream, _)) => {
                        stream.set_nonblocking(false).map_err(|e| e.to_string())?;
                        return Ok(stream);
                    }
                    Err(e)
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            && std::time::Instant::now() < deadline =>
                    {
                        thread::sleep(Duration::from_millis(1))
                    }
                    Err(e) => return Err(e.to_string()),
                }
            }
        };
        let mut stream = accept()?;
        read_request_body(&mut stream)?;
        write_response(
            &mut stream,
            HandshakeResponse {
                peer: remote,
                session: SessionId::new("streaming-server").map_err(|e| e.to_string())?,
                selected_version: ProtocolVersion::V1_5,
                features: FeatureSet {
                    resumable_observations: true,
                    resumable_artifacts: true,
                    incremental_catalog: false,
                    archived_execution_replay: true,
                },
                limits: HardLimits::default(),
                lease: HeartbeatLease {
                    heartbeat_ms: 100,
                    idle_timeout_ms: 500,
                    execution_lease_ms: 1000,
                },
                drain: DrainState::Ready,
            },
        )?;
        let mut stream = accept()?;
        let bytes = read_request_body(&mut stream)?;
        let envelope: ProtocolEnvelope<ServingInvocationRequest> =
            decode_envelope(&bytes, DecodeLimits::default()).map_err(|e| e.to_string())?;
        let request = envelope.message;
        if lost_submission_replies {
            for attempt in 0..3 {
                let now = clock.now.fetch_add(90, Ordering::SeqCst) + 90;
                if now >= expires.load(Ordering::SeqCst) {
                    clock.now.store(1001, Ordering::SeqCst);
                    return Err("submission retries exhausted the origin lease".to_owned());
                }
                // The first request was accepted, but every POST reply is lost. Retries must
                // retain its exact bytes, and lookup must recover that same acceptance.
                drop(stream);
                if refuse {
                    return Ok(());
                }
                stream = accept()?;
                let bytes = read_request_body(&mut stream)?;
                if attempt < 2 {
                    let repeated: ProtocolEnvelope<ServingInvocationRequest> =
                        decode_envelope(&bytes, DecodeLimits::default())
                            .map_err(|e| e.to_string())?;
                    if repeated.message != request {
                        return Err("retry changed accepted request".to_owned());
                    }
                }
            }
            let now = clock.now.fetch_add(90, Ordering::SeqCst) + 90;
            if now >= expires.load(Ordering::SeqCst) {
                clock.now.store(1001, Ordering::SeqCst);
                return Err("acceptance lookup exhausted the origin lease".to_owned());
            }
        }
        if delayed_submission {
            let now = clock.now.fetch_add(90, Ordering::SeqCst) + 90;
            if now >= expires.load(Ordering::SeqCst) {
                clock.now.store(1001, Ordering::SeqCst);
                return Err("submission exhausted the origin lease after input cleanup".to_owned());
            }
        }
        let execution = PeerExecutionId::new("streaming-execution").map_err(|e| e.to_string())?;
        if lost_submission_replies {
            write_response(
                &mut stream,
                InvocationLookup::Known {
                    request_id: request.request_id.clone(),
                    execution: execution.clone(),
                    request_digest: request.request_digest.clone(),
                    accepted_at_unix_ms: 100,
                    status: RemoteExecutionStatus::Running,
                    last_sequence: 0,
                    history: ObservationHistory::Hot,
                },
            )?;
        } else {
            write_response(
                &mut stream,
                InvocationAcceptance::Accepted {
                    request_id: request.request_id.clone(),
                    execution: execution.clone(),
                    request_digest: request.request_digest.clone(),
                    accepted_at_unix_ms: 100,
                    lease_expires_at_unix_ms: 1000,
                    replayed: false,
                },
            )?;
        }
        for sequence in 1..=if refuse { 1 } else { 4 } {
            let mut stream = accept()?;
            read_request_body(&mut stream)?;
            let now = clock.now.fetch_add(90, Ordering::SeqCst) + 90;
            if now >= expires.load(Ordering::SeqCst) {
                clock.now.store(1001, Ordering::SeqCst);
                return Err("continuous output exhausted the origin lease".to_owned());
            }
            let terminal = sequence == 4;
            let kind = if terminal {
                InvocationEventKind::Terminal {
                    terminal: InvocationTerminal::new(
                        TerminalStatus::Success,
                        Vec::new(),
                        None,
                        None,
                        request.selection.operation_contract().side_effect(),
                    )
                    .map_err(|e| e.to_string())?,
                }
            } else {
                InvocationEventKind::Progress {
                    message: "still generating".to_owned(),
                    completed_units: None,
                    total_units: None,
                }
            };
            let event = InvocationEvent::new(request.request.invocation().clone(), sequence, kind)
                .map_err(|e| e.to_string())?;
            write_response(
                &mut stream,
                ObservationPage {
                    status: if terminal {
                        RemoteExecutionStatus::Terminal
                    } else {
                        RemoteExecutionStatus::Running
                    },
                    execution: execution.clone(),
                    after_sequence: sequence - 1,
                    next_sequence: sequence,
                    terminal,
                    closed: terminal,
                    history: ObservationHistory::Hot,
                    observations: vec![PeerObservation {
                        execution: execution.clone(),
                        sequence,
                        category: if terminal {
                            ObservationCategory::Terminal
                        } else {
                            ObservationCategory::Progress
                        },
                        observed_at_unix_ms: now,
                        event,
                    }],
                },
            )?;
        }
        Ok(())
    });
    let host = CapabilityHost::new(
        HostConfig {
            max_registrations: 1,
            max_generations_per_capability: 1,
            max_concurrent_per_generation: 1,
            observation_stale_after_ms: 1000,
        },
        CapabilitySelectionPolicy::priorities(BTreeMap::new()),
    )?;
    host.register(fixture.descriptor.clone(), fixture.adapter.clone(), None)?;
    let snapshot = ResolvedCapabilitySnapshot::from_descriptor(
        &fixture.descriptor,
        fixture.request.operation(),
    )?;
    let result =
        host.execute_exact_with_context(&snapshot, &fixture.request, &fixture.context, &reporter);
    server.join().map_err(|_| "streaming server panicked")??;
    if refuse {
        assert!(
            result
                .err()
                .ok_or("renewal refusal must stop execution")?
                .to_string()
                .contains("renewal refused")
        );
        assert_eq!(reporter.seen.load(Ordering::SeqCst), 0);
    } else {
        result?;
        assert_eq!(reporter.seen.load(Ordering::SeqCst), 4);
    }
    assert!(
        fixture
            .adapter
            .active
            .lock()
            .map_err(|_| "active map poisoned")?
            .is_empty()
    );
    Ok(())
}

#[test]
fn continuous_peer_progress_renews_origin_lease() -> Result<(), Box<dyn std::error::Error>> {
    streaming_case(false, false, false)
}

#[test]
fn continuous_peer_progress_stops_on_renewal_refusal() -> Result<(), Box<dyn std::error::Error>> {
    streaming_case(true, false, false)
}

#[test]
fn submission_and_observation_each_renew_after_prior_request_latency()
-> Result<(), Box<dyn std::error::Error>> {
    streaming_case(false, true, false)
}

#[test]
fn lost_submission_replies_renew_between_retries_and_lookup()
-> Result<(), Box<dyn std::error::Error>> {
    streaming_case(false, false, true)
}

#[test]
fn lost_submission_reply_stops_before_retry_when_renewal_is_refused()
-> Result<(), Box<dyn std::error::Error>> {
    streaming_case(true, false, true)
}
