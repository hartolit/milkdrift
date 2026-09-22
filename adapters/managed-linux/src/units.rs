use crate::{
    command, platform_error,
    recipe::{Deployment, InferenceBackend, ModelService},
    rejected,
};
use milkdrift_capability_host::managed::ManagedError;
use milkdrift_persistence::managed::ApprovedSetup;
use std::{fs, path::Path, time::Duration};

pub(crate) fn systemd_directory(path: &Path) -> Result<(), ManagedError> {
    let metadata = fs::symlink_metadata(path).map_err(rejected)?;
    if !metadata.is_dir() || fs::canonicalize(path).map_err(rejected)? != path {
        return Err(rejected(
            "systemd user unit directory must be canonical without symlinks",
        ));
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != rustix::process::getuid().as_raw() || metadata.mode() & 0o022 != 0 {
            return Err(rejected(
                "systemd user unit directory must be manager-owned and not writable by other identities",
            ));
        }
    }
    Ok(())
}

fn cleanup_text(setup: &ApprovedSetup) -> String {
    // Quadlet 6 appends removal by name, even after custom [Service] directives. A systemd
    // drop-in is applied afterwards, so a failed creation can never delete a name collision.
    format!(
        "# Milkdrift ownership={} recipe={}\n[Service]\nRuntimeDirectory=milkdrift-%N\nRuntimeDirectoryMode=0700\nExecStop=\nExecStop=/usr/bin/podman stop --ignore --time=10 --cidfile=%t/milkdrift-%N/container.cid\nExecStopPost=\nExecStopPost=/usr/bin/podman rm --force --ignore --cidfile=%t/milkdrift-%N/container.cid\n",
        setup.ownership, setup.recipe.digest
    )
}

pub(crate) fn verify_cleanup(setup: &ApprovedSetup) -> Result<(), ManagedError> {
    let d = deployment(setup)?;
    let directory = d.systemd_directory.join(format!("{}.service.d", d.unit));
    if !directory.try_exists().map_err(platform_error)? {
        if fs::symlink_metadata(&directory).is_ok() {
            return Err(platform_error("symlink at service drop-in path"));
        }
        return Ok(());
    }
    crate::platform::private_directory(&directory)?;
    for entry in fs::read_dir(&directory).map_err(platform_error)? {
        if entry.map_err(platform_error)?.file_name() != "50-milkdrift.conf" {
            return Err(platform_error(
                "unexpected configuration in owned service drop-in directory",
            ));
        }
    }
    let path = directory.join("50-milkdrift.conf");
    if path.try_exists().map_err(platform_error)? {
        crate::platform::regular_file(&path, 65_536)?;
        if fs::read_to_string(&path).map_err(platform_error)? != cleanup_text(setup) {
            return Err(platform_error(
                "owned service cleanup configuration drift detected",
            ));
        }
    } else if fs::symlink_metadata(&path).is_ok() {
        return Err(platform_error("symlink at service cleanup path"));
    }
    Ok(())
}

pub(crate) fn install_cleanup(setup: &ApprovedSetup) -> Result<(), ManagedError> {
    use std::io::Write;
    let d = deployment(setup)?;
    systemd_directory(&d.systemd_directory)?;
    verify_cleanup(setup)?;
    let directory = d.systemd_directory.join(format!("{}.service.d", d.unit));
    if !directory.exists() {
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&directory).map_err(platform_error)?;
    }
    let path = directory.join("50-milkdrift.conf");
    if !path.exists() {
        let mut file = tempfile::NamedTempFile::new_in(&directory).map_err(platform_error)?;
        file.write_all(cleanup_text(setup).as_bytes())
            .map_err(platform_error)?;
        file.as_file().sync_all().map_err(platform_error)?;
        file.persist_noclobber(path).map_err(platform_error)?;
        fs::File::open(&directory)
            .and_then(|f| f.sync_all())
            .map_err(platform_error)?;
        fs::File::open(&d.systemd_directory)
            .and_then(|f| f.sync_all())
            .map_err(platform_error)?;
    }
    Ok(())
}

pub(crate) fn remove_cleanup(setup: &ApprovedSetup) -> Result<(), ManagedError> {
    verify_cleanup(setup)?;
    let d = deployment(setup)?;
    let directory = d.systemd_directory.join(format!("{}.service.d", d.unit));
    let path = directory.join("50-milkdrift.conf");
    if path.exists() {
        fs::remove_file(path).map_err(platform_error)?;
    }
    if directory.exists() {
        fs::remove_dir(directory).map_err(platform_error)?;
    }
    fs::File::open(&d.systemd_directory)
        .and_then(|f| f.sync_all())
        .map_err(platform_error)?;
    Ok(())
}

pub(crate) fn verify_effective_cleanup(d: &Deployment) -> Result<(), ManagedError> {
    let runtime = std::env::var("XDG_RUNTIME_DIR").map_err(rejected)?;
    for (property, verb) in [
        ("ExecStop", "stop --ignore --time=10"),
        ("ExecStopPost", "rm --force --ignore"),
    ] {
        let output = command::run(
            Path::new("/usr/bin/systemctl"),
            &[
                "--user".to_owned(),
                "show".to_owned(),
                format!("{}.service", d.unit),
                format!("--property={property}"),
                "--value".to_owned(),
            ],
            &[],
            Duration::from_secs(15),
            65_536,
            None,
        )?;
        let expected = format!(
            "/usr/bin/podman {verb} --cidfile={runtime}/milkdrift-{}/container.cid",
            d.unit
        );
        let text = String::from_utf8_lossy(&output.stdout);
        let commands: Vec<_> = text
            .split("argv[]=")
            .skip(1)
            .filter_map(|part| part.split_once(" ;").map(|p| p.0))
            .collect();
        if !output.success || commands != [expected.as_str()] {
            return Err(platform_error(
                "effective systemd cleanup is not the exact owned container-ID command; verify the configured systemd user unit search directory",
            ));
        }
    }
    Ok(())
}

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
        "# Milkdrift ownership={} recipe={}\n[Unit]\nDescription=Milkdrift owned llama-server\n[Container]\nImage={}\nPull=never\nContainerName={}\nLabel=org.milkdrift.owner={}\nLabel=org.milkdrift.platform={}\nLabel=org.milkdrift.recipe={}\nReadOnly=true\nNoNewPrivileges=true\nEnvironment=XDG_CACHE_HOME=/tmp\nDropCapability=all\nUserNS=auto:size=65536\nNetwork=pasta:--no-map-gw\nPublishPort=127.0.0.1:{}:8080\nVolume={}:/models/model.gguf:ro\nEntrypoint={}\nExec=--model /models/model.gguf --host 0.0.0.0 --port 8080 --ctx-size {} --threads {} --parallel 1 --alias ornith\nPodmanArgs=--replace=false --cidfile=%t/milkdrift-%N/container.cid --read-only-tmpfs=false --tmpfs=/tmp:rw,nodev,nosuid,size=256m --memory={} --memory-swap={} --cpus={} --pids-limit={} --security-opt=no-new-privileges\n[Service]\nRestart=on-failure\nTimeoutStartSec=40\nTimeoutStopSec=20\n",
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
