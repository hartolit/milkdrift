use crate::{
    recipe::{Deployment, InferenceBackend, ModelService},
    rejected,
};
use milkdrift_capability_host::managed::ManagedError;
use milkdrift_persistence::managed::ApprovedSetup;

pub(crate) fn deployment(setup: &ApprovedSetup) -> Result<Deployment, ManagedError> {
    let d: Deployment =
        serde_json::from_value(setup.configuration.value().clone()).map_err(rejected)?;
    if !milkdrift_contracts::is_canonical_blake3_digest(&setup.ownership) || d.generation == 0 {
        return Err(rejected("invalid deployment ownership or generation"));
    }
    let prefix = volume_prefix(&setup.platform_owner, &setup.ownership);
    if d.volume_prefix != prefix || d.unit != format!("{prefix}g{}", d.generation) {
        return Err(rejected(
            "deployed identities differ from durable ownership",
        ));
    }
    if d.recipe.reference()? != setup.recipe {
        return Err(rejected(
            "stored deployment differs from its approved recipe",
        ));
    }
    Ok(d)
}

pub(crate) fn volume_prefix(platform_owner: &str, ownership: &str) -> String {
    let digest = crate::digest(format!("{platform_owner}:{ownership}"));
    format!("md{}", &digest[3..35])
}

pub(crate) fn unit_text(
    setup: &ApprovedSetup,
    running: bool,
) -> Result<Option<String>, ManagedError> {
    let d = deployment(setup)?;
    let ModelService::Owned {
        image,
        executable,
        model,
        port,
        context_tokens,
        threads,
        backend,
        ..
    } = &d.recipe.model_service
    else {
        return Ok(None);
    };
    let mut text = format!(
        "# Milkdrift ownership={} recipe={}\n[Unit]\nDescription=Milkdrift owned llama-server\n[Container]\nImage={}\nPull=never\nContainerName={}\nLabel=org.milkdrift.owner={}\nLabel=org.milkdrift.platform={}\nLabel=org.milkdrift.recipe={}\nReadOnly=true\nNoNewPrivileges=true\nEnvironment=XDG_CACHE_HOME=/tmp\nDropCapability=all\nUserNS=auto:size=65536\nNetwork=pasta:--no-map-gw\nPublishPort=127.0.0.1:{}:8080\nVolume={}:/models/model.gguf:ro\nEntrypoint={}\nExec=--model /models/model.gguf --host 0.0.0.0 --port 8080 --ctx-size {} --threads {} --parallel 1 --alias ornith\nPodmanArgs=--replace=false --read-only-tmpfs=false --tmpfs=/tmp:rw,nodev,nosuid,size=256m --memory={} --memory-swap={} --cpus={} --pids-limit={} --security-opt=no-new-privileges\n[Service]\nRestart=on-failure\nTimeoutStartSec=40\nTimeoutStopSec=20\n",
        setup.ownership,
        setup.recipe.digest,
        image,
        d.unit,
        setup.ownership,
        setup.platform_owner,
        setup.recipe.digest,
        port,
        model.display(),
        executable,
        context_tokens,
        threads,
        d.recipe.memory_bytes,
        d.recipe.memory_bytes,
        f64::from(d.recipe.cpu_percent) / 100.0,
        d.recipe.pids
    );
    if let InferenceBackend::Vulkan { render_device } = backend {
        // Insert only the validated exact device; additional groups are owned by the rootless
        // runtime and must pass the real-host verification before this profile is usable.
        text = text.replace(
            "ReadOnly=true\n",
            &format!("AddDevice={}\nReadOnly=true\n", render_device.display()),
        );
        text = text.replace(" --parallel 1", " --n-gpu-layers 999 --parallel 1");
    }
    if running {
        text.push_str("[Install]\nWantedBy=default.target\n");
    }
    Ok(Some(text))
}
