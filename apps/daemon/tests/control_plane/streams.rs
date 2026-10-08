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

fn capability(observation: ObservationEnvelope) -> TestResult<CapabilityRead> {
    match observation.observation {
        Observation::Capability(value) => Ok(value),
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
        assert_eq!(
            capability(narrow_first_event)?,
            narrow_catalogue.first().ok_or("narrow absent")?.clone()
        );
        let broad_values = vec![
            capability(broad_first_event)?,
            capability(next(&mut broad_stream).await?)?,
        ];
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
        assert_eq!(
            capability(next(&mut broad_resumed).await?)?,
            broad_catalogue
                .get(1)
                .ok_or("second capability absent")?
                .clone()
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(650), narrow_resumed.next())
                .await
                .is_err()
        );
        let mut cross_authority = narrow.subscribe("v1/stream/capabilities", Some(broad_cursor));
        assert!(
            matches!(tokio::time::timeout(Duration::from_secs(5), cross_authority.next()).await?.ok_or("missing refusal")?, Err(ClientError::Api(error)) if error.code == ErrorCode::InvalidInput)
        );
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
        assert!(
            matches!(tokio::time::timeout(Duration::from_secs(5), old_cursor.next()).await?.ok_or("missing rotated refusal")?, Err(ClientError::Api(error)) if error.code == ErrorCode::InvalidInput)
        );
        let mut fresh = rotated.subscribe("v1/stream/capabilities", None);
        assert_eq!(
            capability(next(&mut fresh).await?)?.capability_id,
            "golden-local-process"
        );
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
        capability(next(&mut stream).await?)?.capability_id,
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
