use super::*;
use milkdrift_authority::CapabilityExecutionRequirements;
use milkdrift_capability::{
    CancellationAcknowledgement, CancellationRequest, CapabilityDescriptor,
    CapabilityDescriptorDocument, CapabilityObservation, InvocationAdmissionEnvelope,
};
use milkdrift_capability_host::{
    AdapterError, AdapterInvocation, AdapterReporter, CapabilityAdapter,
};
use std::sync::mpsc::{Receiver, SyncSender};

struct ShutdownProbe {
    entered: SyncSender<()>,
    release: Mutex<Receiver<()>>,
    fail: bool,
}
impl CapabilityAdapter for ShutdownProbe {
    fn admission_envelope(
        &self,
        _: &AdapterInvocation<'_>,
    ) -> Result<InvocationAdmissionEnvelope, AdapterError> {
        Err(AdapterError::rejected("lifecycle fixture cannot execute"))
    }
    fn authority_requirements(&self) -> CapabilityExecutionRequirements {
        CapabilityExecutionRequirements::default()
    }
    fn start(&self) -> Result<(), AdapterError> {
        Ok(())
    }
    fn execute(
        &self,
        _: &AdapterInvocation<'_>,
        _: &dyn AdapterReporter,
    ) -> Result<(), AdapterError> {
        Err(AdapterError::rejected("lifecycle fixture cannot execute"))
    }
    fn cancel(&self, _: &CancellationRequest) -> Result<CancellationAcknowledgement, AdapterError> {
        Err(AdapterError::rejected("lifecycle fixture cannot cancel"))
    }
    fn health(&self, _: u64) -> Result<CapabilityObservation, AdapterError> {
        Err(AdapterError::unavailable(
            "lifecycle fixture has no execution health",
        ))
    }
    fn begin_drain(&self) -> Result<(), AdapterError> {
        Ok(())
    }
    fn shutdown(&self) -> Result<(), AdapterError> {
        self.entered
            .send(())
            .map_err(|_| AdapterError::unavailable("shutdown observer absent"))?;
        self.release
            .lock()
            .map_err(|_| AdapterError::unavailable("shutdown gate poisoned"))?
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| AdapterError::unavailable("shutdown release absent"))?;
        if self.fail {
            Err(AdapterError::external_failure(
                "injected adapter shutdown failure",
            ))
        } else {
            Ok(())
        }
    }
}

fn descriptor() -> Result<CapabilityDescriptor, Box<dyn std::error::Error>> {
    Ok(CapabilityDescriptorDocument::from_json(include_bytes!(
        "../../../../../crates/capability/tests/fixtures/descriptor-v1.json"
    ))?
    .body()
    .clone())
}

#[test]
fn startup_cleanup_returns_adapter_failure_and_missing_deadline_evidence()
-> Result<(), Box<dyn std::error::Error>> {
    for deadline in [Duration::ZERO, Duration::from_secs(5)] {
        let host = CapabilityHost::new(
            HostConfig {
                max_registrations: 1,
                max_generations_per_capability: 1,
                max_concurrent_per_generation: 1,
                observation_stale_after_ms: 60_000,
            },
            CapabilitySelectionPolicy::priorities(BTreeMap::new()),
        )?;
        let (entered, observed) = sync_channel(1);
        let (release, released) = sync_channel(1);
        host.register(
            descriptor()?,
            Arc::new(ShutdownProbe {
                entered,
                release: Mutex::new(released),
                fail: true,
            }),
            None,
        )?;
        release.send(())?;
        let mut cleanup = StartupCleanup {
            host: Some(host.clone()),
            service: None,
            effects: None,
            deadline,
        };
        let error = cleanup
            .finish()
            .err()
            .ok_or("startup cleanup failure lost")?;
        assert!(error.contains("startup cleanup unconfirmed"));
        if deadline.is_zero() {
            assert!(error.contains("adapter shutdown remains unconfirmed"));
            assert!(host.shutdown().is_err());
        } else {
            assert!(error.contains("injected adapter shutdown failure"));
        }
        observed.recv_timeout(Duration::from_secs(5))?;
    }
    Ok(())
}

#[tokio::test]
async fn last_caller_disconnect_keeps_owner_until_adapter_shutdown_finishes()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let token = root.path().join("token");
    fs::write(&token, "shutdown-probe-token")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&token, fs::Permissions::from_mode(0o600))?;
    }
    let host = DaemonHost::start(crate::host::tests::owner_test_config(
        root.path(),
        &token,
        32,
    )?)?;
    let (entered, observed) = sync_channel(1);
    let (release, released) = sync_channel(1);
    let descriptor = descriptor()?;
    host.dispatch(false, move |owner| {
        owner
            .capability_host
            .register(
                descriptor,
                Arc::new(ShutdownProbe {
                    entered,
                    release: Mutex::new(released),
                    fail: false,
                }),
                None,
            )
            .map_err(|error| super::super::read_model::invalid(&error.to_string()))?;
        Ok(())
    })
    .await
    .map_err(|error| error.message)?;
    let join = host
        .join
        .lock()
        .map_err(|_| "join state poisoned")?
        .take()
        .ok_or("owner handle absent")?;
    let health = host.health.clone();
    drop(host);
    observed.recv_timeout(Duration::from_secs(5))?;
    let waited_for_cleanup = !join.is_finished();
    release.send(())?;
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !join.is_finished() && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(join.is_finished(), "owner did not complete bounded cleanup");
    join.join().map_err(|_| "owner panicked")?;
    assert!(
        waited_for_cleanup,
        "owner returned before adapter cleanup was released"
    );
    assert_eq!(
        health.read().state,
        milkdrift_control_protocol::DaemonState::Stopped
    );
    Ok(())
}
