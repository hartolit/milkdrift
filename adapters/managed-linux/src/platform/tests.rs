use super::{start_owned, verify_subordinate_ids};
use crate::{InferenceBackend, LinuxRecipe, ModelService, recipe::Deployment, units};
use milkdrift_capability::{BoundedJson, managed::ManagedName};
use milkdrift_persistence::managed::ApprovedSetup;
use std::cell::Cell;

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn owned_setup() -> Result<ApprovedSetup> {
    let mut recipe = LinuxRecipe::from_json(include_bytes!(
        "../../../../examples/managed-linux/slotbook.json"
    ))?;
    recipe.model_service = ModelService::Owned {
        image: format!("sha256:{}", "a".repeat(64)),
        executable: "/usr/bin/llama-server".to_owned(),
        model: "/models/approved.gguf".into(),
        model_digest: format!("b3_{}", "b".repeat(64)),
        model_bytes: 1024,
        port: 8080,
        context_tokens: 4096,
        token_limits: milkdrift_model_provider::ModelTokenLimits::ByteBpe {
            template_tokens_per_message: 8,
            template_tokens_per_request: 8,
            maximum_input_tokens: 4096,
            maximum_output_tokens: 128,
            output_control: milkdrift_model_provider::OutputTokenControl::MaxTokens,
            source: "deterministic unit-generation fixture".to_owned(),
        },
        threads: 1,
        backend: InferenceBackend::Cpu {},
    };
    let ownership = format!("b3_{}", "c".repeat(64));
    let prefix = units::volume_prefix("fixture", &ownership);
    let reference = recipe.reference()?;
    let deployment = Deployment {
        recipe,
        installation: ManagedName::new("slotbook")?,
        generation: 1,
        unit: format!("{prefix}g1"),
        volume_prefix: prefix,
        manager_root: "/manager".into(),
        quadlet_directory: "/quadlets".into(),
    };
    Ok(ApprovedSetup {
        recipe: reference,
        mechanism: "linux-quadlet-v1".to_owned(),
        configuration: BoundedJson::new(serde_json::to_value(deployment)?)?,
        platform_owner: "fixture".to_owned(),
        ownership,
        resources: Vec::new(),
        capabilities: Vec::new(),
    })
}

#[test]
fn service_name_collision_cannot_reach_the_supervisor() -> Result {
    let setup = owned_setup()?;
    let starts = Cell::new(0);
    let owned = serde_json::json!({"Config":{"Labels":{
        "org.milkdrift.owner": setup.ownership,
        "org.milkdrift.platform": setup.platform_owner,
        "org.milkdrift.recipe": setup.recipe.digest
    }}});
    for field in [
        "org.milkdrift.owner",
        "org.milkdrift.platform",
        "org.milkdrift.recipe",
    ] {
        let mut foreign = owned.clone();
        foreign["Config"]["Labels"][field] = serde_json::json!("foreign");
        assert!(
            start_owned(&setup, Some(&foreign), || {
                starts.set(starts.get() + 1);
                Ok(())
            })
            .is_err()
        );
    }
    assert_eq!(starts.get(), 0);
    for container in [None, Some(&owned)] {
        start_owned(&setup, container, || {
            starts.set(starts.get() + 1);
            Ok(())
        })?;
    }
    assert_eq!(starts.get(), 2);
    // The engine must also refuse a container appearing after the identity check.
    let definition = units::unit_text(&setup, true)?.ok_or("owned unit missing")?;
    assert!(definition.contains("PodmanArgs=--replace=false "));
    Ok(())
}

#[test]
fn owned_model_leaves_a_private_user_namespace_for_the_worker() -> Result {
    let setup = owned_setup()?;
    let deployment = units::deployment(&setup)?;
    let mut info = serde_json::json!({"host":{"idMappings":{
        "uidmap":[{"container_id":1,"host_id":100000,"size":65536}],
        "gidmap":[{"container_id":1,"host_id":100000,"size":65536}]
    }}});
    verify_subordinate_ids(&info, &ModelService::Disabled {})?;
    assert!(verify_subordinate_ids(&info, &deployment.recipe.model_service).is_err());
    info["host"]["idMappings"]["uidmap"][0]["size"] = serde_json::json!(131072);
    assert!(verify_subordinate_ids(&info, &deployment.recipe.model_service).is_err());
    info["host"]["idMappings"]["gidmap"][0]["size"] = serde_json::json!(131072);
    verify_subordinate_ids(&info, &deployment.recipe.model_service)?;
    Ok(())
}
