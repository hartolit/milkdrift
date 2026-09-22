use super::regular_file;
use crate::{ModelService, rejected};
use milkdrift_capability_host::managed::ManagedError;
use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};

pub(super) fn verify_podman_version(version: &str) -> Result<(), ManagedError> {
    let mut parts = version.split('.');
    let major = parts.next().and_then(|p| p.parse::<u32>().ok());
    let minor = parts.next().and_then(|p| p.parse::<u32>().ok());
    if !matches!((major, minor), (Some(5), Some(4..)) | (Some(6), Some(_))) {
        return Err(rejected(
            "supported mechanism requires Podman 5.4..5.x or 6.x; other majors require mechanism qualification",
        ));
    }
    Ok(())
}

pub(super) fn verify_controllers(info: &serde_json::Value) -> Result<(), ManagedError> {
    let controllers = info
        .pointer("/host/cgroupControllers")
        .and_then(|v| v.as_array());
    let missing: Vec<_> = ["cpu", "memory", "pids"]
        .into_iter()
        .filter(|required| {
            !controllers.is_some_and(|values| values.iter().any(|v| v.as_str() == Some(required)))
        })
        .collect();
    if !missing.is_empty() {
        return Err(rejected(format!(
            "missing delegated cgroup v2 controllers: {}; configure the systemd user service delegation before prepare; limits will not be weakened",
            missing.join(", ")
        )));
    }
    Ok(())
}

pub(super) fn verify_subordinate_ids(
    info: &serde_json::Value,
    service: &ModelService,
) -> Result<(), ManagedError> {
    // The owned model and a worker need separate simultaneous auto user namespaces.
    let required = if matches!(service, ModelService::Owned { .. }) {
        2
    } else {
        1
    };
    for mapping in ["/host/idMappings/uidmap", "/host/idMappings/gidmap"] {
        if !info
            .pointer(mapping)
            .and_then(|v| v.as_array())
            .is_some_and(|maps| {
                maps.iter()
                    .filter(|m| {
                        m.get("container_id")
                            .and_then(|v| v.as_u64())
                            .is_some_and(|id| id > 0)
                    })
                    .filter_map(|m| m.get("size").and_then(|n| n.as_u64()))
                    .fold(0_u64, |count, size| count.saturating_add(size / 65_536))
                    >= required
            })
        {
            return Err(rejected(
                "rootless setup requires 65536 subordinate UIDs/GIDs per simultaneous worker and owned service",
            ));
        }
    }
    Ok(())
}

pub(super) fn verify_model(path: &Path, expected: &str, size: u64) -> Result<(), ManagedError> {
    regular_file(path, size)?;
    let mut file = fs::File::open(path).map_err(rejected)?;
    if file.metadata().map_err(rejected)?.len() != size {
        return Err(rejected("model size differs"));
    }
    let mut hasher = blake3::Hasher::new();
    let mut chunk = [0; 1_048_576];
    use std::io::Read;
    let deadline = Instant::now() + Duration::from_secs(300);
    let mut total = 0_u64;
    loop {
        if Instant::now() >= deadline {
            return Err(rejected("model verification exceeded five minutes"));
        }
        let n = file.read(&mut chunk).map_err(rejected)?;
        if n == 0 {
            break;
        }
        total = total.saturating_add(n as u64);
        if total > size {
            return Err(rejected("model changed during bounded verification"));
        }
        hasher.update(&chunk[..n]);
    }
    if format!("b3_{}", hasher.finalize()) != expected {
        return Err(rejected("model bytes differ from approved digest"));
    }
    Ok(())
}
