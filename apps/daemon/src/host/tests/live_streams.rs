//! Drive real registry lifecycle transitions and observe them only through the public client.

use super::*;
use futures_util::StreamExt as _;
use milkdrift_capability::CapabilityId;
use milkdrift_control_client::{BearerCredential, ClientConfig, ClientError, ControlClient};
use milkdrift_control_protocol::{CapabilityRead, Cursor, Observation, ObservationEnvelope};
use std::{pin::Pin, time::Duration};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
type Subscription =
    Pin<Box<dyn futures_util::Stream<Item = Result<ObservationEnvelope, ClientError>> + Send>>;

async fn converge(
    stream: &mut Subscription,
    view: &mut Vec<CapabilityRead>,
    expected: &[CapabilityRead],
) -> TestResult<Cursor> {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let event = stream.next().await.ok_or("capability stream ended")??;
            match event.observation {
                Observation::CapabilitySnapshot(values) => *view = values,
                other => return Err(format!("unexpected observation: {other:?}").into()),
            }
            if view == expected {
                return Ok(event.cursor);
            }
        }
    })
    .await
    .map_err(|_| "stream failed to remove the retired generation from its current view")?
}

async fn drain(
    host: &DaemonHost,
    capability: &'static str,
    generation: u64,
    finish: bool,
) -> TestResult {
    host.dispatch(false, move |owner| {
        let capability = CapabilityId::new(capability)
            .map_err(|error| crate::host::invalid(&error.to_string()))?;
        if finish {
            owner.capability_host.finish_drain(&capability, generation)
        } else {
            owner.capability_host.begin_drain(&capability, generation)
        }
        .map_err(|error| crate::host::invalid(&error.to_string()))
    })
    .await
    .map_err(|error| error.message.into())
}

async fn register_replacement(host: &DaemonHost, root: &std::path::Path) -> TestResult {
    let mut profile: serde_json::Value = serde_json::from_slice(
        &super::unavailable_models::profile("127.0.0.1:9".parse()?)?.to_canonical_json()?,
    )?;
    *profile
        .get_mut("revision")
        .ok_or("profile revision absent")? = serde_json::json!(2);
    let path = root.join("replacement.json");
    fs::write(&path, serde_json::to_vec(&profile)?)?;
    let artifacts = root.join("data/execution");
    let secrets = host.auth.resolver();
    host.dispatch(false, move |owner| {
        let invalid = |error: String| crate::host::invalid(&error);
        let data = milkdrift_capability_host::StoreInvocationDataAccess::new(
            owner.store.clone(),
            artifacts,
            milkdrift_persistence::ArtifactReadAuthority::Authorized {
                actor: milkdrift_authority::ActorRef::new("service:daemon-runtime")
                    .map_err(|error| invalid(error.to_string()))?,
                evidence: milkdrift_persistence::EvidenceId::new("daemon-materialization")
                    .map_err(|error| invalid(error.to_string()))?,
            },
        )
        .map_err(|error| invalid(error.to_string()))?;
        crate::host::capabilities::register_configured(
            &crate::AdapterConfig {
                model_profiles: vec![crate::ModelProfileConfig {
                    capability_id: "model:retained".into(),
                    profile: path,
                }],
                ..crate::AdapterConfig::default()
            },
            &owner.capability_host,
            Arc::new(data),
            secrets,
            owner.now()?,
        )
        .map_err(invalid)
    })
    .await
    .map_err(|error| error.message.into())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn capability_stream_removal_converges_with_fresh_and_reconnected_clients() -> TestResult {
    let root = tempfile::tempdir()?;
    let token = root.path().join("token");
    fs::write(&token, "catalogue-lifecycle-token")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&token, fs::Permissions::from_mode(0o600))?;
    }
    let profile = root.path().join("model.json");
    fs::write(
        &profile,
        super::unavailable_models::profile("127.0.0.1:9".parse()?)?.to_canonical_json()?,
    )?;
    let mut config = owner_test_document(root.path(), &token, 32, 10);
    let narrow_token = root.path().join("narrow.token");
    fs::write(&narrow_token, "catalogue-narrow-token")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&narrow_token, fs::Permissions::from_mode(0o600))?;
    }
    config.secret_sources.insert(
        "credential:narrow".into(),
        SecretSourceConfig::File { path: narrow_token },
    );
    let mut narrow_actor = config.actors.first().ok_or("actor absent")?.clone();
    narrow_actor.actor = "human:narrow".into();
    narrow_actor.grant_id = "grant:narrow".into();
    narrow_actor.credential_ref = "credential:narrow".into();
    narrow_actor.authority.resources.capability =
        milkdrift_authority::CapabilityAuthorityScopeBuilder::new(
            milkdrift_capability::SideEffectClass::Unknown,
        )
        .only_capabilities(std::collections::BTreeSet::from([CapabilityId::new(
            "model:retained",
        )?]))?
        .build();
    config.actors.push(narrow_actor);
    config
        .adapters
        .model_profiles
        .push(crate::ModelProfileConfig {
            capability_id: "model:retained".into(),
            profile,
        });
    let host = DaemonHost::start(config.validate(root.path())?)?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let endpoint = url::Url::parse(&format!("http://{}/", listener.local_addr()?))?;
    let (shutdown, stop) = tokio::sync::oneshot::channel();
    let serving_host = host.clone();
    let serving = tokio::spawn(crate::serve(listener, serving_host, async {
        let _closed = stop.await;
    }));
    let client = ControlClient::new(
        ClientConfig::new(endpoint.clone()),
        BearerCredential::new("catalogue-lifecycle-token")?,
    )?;
    let narrow = ControlClient::new(
        ClientConfig::new(endpoint),
        BearerCredential::new("catalogue-narrow-token")?,
    )?;
    let work: TestResult = async {
        let initial = client.capabilities().await?;
        assert_eq!(initial.len(), 2);
        let mut live = client.subscribe("v1/stream/capabilities", None);
        let mut view = Vec::new();
        converge(&mut live, &mut view, &initial).await?;
        let mut narrow_live = narrow.subscribe("v1/stream/capabilities", None);
        let mut narrow_view = Vec::new();
        let narrow_initial = narrow.capabilities().await?;
        assert_eq!(narrow_initial.len(), 1);
        let narrow_cursor = converge(&mut narrow_live, &mut narrow_view, &narrow_initial).await?;
        let control_generation = initial
            .iter()
            .find(|value| value.capability_id == "milkdrift-workflow-control")
            .ok_or("control absent")?
            .generation;
        drain(
            &host,
            "milkdrift-workflow-control",
            control_generation,
            false,
        )
        .await?;
        let draining = client.capabilities().await?;
        assert_eq!(draining.len(), 2);
        let draining_control = draining
            .iter()
            .find(|value| value.capability_id == "milkdrift-workflow-control")
            .ok_or("control absent")?;
        assert!(draining_control.draining);
        assert!(!draining_control.current);
        let before_removal = converge(&mut live, &mut view, &draining).await?;
        drain(
            &host,
            "milkdrift-workflow-control",
            control_generation,
            true,
        )
        .await?;
        let current = client.capabilities().await?;
        assert_eq!(current.len(), 1);
        assert_eq!(
            current.first().ok_or("model absent")?.capability_id,
            "model:retained"
        );
        let after_removal = converge(&mut live, &mut view, &current).await?;
        assert_eq!(narrow.capabilities().await?, narrow_initial);
        let mut narrow_resumed = narrow.subscribe("v1/stream/capabilities", Some(narrow_cursor));
        assert!(
            tokio::time::timeout(Duration::from_millis(650), narrow_resumed.next())
                .await
                .is_err(),
            "hidden retirement must not disclose its identity or alter the narrow snapshot"
        );
        let mut resumed = client.subscribe("v1/stream/capabilities", Some(before_removal));
        let mut resumed_view = draining;
        converge(&mut resumed, &mut resumed_view, &current).await?;
        let mut fresh = client.subscribe("v1/stream/capabilities", None);
        converge(&mut fresh, &mut Vec::new(), &current).await?;
        let mut at_head = client.subscribe("v1/stream/capabilities", Some(after_removal));
        assert!(
            tokio::time::timeout(Duration::from_millis(650), at_head.next())
                .await
                .is_err()
        );
        register_replacement(&host, root.path()).await?;
        let replaced = client.capabilities().await?;
        assert_eq!(replaced.len(), 2);
        assert!(!replaced.first().ok_or("old generation absent")?.current);
        assert!(replaced.get(1).ok_or("new generation absent")?.current);
        assert_eq!(
            replaced.get(1).ok_or("new generation absent")?.generation,
            2
        );
        converge(&mut live, &mut view, &replaced).await?;
        converge(&mut narrow_live, &mut narrow_view, &replaced).await?;
        drain(&host, "model:retained", 1, false).await?;
        drain(&host, "model:retained", 1, true).await?;
        let replacement_only = client.capabilities().await?;
        assert_eq!(replacement_only.len(), 1);
        assert_eq!(
            replacement_only
                .first()
                .ok_or("replacement absent")?
                .generation,
            2
        );
        converge(&mut live, &mut view, &replacement_only).await?;
        drain(&host, "model:retained", 2, false).await?;
        drain(&host, "model:retained", 2, true).await?;
        converge(&mut live, &mut view, &[]).await?;
        let mut empty = client.subscribe("v1/stream/capabilities", None);
        converge(&mut empty, &mut Vec::new(), &[]).await?;
        Ok(())
    }
    .await;
    shutdown.send(()).map_err(|()| "shutdown receiver gone")?;
    let stopped = serving.await?;
    work?;
    stopped?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn capability_cursor_cannot_attach_to_another_http_feed_on_the_same_host() -> TestResult {
    let root = tempfile::tempdir()?;
    let token = root.path().join("token");
    fs::write(&token, "http-feed-token")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&token, fs::Permissions::from_mode(0o600))?;
    }
    let host = DaemonHost::start(owner_test_config(root.path(), &token, 32)?)?;
    let mut servers = Vec::new();
    let mut clients = Vec::new();
    for _ in 0..2 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let endpoint = url::Url::parse(&format!("http://{}/", listener.local_addr()?))?;
        let (shutdown, stop) = tokio::sync::oneshot::channel();
        let router = crate::http::router(host.clone())?;
        let serving = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async {
                    let _closed = stop.await;
                })
                .await
        });
        servers.push((shutdown, serving));
        clients.push(ControlClient::new(
            ClientConfig::new(endpoint),
            BearerCredential::new("http-feed-token")?,
        )?);
    }
    let result: TestResult = async {
        let mut first = clients
            .first()
            .ok_or("first server absent")?
            .subscribe("v1/stream/capabilities", None);
        let old = tokio::time::timeout(Duration::from_secs(5), first.next())
            .await?
            .ok_or("first observation absent")??;
        let second_client = clients.get(1).ok_or("second server absent")?;
        let mut second = second_client.subscribe("v1/stream/capabilities", None);
        let new = tokio::time::timeout(Duration::from_secs(5), second.next())
            .await?
            .ok_or("second observation absent")??;
        assert_eq!(
            old.cursor.position_for("capability-health")?,
            new.cursor.position_for("capability-health")?
        );
        let mut resumed = second_client.subscribe("v1/stream/capabilities", Some(old.cursor));
        let event = tokio::time::timeout(Duration::from_secs(2), resumed.next())
            .await
            .map_err(|_| "cursor silently attached to a different retained feed")?
            .ok_or("missing resync")??;
        assert!(matches!(
            event.observation,
            Observation::ResyncRequired { .. }
        ));
        Ok(())
    }
    .await;
    let mut tasks = Vec::new();
    for (shutdown, serving) in servers {
        shutdown
            .send(())
            .map_err(|()| "server shutdown receiver lost")?;
        tasks.push(serving);
    }
    for task in tasks {
        task.await??;
    }
    host.shutdown().await?;
    result
}
