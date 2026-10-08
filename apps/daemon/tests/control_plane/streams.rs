//! Live observations through the production daemon and independent control client.

use super::support::*;
use milkdrift_control_protocol::{CapabilityRead, ObservationEnvelope};
use std::{collections::BTreeSet, pin::Pin};

type Subscription =
    Pin<Box<dyn futures_util::Stream<Item = Result<ObservationEnvelope, ClientError>> + Send>>;

async fn next(stream: &mut Subscription) -> TestResult<ObservationEnvelope> {
    tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await?
        .ok_or("subscription ended")?
        .map_err(Into::into)
}

fn catalogue(observation: ObservationEnvelope) -> TestResult<Vec<CapabilityRead>> {
    match observation.observation {
        Observation::CapabilitySnapshot(values) => Ok(values),
        other => Err(format!("expected capability, got {other:?}").into()),
    }
}

fn configuration_with_scoped_observer(directory: &TempDir) -> TestResult<DaemonConfig> {
    let profile = configured_process_profile(directory)?;
    let mut config = configuration_document_with_process_profiles(directory, 64, vec![profile])?;
    config
        .actors
        .get_mut(1)
        .ok_or("observer absent")?
        .authority
        .resources
        .capability = CapabilityAuthorityScopeBuilder::new(SideEffectClass::ReadOnly)
        .only_capabilities(BTreeSet::from([CapabilityId::new("golden-local-process")?]))?
        .only_operations(BTreeSet::from([OperationId::new("process.execute")?]))?
        .build();
    Ok(config)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn capability_streams_isolate_authority_in_both_subscription_orders() -> TestResult {
    for broad_first in [true, false] {
        let directory = tempfile::tempdir()?;
        let config = configuration_with_scoped_observer(&directory)?;
        let daemon = start(config.validate(directory.path())?, CONTROLLER_TOKEN).await?;
        let narrow = client(&daemon.endpoint, OBSERVER_TOKEN)?;
        let broad_catalogue = daemon.client.capabilities().await?;
        let narrow_catalogue = narrow.capabilities().await?;
        assert_eq!(broad_catalogue.len(), 2);
        assert_eq!(narrow_catalogue.len(), 1);
        assert_eq!(
            narrow_catalogue
                .first()
                .ok_or("narrow catalogue empty")?
                .capability_id,
            "golden-local-process"
        );
        let mut broad_stream = daemon.client.subscribe("v1/stream/capabilities", None);
        let mut narrow_stream = narrow.subscribe("v1/stream/capabilities", None);
        let (broad_first_event, narrow_first_event) = if broad_first {
            (
                next(&mut broad_stream).await?,
                next(&mut narrow_stream).await?,
            )
        } else {
            let narrow_event = next(&mut narrow_stream).await?;
            (next(&mut broad_stream).await?, narrow_event)
        };
        let broad_cursor = broad_first_event.cursor.clone();
        let narrow_cursor = narrow_first_event.cursor.clone();
        assert_eq!(catalogue(narrow_first_event)?, narrow_catalogue);
        let broad_values = catalogue(broad_first_event)?;
        assert_eq!(broad_values, broad_catalogue);
        assert!(
            tokio::time::timeout(Duration::from_millis(650), narrow_stream.next())
                .await
                .is_err(),
            "narrow subscriber received another authority's cached capability"
        );
        drop(broad_stream);
        drop(narrow_stream);
        let mut broad_resumed = daemon
            .client
            .subscribe("v1/stream/capabilities", Some(broad_cursor.clone()));
        let mut narrow_resumed =
            narrow.subscribe("v1/stream/capabilities", Some(narrow_cursor.clone()));
        assert!(
            tokio::time::timeout(Duration::from_millis(650), broad_resumed.next())
                .await
                .is_err()
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(650), narrow_resumed.next())
                .await
                .is_err()
        );
        let mut cross_authority = narrow.subscribe("v1/stream/capabilities", Some(broad_cursor));
        assert!(matches!(
            next(&mut cross_authority).await?.observation,
            Observation::ResyncRequired { .. }
        ));
        write_secret(
            &directory.path().join("observer.token"),
            "rotated-observer-token",
        )?;
        assert!(matches!(
            next(&mut narrow_resumed).await?.observation,
            Observation::StreamClosing { .. }
        ));
        let rotated = client(&daemon.endpoint, "rotated-observer-token")?;
        let mut old_cursor = rotated.subscribe("v1/stream/capabilities", Some(narrow_cursor));
        assert!(matches!(
            next(&mut old_cursor).await?.observation,
            Observation::ResyncRequired { .. }
        ));
        let mut fresh = rotated.subscribe("v1/stream/capabilities", None);
        assert_eq!(catalogue(next(&mut fresh).await?)?, narrow_catalogue);
        drop((
            broad_resumed,
            narrow_resumed,
            cross_authority,
            old_cursor,
            fresh,
        ));
        daemon.stop().await?;
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn capability_stream_closes_when_credential_moves_to_another_actor() -> TestResult {
    let directory = tempfile::tempdir()?;
    let config = configuration_with_scoped_observer(&directory)?;
    let daemon = start(config.validate(directory.path())?, CONTROLLER_TOKEN).await?;
    let narrow = client(&daemon.endpoint, OBSERVER_TOKEN)?;
    let mut stream = narrow.subscribe("v1/stream/capabilities", None);
    assert_eq!(
        catalogue(next(&mut stream).await?)?
            .first()
            .ok_or("snapshot empty")?
            .capability_id,
        "golden-local-process"
    );
    write_secret(
        &directory.path().join("observer.token"),
        "revoked-observer-token",
    )?;
    write_secret(&directory.path().join("controller.token"), OBSERVER_TOKEN)?;
    assert!(matches!(
        next(&mut stream).await?.observation,
        Observation::StreamClosing { .. }
    ));
    drop(stream);
    daemon.stop().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn process_local_stream_cursors_resynchronize_after_restart_but_run_cursor_resumes()
-> TestResult {
    let directory = tempfile::tempdir()?;
    let config = configuration_with_scoped_observer(&directory)?.validate(directory.path())?;
    let daemon = start(config.clone(), CONTROLLER_TOKEN).await?;
    let revision = import_blueprint(&daemon.client, "restart-stream-import").await?;
    daemon
        .client
        .submit(&request(
            "restart-stream-start",
            None,
            Command::StartRun {
                inputs: Vec::new(),
                run_id: "restart-stream".into(),
                workflow_id: "golden".into(),
                revision_id: revision,
            },
        ))
        .await?;
    let mut run = daemon
        .client
        .subscribe("v1/runs/restart-stream/stream", None);
    let run_cursor = next(&mut run).await?.cursor;
    // Queue inspection moves health well beyond a newly opened daemon's generation.
    for _ in 0..32 {
        daemon.client.health().await?;
    }
    let mut health = daemon.client.subscribe("v1/stream/health", None);
    let health_cursor = next(&mut health).await?.cursor;
    let mut capabilities = daemon.client.subscribe("v1/stream/capabilities", None);
    let capability_cursor = next(&mut capabilities).await?.cursor;
    drop((run, health, capabilities));
    daemon.stop().await?;

    let daemon = start(config, CONTROLLER_TOKEN).await?;
    let mut initial_health = daemon.client.subscribe("v1/stream/health", None);
    let initial_health_cursor = next(&mut initial_health).await?.cursor;
    assert!(
        health_cursor.position_for("daemon-health")?
            > initial_health_cursor.position_for("daemon-health")?
    );
    drop(initial_health);
    let mut fresh_capabilities = daemon.client.subscribe("v1/stream/capabilities", None);
    let fresh = next(&mut fresh_capabilities).await?;
    // Deliberately reuse the same numeric position in the new daemon's populated window.
    assert_eq!(
        fresh.cursor.position_for("capability-health")?,
        capability_cursor.position_for("capability-health")?
    );
    assert_eq!(catalogue(fresh)?, daemon.client.capabilities().await?);
    for (path, cursor) in [
        ("v1/stream/health", health_cursor),
        ("v1/stream/capabilities", capability_cursor),
    ] {
        let mut resumed = daemon.client.subscribe(path, Some(cursor));
        assert!(
            matches!(
                next(&mut resumed).await?.observation,
                Observation::ResyncRequired { .. }
            ),
            "{path} silently accepted an earlier daemon's position"
        );
        assert!(resumed.next().await.is_none());
    }
    let mut fresh_health = daemon.client.subscribe("v1/stream/health", None);
    assert!(matches!(
        next(&mut fresh_health).await?.observation,
        Observation::DaemonHealth(_)
    ));
    daemon
        .client
        .submit(&request(
            "restart-stream-pause",
            Some(daemon.client.run("restart-stream").await?.sequence),
            Command::PauseRun {
                run_id: "restart-stream".into(),
            },
        ))
        .await?;
    let old_position = run_cursor.position_for("run:restart-stream")?;
    let mut resumed_run = daemon
        .client
        .subscribe("v1/runs/restart-stream/stream", Some(run_cursor));
    let continued = next(&mut resumed_run).await?;
    assert!(matches!(continued.observation, Observation::Timeline(_)));
    assert!(continued.cursor.position_for("run:restart-stream")? > old_position);
    drop((fresh_capabilities, fresh_health, resumed_run));
    daemon.stop().await
}
