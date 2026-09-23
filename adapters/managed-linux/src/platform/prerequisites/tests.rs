use super::*;
use crate::units;
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
#[test]
fn byte_limits_cannot_silently_round_on_different_host_page_sizes() -> Result {
    let recipe = units::deployment(&super::super::tests::owned_setup()?)?.recipe;
    for page_size in [4096, 65536] {
        let mut limits = recipe.worker_limits.clone();
        limits.validate_page_alignment(page_size, "worker_limits")?;
        limits.temporary_bytes += 1;
        assert!(
            limits
                .validate_page_alignment(page_size, "worker_limits")
                .err()
                .ok_or("tmpfs rounding accepted")?
                .to_string()
                .contains("worker_limits.temporary_bytes")
        );
        limits.temporary_bytes -= 1;
        limits.memory_bytes -= 1;
        assert!(
            limits
                .validate_page_alignment(page_size, "model_service.limits")
                .err()
                .ok_or("memory rounding accepted")?
                .to_string()
                .contains("model_service.limits.memory_bytes")
        );
    }
    Ok(())
}

#[test]
fn current_podman_and_required_delegation_are_checked_before_effects() -> Result {
    for version in ["5.4.0", "5.9.2", "6.0.0", "6.1.2"] {
        verify_podman_version(version)?;
    }
    for version in ["4.9.0", "5.3.2", "6", "6.bad", "7.0.0"] {
        assert!(verify_podman_version(version).is_err(), "{version}");
    }
    verify_controllers(&serde_json::json!({"host":{"cgroupControllers":["cpu","memory","pids"]}}))?;
    for controllers in [
        serde_json::json!(["memory", "pids"]),
        serde_json::json!(["cpu", "pids"]),
        serde_json::json!(["cpu", "memory"]),
        serde_json::json!(null),
    ] {
        assert!(
            verify_controllers(&serde_json::json!({"host":{"cgroupControllers":controllers}}))
                .is_err()
        );
    }
    Ok(())
}

#[test]
fn owned_model_leaves_a_private_user_namespace_for_the_worker() -> Result {
    let mut info = serde_json::json!({"host":{"idMappings":{
        "uidmap":[{"container_id":1,"host_id":100000,"size":65536}],
        "gidmap":[{"container_id":1,"host_id":100000,"size":65536}]
    }}});
    verify_subordinate_ids(&info, 1)?;
    assert!(verify_subordinate_ids(&info, 2).is_err());
    info["host"]["idMappings"]["uidmap"][0]["size"] = serde_json::json!(131072);
    assert!(verify_subordinate_ids(&info, 2).is_err());
    info["host"]["idMappings"]["gidmap"][0]["size"] = serde_json::json!(131072);
    verify_subordinate_ids(&info, 2)?;
    for key in ["uidmap", "gidmap"] {
        info["host"]["idMappings"][key] = serde_json::json!([
            {"container_id":0,"host_id":1000,"size":1},
            {"container_id":1,"host_id":100000,"size":65536},
            {"container_id":65537,"host_id":165536,"size":65536}
        ]);
    }
    verify_subordinate_ids(&info, 2)?;
    Ok(())
}

#[test]
fn simultaneous_memory_and_existing_service_credit_have_one_scope() -> Result {
    let mut recipe = units::deployment(&super::super::tests::owned_setup()?)?.recipe;
    recipe.worker_limits.memory_bytes = 3;
    let ModelService::Owned { limits, .. } = &mut recipe.model_service else {
        return Err("owned fixture missing".into());
    };
    limits.memory_bytes = 7;
    assert!(verify_headroom(required_memory(&recipe, 0)?, 8).is_err());
    verify_headroom(required_memory(&recipe, 2)?, 8)?;
    assert_eq!(required_memory(&recipe, 2)?, 8);
    assert_eq!(required_memory(&recipe, 20)?, 0); // A larger old generation is stopped during update.
    recipe.model_service = ModelService::Disabled {};
    assert_eq!(required_memory(&recipe, 0)?, 3);
    recipe.worker_limits.memory_bytes = u64::MAX;
    assert!(verify_headroom(required_memory(&recipe, 0)?, 100).is_err());
    Ok(())
}
