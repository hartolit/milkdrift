use std::{
    collections::BTreeMap,
    fs,
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use super::clock::{DaemonClockError, DaemonClockSource};
use super::{DaemonHost, read_model::timeline_summary};
use crate::config::{
    ActorBindingConfig, ActorGrantConfig, AdapterConfig, ApplicationReceiptConfig,
    AuthorityPresetConfig, ConfigError, DaemonConfig, PeerHostConfig, RuntimeHostConfig,
    SecretSourceConfig, ShutdownConfig,
};
use milkdrift_control_protocol::TimelineCategory;

struct ControlledDaemonClock(AtomicU64);

impl ControlledDaemonClock {
    const fn new(now: u64) -> Self {
        Self(AtomicU64::new(now))
    }

    fn set(&self, now: u64) {
        self.0.store(now, Ordering::SeqCst);
    }
}

impl DaemonClockSource for ControlledDaemonClock {
    fn now_unix_ms(&self) -> Result<u64, DaemonClockError> {
        Ok(self.0.load(Ordering::SeqCst))
    }
}

fn clock_test_config(
    root: &std::path::Path,
    token: &std::path::Path,
) -> Result<crate::DaemonPlan, ConfigError> {
    DaemonConfig {
        schema_version: crate::DAEMON_CONFIG_SCHEMA_VERSION,
        data_root: root.join("data"),
        bind: SocketAddr::from(([127, 0, 0, 1], 0)),
        secret_sources: BTreeMap::from([(
            "credential:operator".to_owned(),
            SecretSourceConfig::File {
                path: token.to_path_buf(),
            },
        )]),
        actors: vec![ActorBindingConfig {
            credential_ref: "credential:operator".to_owned(),
            actor: "human:clock-operator".to_owned(),
            grant_id: "grant:clock-operator".to_owned(),
            grant_revision: 1,
            revocation_generation: 0,
            preset: AuthorityPresetConfig::Controller,
            authority: ActorGrantConfig::dangerous_administrator(),
            enabled: true,
        }],
        runtime: RuntimeHostConfig {
            request_queue: 1,
            ..RuntimeHostConfig::default()
        },
        adapters: AdapterConfig::default(),
        peers: PeerHostConfig::default(),
        shutdown: ShutdownConfig::default(),
        application_receipts: ApplicationReceiptConfig {
            hot_receipt_bound: 100,
            archive_batch_size: 10,
        },
        security_audit_record_bound: 100,
    }
    .validate(root)
}

#[test]
fn timeline_projection_never_serializes_internal_event_body() {
    assert_eq!(
        timeline_summary(TimelineCategory::Execution),
        "node execution changed"
    );
    assert!(!timeline_summary(TimelineCategory::Execution).contains("NodeScheduled"));
}

#[tokio::test]
async fn daemon_restart_rejects_clock_rollback_before_readiness()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let token = root.path().join("operator.token");
    fs::write(&token, "clock-test-token")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&token, fs::Permissions::from_mode(0o600))?;
    }
    let clock = Arc::new(ControlledDaemonClock::new(100));
    let host =
        DaemonHost::start_with_clock(clock_test_config(root.path(), &token)?, clock.clone())?;
    clock.set(120);
    assert_eq!(host.now().await.map_err(|error| error.message)?, 120);
    host.shutdown().await?;
    clock.set(119);
    assert!(
        DaemonHost::start_with_clock(clock_test_config(root.path(), &token)?, clock.clone(),)
            .is_err()
    );

    clock.set(120);
    let recovered = DaemonHost::start_with_clock(clock_test_config(root.path(), &token)?, clock)?;
    recovered.shutdown().await?;
    Ok(())
}

fn queue_test_host() -> Result<(tempfile::TempDir, DaemonHost), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let token = root.path().join("operator.token");
    fs::write(&token, "queue-test-token")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&token, fs::Permissions::from_mode(0o600))?;
    }
    let host = DaemonHost::start_with_clock(
        clock_test_config(root.path(), &token)?,
        Arc::new(ControlledDaemonClock::new(100)),
    )?;
    Ok((root, host))
}

#[tokio::test]
async fn owner_queue_overload_and_dropped_reply_release_occupancy()
-> Result<(), Box<dyn std::error::Error>> {
    use super::queue::OwnerRequest;
    use milkdrift_control_protocol::ErrorCode;
    use std::sync::mpsc::sync_channel;
    use std::time::Duration;
    use tokio::sync::oneshot;

    let (_root, host) = queue_test_host()?;
    let (entered, entry) = oneshot::channel();
    let (release, released) = sync_channel(1);
    let (reply, abandoned) = oneshot::channel();
    drop(abandoned);
    let mut request = OwnerRequest {
        execute: Box::new(move |_| {
            let _ = entered.send(());
            let outcome = released.recv_timeout(Duration::from_secs(5));
            let _ = reply.send(outcome);
        }),
        stop_owner: false,
        queued: None,
    };
    request.mark_queued(&host.health);
    host.sender
        .try_send(request)
        .map_err(|_| "blocking request refused")?;
    tokio::time::timeout(Duration::from_secs(5), entry).await??;
    assert_eq!(host.health().queued_requests, 0);

    let (reply, response) = oneshot::channel();
    let mut queued = OwnerRequest {
        execute: Box::new(move |owner| {
            let _ = reply.send(owner.now());
        }),
        stop_owner: false,
        queued: None,
    };
    queued.mark_queued(&host.health);
    host.sender
        .try_send(queued)
        .map_err(|_| "queued request refused")?;
    assert_eq!(host.health().queued_requests, 1);
    let refused = host.dispatch(false, |_| Ok(())).await;
    assert!(matches!(refused, Err(error) if error.code == ErrorCode::Overload));
    assert_eq!(host.health().queued_requests, 1);
    release.send(())?;
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), response)
            .await??
            .map_err(|error| error.message)?,
        100
    );
    assert_eq!(host.health().queued_requests, 0);
    assert!(host.dispatch(false, |_| Ok(())).await.is_ok());
    host.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn owner_request_panic_closes_admission_but_preserves_final_clock_and_shutdown_calls()
-> Result<(), Box<dyn std::error::Error>> {
    use milkdrift_control_protocol::{DaemonState, ErrorCode};
    let (root, host) = queue_test_host()?;
    let result = host
        .dispatch(false, |_| -> Result<(), super::PublicFailure> {
            std::panic::resume_unwind(Box::new("injected owner request panic"));
        })
        .await;
    assert!(matches!(result, Err(error) if error.code == ErrorCode::Unavailable));
    // A draining call is ordered after panic classification on the owner thread.
    assert_eq!(
        host.dispatch_draining(|owner| owner.now())
            .await
            .map_err(|error| error.message)?,
        100
    );
    assert_eq!(host.health().state, DaemonState::Failed);
    assert_eq!(host.health().queued_requests, 0);
    assert!(
        matches!(host.dispatch(false, |_| Ok(())).await, Err(error) if error.code == ErrorCode::Unavailable)
    );
    assert_eq!(host.now().await.map_err(|error| error.message)?, 100);
    assert!(host.shutdown().await.is_err());
    // Failed shutdown still joins and relinquishes durable ownership.
    drop(host);
    let reopened = DaemonHost::start_with_clock(
        clock_test_config(root.path(), &root.path().join("operator.token"))?,
        Arc::new(ControlledDaemonClock::new(100)),
    )?;
    reopened.shutdown().await?;
    Ok(())
}
