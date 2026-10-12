//! Live observations through the production daemon and independent control client.

use super::support::*;
use futures_util::StreamExt as _;
use milkdrift_authority::CapabilityAuthorityScopeBuilder;
use milkdrift_capability::SideEffectClass;
use milkdrift_control_protocol::{CapabilityRead, Observation, ObservationEnvelope};
use std::{collections::BTreeSet, pin::Pin};

async fn task_event(response: &mut reqwest::Response) -> TestResult<String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(6);
    let mut bytes = Vec::new();
    while !bytes.windows(2).any(|value| value == b"\n\n") {
        let chunk = tokio::time::timeout_at(deadline, response.chunk())
            .await??
            .ok_or("task stream closed before event")?;
        bytes.extend_from_slice(&chunk);
        if bytes.len() > 65536 {
            return Err("task event exceeded bound".into());
        }
    }
    Ok(String::from_utf8(bytes)?)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn task_stream_cursor_is_actor_bound_and_resynchronizes_after_restart() -> TestResult {
    let directory = TempDir::new()?;
    let profile = configured_process_profile(&directory)?;
    let plan = configuration_with_process_profiles(&directory, 32, vec![profile])?;
    let daemon = start(plan.clone(), CONTROLLER_TOKEN).await?;
    let raw = reqwest::Client::new();
    let mut response = raw
        .get(daemon.endpoint.join("v1/workflows/stream")?)
        .bearer_auth(CONTROLLER_TOKEN)
        .send()
        .await?;
    let first = task_event(&mut response).await?;
    assert!(first.contains("event: workflow_changed"), "{first}");
    let cursor = first
        .lines()
        .find_map(|line| line.strip_prefix("id: "))
        .ok_or("event cursor absent")?
        .to_owned();
    drop(response);
    let foreign = raw
        .get(daemon.endpoint.join("v1/workflows/stream")?)
        .header("Last-Event-ID", &cursor)
        .bearer_auth(OBSERVER_TOKEN)
        .send()
        .await?;
    assert!(foreign.status().is_client_error());
    drop(foreign);
    daemon.stop().await?;
    let restarted = start(plan, CONTROLLER_TOKEN).await?;
    let mut response = raw
        .get(restarted.endpoint.join("v1/workflows/stream")?)
        .header("Last-Event-ID", &cursor)
        .bearer_auth(CONTROLLER_TOKEN)
        .send()
        .await?;
    assert!(response.status().is_success());
    assert!(
        task_event(&mut response)
            .await?
            .contains("event: resync_required")
    );
    drop(response);
    let mut fresh = raw
        .get(restarted.endpoint.join("v1/workflows/stream")?)
        .bearer_auth(CONTROLLER_TOKEN)
        .send()
        .await?;
    assert!(
        task_event(&mut fresh)
            .await?
            .contains("event: workflow_changed")
    );
    write_secret(
        &directory.path().join("controller.token"),
        "revoked-controller-token",
    )?;
    assert!(task_event(&mut fresh).await?.contains("event: forbidden"));
    drop(fresh);
    assert!(
        client(&restarted.endpoint, CONTROLLER_TOKEN)?
            .task_session()
            .await
            .is_err()
    );
    restarted.stop().await
}

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
    let mut second: serde_json::Value = serde_json::from_slice(&fs::read(&profile)?)?;
    let body = second
        .get_mut("profile")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("profile missing")?;
    body.insert(
        "capability".into(),
        serde_json::json!("second-local-process"),
    );
    body.insert("profile_id".into(), serde_json::json!("second-profile"));
    let second_path = directory.path().join("second-process.json");
    fs::write(&second_path, serde_json::to_vec(&second)?)?;
    let mut config =
        configuration_document_with_process_profiles(directory, 64, vec![profile, second_path])?;
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
