//! Output transfer keeps its durable lease and releases staging when work stops.
use std::io::{BufRead, BufReader};

use milkdrift_persistence::ArtifactStore;
use milkdrift_redb_store::RedbStore;
use milkdrift_workspace::{
    ArtifactId, ArtifactProvenance, ArtifactReference, ArtifactRetention, ArtifactSensitivity,
    CausalId, CausalReference, ContentDigest, MediaType,
};

use super::*;
use milkdrift_capability_host::{CorePeerArtifactStore, PeerArtifactStore};

#[derive(Clone, Copy)]
enum TransferOutcome {
    Complete,
    LeaseRefused,
    Shutdown,
}

struct LeaseReporter {
    clock: Arc<ControlledClock>,
    expires_at: Arc<AtomicU64>,
    refuse_after: Option<u64>,
}

impl AdapterReporter for LeaseReporter {
    fn invocation(&self, _event: InvocationEvent) -> Result<(), AdapterError> {
        Err(AdapterError::rejected(
            "transfer must not report an invocation",
        ))
    }

    fn heartbeat(&self) -> Result<(), AdapterError> {
        let now = self
            .clock
            .now_unix_ms()
            .map_err(|error| AdapterError::external_failure(error.to_string()))?;
        if self.refuse_after.is_some_and(|limit| now >= limit) {
            return Err(AdapterError::external_failure(
                "durable lease renewal refused",
            ));
        }
        self.expires_at.store(now + 100, Ordering::SeqCst);
        Ok(())
    }
}

fn request_target(stream: &mut TcpStream) -> Result<String, Box<dyn std::error::Error>> {
    // Windows accepted sockets can inherit the listener's nonblocking mode.
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let target = line
        .split_whitespace()
        .nth(1)
        .ok_or("missing target")?
        .to_owned();
    let mut length = 0;
    let mut header_bytes = line.len();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Err("incomplete request".into());
        }
        header_bytes += line.len();
        if header_bytes > 8192 {
            return Err("request headers exceed fixture bound".into());
        }
        if line == "\r\n" {
            break;
        }
        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            length = value.trim().parse::<usize>()?;
        }
    }
    if length > 8192 {
        return Err("request body exceeds fixture bound".into());
    }
    reader.read_exact(&mut vec![0; length])?;
    Ok(target)
}

fn transfer_case(outcome: TransferOutcome) -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let core = Arc::new(RedbStore::open(root.path())?);
    let clock = Arc::new(ControlledClock::new(100));
    let artifacts = Arc::new(CorePeerArtifactStore::new(
        core.clone(),
        1024,
        1024,
        clock.clone(),
    )?);
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
    listener.set_nonblocking(true)?;
    let mut fixture = remote_case(ConformanceScenario::Lifecycle)?;
    let adapter = Arc::get_mut(&mut fixture.adapter).ok_or("adapter already shared")?;
    adapter.client = PeerHttpClient::new(PeerClientConfig {
        endpoint: Url::parse(&format!("http://{}/", listener.local_addr()?))?,
        local_peer: adapter.client.local_peer().clone(),
        expected_remote_peer: adapter.client.remote_peer().clone(),
        session: SessionId::new("artifact-client")?,
        versions: ProtocolVersionRange::default(),
        bearer_credential: adapter.relationship.bearer_credential.clone(),
        insecure_loopback: InsecureLoopbackMode::AllowInsecureLoopbackDevelopment,
        request_timeout: Duration::from_secs(2),
        observation_poll_interval: Duration::from_millis(1),
    })?;
    adapter.artifacts = artifacts.clone();
    adapter.clock = clock.clone();
    adapter
        .relationship
        .artifact_sensitivities
        .insert(ArtifactSensitivity::Restricted);
    adapter.start()?;
    let content = b"abcdef";
    let offer = milkdrift_peer_protocol::ArtifactMetadataOffer {
        transfer: milkdrift_peer_protocol::TransferId::new("output-transfer")?,
        direction: milkdrift_peer_protocol::ArtifactTransferDirection::Download,
        artifact: ArtifactReference::new(
            ArtifactId::new("chunked-output")?,
            ContentDigest::for_bytes(content),
            MediaType::new("text/plain")?,
            content.len() as u64,
        ),
        sensitivity: ArtifactSensitivity::Restricted,
        retention: ArtifactRetention::WhileReferenced,
        provenance: ArtifactProvenance::new(
            CausalReference::External {
                source: CausalId::new("serving-execution")?,
            },
            Vec::new(),
        )?,
        source_peer: adapter.client.remote_peer().clone(),
        binding: milkdrift_peer_protocol::ArtifactTransferBinding::Execution {
            execution: PeerExecutionId::new("chunked-execution")?,
        },
        expires_at_unix_ms: 1000,
    };
    let observation = PeerObservation {
        execution: offer
            .binding
            .execution()
            .ok_or("missing output execution")?
            .clone(),
        sequence: 1,
        category: ObservationCategory::Artifact,
        observed_at_unix_ms: 100,
        event: InvocationEvent::new(
            fixture.request.invocation().clone(),
            1,
            InvocationEventKind::Output {
                name: "result".to_owned(),
                reference: milkdrift_capability::ArtifactReference::new(
                    offer.artifact.artifact().as_str(),
                    offer.artifact.digest().to_hex(),
                    Some("text/plain".to_owned()),
                    Some(content.len() as u64),
                )?,
            },
        )?,
    };
    // Each request consumes most of a lease. Multiple chunks require renewals even
    // though the remote execution has already finished and emits no more progress.
    let expires_at = Arc::new(AtomicU64::new(200));
    let reporter = LeaseReporter {
        clock: clock.clone(),
        expires_at: expires_at.clone(),
        refuse_after: matches!(outcome, TransferOutcome::LeaseRefused).then_some(190),
    };
    let server_offer = offer.clone();
    let serving_adapter = fixture.adapter.clone();
    let server = std::thread::spawn(move || -> Result<usize, String> {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut offset = 0;
        loop {
            let (mut stream, _) = match listener.accept() {
                Ok(connection) => connection,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if std::time::Instant::now() >= deadline {
                        return Err("transfer did not release its remote state".to_owned());
                    }
                    std::thread::sleep(Duration::from_millis(1));
                    continue;
                }
                Err(error) => return Err(error.to_string()),
            };
            let target = request_target(&mut stream).map_err(|error| error.to_string())?;
            if target.ends_with("/artifact") {
                write_response(&mut stream, &server_offer)?;
            } else if target.ends_with("/negotiate") {
                write_response(
                    &mut stream,
                    milkdrift_peer_protocol::ArtifactTransferDecision::Transfer {
                        next_offset: 0,
                        maximum_chunk_bytes: 2,
                    },
                )?;
            } else if target.ends_with("/abort") {
                write_response(&mut stream, ())?;
                return Ok(offset);
            } else {
                if !target.ends_with(&format!("/content?offset={offset}&maximum_bytes=2"))
                    || offset >= content.len()
                {
                    return Err(format!("unexpected chunk request: {target}"));
                }
                let now = clock.now.fetch_add(90, Ordering::SeqCst) + 90;
                if now >= expires_at.load(Ordering::SeqCst) {
                    return Err("output transfer let its durable lease expire".to_owned());
                }
                let next = offset + 2;
                if matches!(outcome, TransferOutcome::Shutdown) {
                    serving_adapter
                        .shutdown()
                        .map_err(|error| error.to_string())?;
                }
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nx-milkdrift-artifact-offset: {offset}\r\nx-milkdrift-artifact-final: {}\r\nConnection: close\r\n\r\n", next == content.len()).map_err(|error| error.to_string())?;
                stream
                    .write_all(&content[offset..next])
                    .map_err(|error| error.to_string())?;
                offset = next;
            }
        }
    });
    let mut imported = BTreeSet::new();
    let mut total = 0;
    let result = fixture.adapter.import_output(
        offer
            .binding
            .execution()
            .ok_or("missing output execution")?,
        &observation,
        1000,
        &mut imported,
        &mut total,
        None,
        &reporter,
    );
    let received = server.join().map_err(|_| "transfer server panicked")??;
    let complete = matches!(outcome, TransferOutcome::Complete);
    assert_eq!(result.is_ok(), complete, "{result:?}");
    assert_eq!(received, if complete { content.len() } else { 2 });
    assert_eq!(
        core.metadata(offer.artifact.artifact())?.is_some(),
        complete
    );
    assert_eq!(imported.len(), usize::from(complete));
    assert_eq!(total, if complete { content.len() as u64 } else { 0 });
    assert!(
        artifacts
            .transfer_facts(fixture.adapter.client.remote_peer(), &offer.transfer)
            .is_err()
    );
    Ok(())
}

#[test]
fn multi_chunk_output_renews_lease_until_verified_publication()
-> Result<(), Box<dyn std::error::Error>> {
    transfer_case(TransferOutcome::Complete)
}

#[test]
fn refused_lease_renewal_stops_output_transfer_and_aborts_staging()
-> Result<(), Box<dyn std::error::Error>> {
    transfer_case(TransferOutcome::LeaseRefused)
}

#[test]
fn shutdown_stops_output_transfer_and_aborts_staging() -> Result<(), Box<dyn std::error::Error>> {
    transfer_case(TransferOutcome::Shutdown)
}
