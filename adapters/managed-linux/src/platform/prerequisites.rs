use super::{LinuxManagedPlatform, regular_file};
use crate::{LinuxRecipe, command, platform_error};
use crate::{ModelService, rejected};
use milkdrift_capability_host::managed::ManagedError;
use milkdrift_persistence::managed::ApprovedSetup;
use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};

fn verify_podman_version(version: &str) -> Result<(), ManagedError> {
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

fn verify_controllers(info: &serde_json::Value) -> Result<(), ManagedError> {
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

fn verify_subordinate_ids(info: &serde_json::Value, required: u64) -> Result<(), ManagedError> {
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
                    .fold(0_u64, |count, size| {
                        count.saturating_add(size / crate::recipe::PRIVATE_USER_IDS)
                    })
                    >= required
            })
        {
            return Err(rejected(
                "rootless setup requires 65536 subordinate UIDs/GIDs per simultaneous isolated container",
            ));
        }
    }
    Ok(())
}

fn verify_model(
    path: &Path,
    expected: &str,
    size: u64,
    timeout: Duration,
) -> Result<(), ManagedError> {
    regular_file(path, size)?;
    let mut file = fs::File::open(path).map_err(rejected)?;
    if file.metadata().map_err(rejected)?.len() != size {
        return Err(rejected("model size differs"));
    }
    let mut hasher = blake3::Hasher::new();
    let mut chunk = [0; 1_048_576];
    use std::io::Read;
    let deadline = Instant::now() + timeout;
    let mut total = 0_u64;
    loop {
        if Instant::now() >= deadline {
            return Err(rejected(
                "model_service.timeouts.model_verification_ms expired while hashing the model",
            ));
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

impl LinuxManagedPlatform {
    /// Shared prerequisites for all managed containers; resource-specific inputs stay with their recipe.
    pub(super) fn diagnose_host(
        simultaneous_containers: u64,
        persistent_service: bool,
    ) -> Result<String, ManagedError> {
        #[cfg(not(target_os = "linux"))]
        let _ = persistent_service;
        let bytes = Self::podman(&["info".to_owned(), "--format=json".to_owned()])?;
        let info: serde_json::Value = serde_json::from_slice(&bytes).map_err(platform_error)?;
        let version = info
            .pointer("/version/Version")
            .or_else(|| info.pointer("/version/version"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| rejected("Podman version unavailable"))?;
        verify_podman_version(version)?;
        if info
            .pointer("/host/security/rootless")
            .and_then(|v| v.as_bool())
            != Some(true)
            || info.pointer("/host/cgroupVersion").and_then(|v| v.as_str()) != Some("v2")
            || info.pointer("/host/cgroupManager").and_then(|v| v.as_str()) != Some("systemd")
        {
            return Err(rejected(
                "rootless Podman, cgroup v2 and systemd delegation are required",
            ));
        }
        verify_controllers(&info)?;
        Self::systemctl(&["show-environment".to_owned()])?;
        verify_subordinate_ids(&info, simultaneous_containers)?;
        #[cfg(target_os = "linux")]
        if persistent_service {
            let lingering = command::checked(
                "/usr/bin/loginctl",
                &[
                    "show-user".to_owned(),
                    rustix::process::getuid().as_raw().to_string(),
                    "--property=Linger".to_owned(),
                    "--value".to_owned(),
                ],
            )?;
            if lingering != b"yes\n" {
                return Err(rejected(
                    "persistent owned services require explicitly enabled systemd user lingering",
                ));
            }
        }
        Ok(version.to_owned())
    }

    pub(super) fn diagnose_setup(
        &self,
        setup: &ApprovedSetup,
        replacing: Option<&ApprovedSetup>,
    ) -> Result<Vec<String>, ManagedError> {
        let d = self.check_owner(setup)?;
        #[cfg(target_os = "linux")]
        {
            let page_size = rustix::param::page_size() as u64;
            d.recipe
                .worker_limits
                .validate_page_alignment(page_size, "worker_limits")?;
            if let ModelService::Owned { limits, .. } = &d.recipe.model_service {
                limits.validate_page_alignment(page_size, "model_service.limits")?;
            }
        }
        let persistent_service = matches!(d.recipe.model_service, ModelService::Owned { .. });
        let version =
            Self::diagnose_host(if persistent_service { 2 } else { 1 }, persistent_service)?;
        let memory = bounded_text("/proc/meminfo")?
            .lines()
            .find_map(|line| {
                line.strip_prefix("MemAvailable:")
                    .and_then(|v| v.split_whitespace().next())
                    .and_then(|v| v.parse::<u64>().ok())
            })
            .ok_or_else(|| rejected("available shared RAM could not be observed"))?
            .saturating_mul(1024);
        let credit = self.existing_service_memory(replacing)?;
        let required = required_memory(&d.recipe, credit)?;
        verify_headroom(required, memory)?;
        #[cfg(target_os = "linux")]
        {
            let stat = rustix::fs::statvfs(&self.config.state_root).map_err(rejected)?;
            if stat.f_bavail.saturating_mul(stat.f_frsize) < d.recipe.minimum_free_bytes {
                return Err(rejected("insufficient free storage for the approved setup"));
            }
        }
        Self::podman(&[
            "image".to_owned(),
            "inspect".to_owned(),
            d.recipe.worker_image.clone(),
        ])?;
        if let ModelService::Owned {
            image,
            model,
            model_digest,
            model_bytes,
            backend,
            timeouts,
            ..
        } = &d.recipe.model_service
        {
            Self::podman(&["image".to_owned(), "inspect".to_owned(), image.clone()])?;
            verify_model(
                model,
                model_digest,
                *model_bytes,
                Duration::from_millis(timeouts.model_verification_ms),
            )?;
            if let crate::InferenceBackend::Vulkan { render_device, .. } = backend {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::FileTypeExt;
                    if !fs::metadata(render_device)
                        .map_err(rejected)?
                        .file_type()
                        .is_char_device()
                    {
                        return Err(rejected(
                            "configured Vulkan render node is not a character device",
                        ));
                    }
                }
            }
        }
        Ok(vec![format!("Podman {version}; rootless systemd/cgroup v2; available shared RAM {memory} bytes"), "Only named working storage is writable by the worker; mutable tool experiments require explicit image promotion".to_owned(), format!("worker network: {:?}; model backend: {:?}", d.recipe.worker_network, match &d.recipe.model_service { ModelService::Owned { backend, .. } => Some(backend), _ => None })])
    }
    fn existing_service_memory(&self, setup: Option<&ApprovedSetup>) -> Result<u64, ManagedError> {
        let Some(setup) = setup else {
            return Ok(0);
        };
        let d = self.check_owner(setup)?;
        let ModelService::Owned { limits, .. } = &d.recipe.model_service else {
            return Ok(0);
        };
        let Some(container) = Self::inspect("container", &d.unit)? else {
            return Ok(0);
        };
        Self::verify_labels(&container, setup, Some(&setup.recipe.digest))?;
        if container
            .pointer("/State/Running")
            .and_then(|v| v.as_bool())
            != Some(true)
        {
            return Ok(0);
        }
        let pid = container
            .pointer("/State/Pid")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| {
                platform_error("owned service PID missing during headroom observation")
            })?;
        let cgroup = bounded_text(&format!("/proc/{pid}/cgroup"))?;
        let path = cgroup
            .lines()
            .find_map(|line| line.strip_prefix("0::"))
            .ok_or_else(|| platform_error("owned service has no unified cgroup"))?;
        if !Path::new(path).is_absolute()
            || !Path::new(path).components().all(|c| {
                matches!(
                    c,
                    std::path::Component::RootDir | std::path::Component::Normal(_)
                )
            })
        {
            return Err(platform_error("owned service cgroup path is not canonical"));
        }
        let stat = bounded_text(&format!("/sys/fs/cgroup{path}/memory.stat"))?;
        // MemAvailable already includes reclaimable file cache. Credit only anonymous resident
        // memory of this exact service, which will be retained or stopped by this transition.
        let anonymous = stat
            .lines()
            .find_map(|line| line.strip_prefix("anon "))
            .and_then(|v| v.parse::<u64>().ok())
            .ok_or_else(|| platform_error("owned service anonymous memory is unavailable"))?;
        let observed = Self::inspect("container", &d.unit)?
            .ok_or_else(|| platform_error("service disappeared during headroom observation"))?;
        if observed.get("Id") != container.get("Id")
            || observed.pointer("/State/Pid") != container.pointer("/State/Pid")
            || observed.pointer("/State/Running").and_then(|v| v.as_bool()) != Some(true)
        {
            return Err(platform_error(
                "service changed during headroom observation; retry preparation",
            ));
        }
        Ok(anonymous.min(limits.memory_bytes))
    }
}

fn bounded_text(path: &str) -> Result<String, ManagedError> {
    use std::io::Read;
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(platform_error)?
        .take(command::HELPER_OUTPUT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(platform_error)?;
    if bytes.len() > command::HELPER_OUTPUT_BYTES {
        return Err(platform_error(
            "kernel resource observation exceeded its document bound",
        ));
    }
    String::from_utf8(bytes).map_err(platform_error)
}

fn required_memory(recipe: &LinuxRecipe, existing_anonymous: u64) -> Result<u64, ManagedError> {
    let service = match &recipe.model_service {
        ModelService::Owned { limits, .. } => limits.memory_bytes,
        _ => 0,
    };
    let total = recipe
        .worker_limits
        .memory_bytes
        .checked_add(service)
        .ok_or_else(|| rejected("worker and service memory sum overflow"))?;
    Ok(total.saturating_sub(existing_anonymous))
}

#[cfg(test)]
mod tests;

fn verify_headroom(required: u64, available: u64) -> Result<(), ManagedError> {
    if required > available {
        return Err(rejected(format!(
            "insufficient observed host headroom: worker and service need {required} additional bytes, MemAvailable is {available}; these are independent caps, not reserved memory"
        )));
    }
    Ok(())
}
