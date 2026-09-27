use super::private_id_mappings;

fn inspected_limits() -> (crate::ContainerLimits, serde_json::Value) {
    let limits = crate::ContainerLimits {
        memory_bytes: 268_435_456,
        cpu_percent: 100,
        pids: 64,
        temporary_bytes: 16_777_216,
    };
    let value = serde_json::json!({
        "EffectiveCaps": [],
        "HostConfig": {
            "Privileged": false,
            "ReadonlyRootfs": true,
            "Memory": limits.memory_bytes,
            "MemorySwap": limits.memory_bytes,
            "PidsLimit": limits.pids,
            "CpuQuota": 100_000,
            "CpuPeriod": 100_000,
            "NetworkMode": "none",
            "SecurityOpt": ["no-new-privileges"],
            "Tmpfs": {"/tmp": format!("rw,nodev,nosuid,size={}", limits.temporary_bytes)},
            "IDMappings": {"UidMap": ["0:1:65536"], "GidMap": ["0:1:65536"]}
        }
    });
    (limits, value)
}

#[test]
fn observed_cpu_limits_require_a_positive_exact_ratio() {
    let (limits, value) = inspected_limits();
    assert!(super::verify_limits(&value, &limits).is_ok());
    for (quota, period) in [(0, 0), (0, 100_000), (100_000, 0), (u64::MAX, u64::MAX / 2)] {
        let mut changed = value.clone();
        changed["HostConfig"]["CpuQuota"] = quota.into();
        changed["HostConfig"]["CpuPeriod"] = period.into();
        assert!(
            super::verify_limits(&changed, &limits).is_err(),
            "unverified CPU enforcement: quota={quota}, period={period}"
        );
    }
}

#[test]
fn observed_privilege_restriction_must_be_explicitly_enabled() {
    let (limits, mut value) = inspected_limits();
    for enabled in ["no-new-privileges", "no-new-privileges=true"] {
        value["HostConfig"]["SecurityOpt"] = serde_json::json!([enabled]);
        assert!(super::verify_limits(&value, &limits).is_ok());
    }
    for options in [
        serde_json::json!([]),
        serde_json::json!(["no-new-privileges=false"]),
        serde_json::json!(["no-new-privileges-unknown"]),
        serde_json::json!(["no-new-privileges", "no-new-privileges=false"]),
    ] {
        value["HostConfig"]["SecurityOpt"] = options;
        assert!(super::verify_limits(&value, &limits).is_err());
    }
}

#[test]
fn task_commands_require_the_inspected_identity_and_owned_generation()
-> Result<(), Box<dyn std::error::Error>> {
    let setup = milkdrift_persistence::managed::ApprovedSetup {
        protection: None,
        recipe: crate::LinuxRecipe::from_json(include_bytes!(
            "../../tests/fixtures/owned-recipe-v2.json"
        ))?
        .reference()?,
        mechanism: "linux-quadlet-v4".to_owned(),
        configuration: milkdrift_capability::BoundedJson::new(serde_json::json!({}))?,
        platform_owner: "test-platform".to_owned(),
        ownership: crate::digest("test-task-owner"),
        resources: Vec::new(),
        capabilities: Vec::new(),
    };
    let id = "a".repeat(64);
    let name = "mdtask-owned";
    let value = serde_json::json!({
        "Id": id, "Name": name, "Config": {"Labels": {
            "org.milkdrift.owner": setup.ownership,
            "org.milkdrift.platform": setup.platform_owner,
            "org.milkdrift.recipe": setup.recipe.digest
        }}
    });
    assert_eq!(super::task_identity(&value, &setup, name)?, id);
    assert!(super::task_identity(&value, &setup, "mdtask-other").is_err());
    for invalid in ["", "--all", "short-id", &"z".repeat(64)] {
        let mut changed = value.clone();
        changed["Id"] = invalid.into();
        assert!(super::task_identity(&changed, &setup, name).is_err());
    }
    for key in [
        "org.milkdrift.owner",
        "org.milkdrift.platform",
        "org.milkdrift.recipe",
    ] {
        let mut changed = value.clone();
        changed["Config"]["Labels"][key] = "foreign".into();
        assert!(super::task_identity(&changed, &setup, name).is_err());
    }
    Ok(())
}

#[test]
fn realized_user_maps_exclude_the_manager_identity() {
    for offset in [1, 65537] {
        assert!(private_id_mappings(Some(&serde_json::json!({
            "UidMap":[format!("0:{offset}:65536")], "GidMap":[format!("0:{offset}:65536")]
        }))));
    }
    for invalid in [
        "0:0:65536",
        "0:1:65535",
        "1:1:65536",
        "0:1:0",
        "bad",
        "0:18446744073709551615:65536",
    ] {
        for key in ["UidMap", "GidMap"] {
            let mut maps = serde_json::json!({"UidMap":["0:1:65536"],"GidMap":["0:1:65536"]});
            maps[key] = serde_json::json!([invalid]);
            assert!(!private_id_mappings(Some(&maps)), "{key}: {invalid}");
        }
    }
    assert!(!private_id_mappings(None));
    assert!(!private_id_mappings(Some(&serde_json::json!({}))));
}

#[test]
fn encoded_output_bound_covers_escaping_replacement_and_extreme_exit_codes()
-> Result<(), Box<dyn std::error::Error>> {
    let stdout = [0, 1, b'"', b'\\', 255];
    let stderr = [254, 0];
    let document = super::result_document(
        &String::from_utf8_lossy(&stdout),
        &String::from_utf8_lossy(&stderr),
        false,
        Some(i32::MIN),
    );
    let bound = super::output_artifact_limit((stdout.len() + stderr.len()) as u64)?;
    assert!(serde_json::to_vec(&document)?.len() as u64 <= bound);
    assert!(super::output_artifact_limit(u64::MAX).is_err());
    Ok(())
}
