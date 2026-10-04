//! Preparation uses the serving owner in an execution-only host and performs no external work.
use super::support::*;
use milkdrift_control_protocol::HostRole;
use milkdrift_peer_protocol::{DirectInvocationDraft, InvocationAcceptance, InvocationLookup};
use serde_json::json;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn preparation_derives_selection_and_refuses_invalid_choices_without_entry() -> TestResult {
    for (effect, key) in [
        ("idempotent_write", Some("prepared-process")),
        ("read_only", None),
        ("non_idempotent_write", None),
    ] {
        preparation_case(effect, key).await?;
    }
    Ok(())
}

async fn preparation_case(effect: &str, expected_key: Option<&str>) -> TestResult {
    let directory = TempDir::new()?;
    let profile_path = configured_process_profile(&directory)?;
    let mut profile: serde_json::Value = serde_json::from_slice(&fs::read(&profile_path)?)?;
    *profile
        .pointer_mut("/profile/side_effect")
        .ok_or("fixture field /profile/side_effect absent")? = json!(effect);
    if expected_key.is_some() {
        *profile
            .pointer_mut("/profile/idempotency")
            .ok_or("fixture field /profile/idempotency absent")? = json!("capability_scoped");
        *profile
            .pointer_mut("/profile/arguments")
            .ok_or("fixture field /profile/arguments absent")? = json!(["echo", "{{key}}"]);
        *profile
            .pointer_mut("/profile/substitutions")
            .ok_or("fixture field /profile/substitutions absent")? =
            json!({"key":{"type":"idempotency_key"}});
    }
    fs::write(&profile_path, serde_json::to_vec(&profile)?)?;
    let mut config =
        configuration_document_with_process_profiles(&directory, 32, vec![profile_path])?;
    config.role = HostRole::ExecutionOnly;
    let daemon = start(config.validate(directory.path())?, CONTROLLER_TOKEN).await?;
    let discovery = daemon.client.execution_discovery().await?;
    let entry = discovery.catalog.entries.first().ok_or("process catalog")?;
    let operation = entry
        .invocable_operations
        .first()
        .ok_or("process operation")?;
    let draft = DirectInvocationDraft {
        host: discovery.host.clone(),
        request_id: milkdrift_peer_protocol::PeerRequestId::new("prepared-process")?,
        capability: entry.descriptor.identity().clone(),
        operation: operation.clone(),
        inputs: vec![],
        limits: None,
    };
    let prepared = daemon.client.prepare_invocation(&draft).await?;
    assert_eq!(prepared.catalog_generation, discovery.catalog.generation);
    assert_eq!(prepared.catalog_digest, discovery.catalog.digest);
    assert_eq!(prepared.limits, discovery.limits);
    assert_eq!(
        prepared.selection.descriptor_revision(),
        entry.descriptor.descriptor_revision()
    );
    assert_eq!(
        prepared.request.provider_profile(),
        entry.descriptor.provider_profile()
    );
    assert_eq!(
        prepared.request.idempotency_key().map(|key| key.as_str()),
        expected_key
    );
    assert!(
        prepared.deadline_unix_ms
            >= discovery.catalog.issued_at_unix_ms + prepared.limits.duration_ms
    );
    assert!(matches!(
        daemon.client.invocation_lookup(&draft.request_id).await?,
        InvocationLookup::NotAccepted { .. }
    ));

    for mutation in [
        "host",
        "capability",
        "operation",
        "limits",
        "workspace",
        "duplicate",
    ] {
        let mut value = serde_json::to_value(&draft)?;
        match mutation {
            "host" => {
                *value
                    .pointer_mut("/host")
                    .ok_or("fixture field /host absent")? = json!("host:other")
            }
            "capability" => {
                *value
                    .pointer_mut("/capability")
                    .ok_or("fixture field /capability absent")? = json!("hidden-or-missing")
            }
            "operation" => {
                *value
                    .pointer_mut("/operation")
                    .ok_or("fixture field /operation absent")? = json!("missing.operation")
            }
            "limits" => {
                *value
                    .pointer_mut("/limits")
                    .ok_or("fixture field /limits absent")? =
                    serde_json::to_value(&discovery.limits)?;
                *value
                    .pointer_mut("/limits/artifact_bytes")
                    .ok_or("fixture field /limits/artifact_bytes absent")? =
                    json!(discovery.limits.artifact_bytes + 1);
            }
            "workspace" => {
                *value
                    .pointer_mut("/inputs")
                    .ok_or("fixture field /inputs absent")? = json!([{"name":"source","value":{"type":"workspace_value","identity":"private","version":"1"}}])
            }
            "duplicate" => {
                *value
                    .pointer_mut("/inputs")
                    .ok_or("fixture field /inputs absent")? = json!([
                    {"name":"source","value":{"type":"inline","value":"first"}},
                    {"name":"source","value":{"type":"inline","value":"second"}}
                ])
            }
            _ => return Err("unknown test mutation".into()),
        }
        let reply = reqwest::Client::new()
            .post(daemon.endpoint.join("v1/invocations/prepare")?)
            .bearer_auth(CONTROLLER_TOKEN)
            .json(&value)
            .send()
            .await?;
        assert!(
            reply.status().is_client_error(),
            "{mutation}: {}",
            reply.text().await?
        );
    }
    let observer = client(&daemon.endpoint, OBSERVER_TOKEN)?;
    assert!(observer.prepare_invocation(&draft).await.is_err());
    assert!(matches!(
        daemon.client.invocation_lookup(&draft.request_id).await?,
        InvocationLookup::NotAccepted { .. }
    ));
    let accepted = daemon.client.invoke(&prepared).await?;
    assert!(
        matches!(
            accepted,
            InvocationAcceptance::Accepted {
                replayed: false,
                ..
            }
        ),
        "{accepted:?}"
    );
    assert!(matches!(
        daemon.client.invoke(&prepared).await?,
        InvocationAcceptance::Accepted { replayed: true, .. }
    ));
    daemon.stop().await
}
