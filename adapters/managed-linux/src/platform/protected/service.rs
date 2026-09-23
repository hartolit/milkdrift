use super::{
    Deployment, LinuxManagedPlatform, ManagedError, decode, digest, ensure_root, immutable,
    regular_file, rejected,
};
use milkdrift_capability::managed::DataDisposition;
use milkdrift_persistence::managed::{
    ApprovedSetup, InstallationRecord, ManagedObservation, ManagedStep,
};
use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};

fn unit_text(setup: &ApprovedSetup, d: &Deployment) -> Result<String, ManagedError> {
    let candidate = d
        .candidate
        .as_ref()
        .ok_or_else(|| rejected("no verified candidate"))?;
    let r = &d.recipe;
    let config = d.root.join("application.json");
    let runtime = std::env::var("XDG_RUNTIME_DIR").map_err(rejected)?;
    if !crate::recipe::safe_absolute(Path::new(&runtime)) {
        return Err(rejected("unsafe runtime directory"));
    }
    let cid = format!("{runtime}/milkdrift-{}/container.cid", d.unit);
    Ok(format!(
        "# Milkdrift owner={} recipe={}\n[Unit]\nDescription=Milkdrift protected application\n[Service]\nType=notify\nNotifyAccess=all\nDelegate=yes\nKillMode=mixed\nStandardOutput=null\nStandardError=null\nRuntimeDirectory=milkdrift-{}\nRuntimeDirectoryMode=0700\nExecStart=/usr/bin/podman run --sdnotify=conmon --cgroups=split --pull=never --replace=false --name={} --cidfile={} --label=org.milkdrift.owner={} --label=org.milkdrift.platform={} --label=org.milkdrift.recipe={} --log-driver=none --read-only --read-only-tmpfs=false --tmpfs={} --cap-drop=all --security-opt=no-new-privileges --userns=auto:size=65536 --network=pasta:--no-map-gw --publish=127.0.0.1:{}:8080 --memory={} --memory-swap={} --cpus={} --pids-limit={} --volume={}:/candidate/app.py:ro --volume={}:/config/application.json:ro --volume={}:/config/token:ro --volume={}:/config/clock:ro --volume={}:/data:U --entrypoint={} {} -I /candidate/app.py\nExecStop=/usr/bin/podman stop --ignore --time={} --cidfile={}\nExecStopPost=/usr/bin/podman rm --force --ignore --cidfile={}\nRestart=on-failure\nTimeoutStartSec={}ms\nTimeoutStopSec={}s\n[Install]\nWantedBy=default.target\n",
        setup.ownership,
        setup.recipe.digest,
        d.unit,
        d.unit,
        cid,
        setup.ownership,
        setup.platform_owner,
        setup.recipe.digest,
        r.limits.temporary_mount(),
        r.port,
        r.limits.memory_bytes,
        r.limits.memory_bytes,
        r.limits.cpus(),
        r.limits.pids,
        candidate.display(),
        config.display(),
        d.root.join("token").display(),
        r.clock_file.display(),
        d.data_volume,
        r.executable,
        r.image,
        r.shutdown_ms.div_ceil(1000),
        cid,
        cid,
        r.startup_ms,
        r.shutdown_ms.div_ceil(1000) + 30
    ))
}
fn unit_path(d: &Deployment) -> std::path::PathBuf {
    d.systemd_directory.join(format!("{}.service", d.unit))
}
fn verify_unit(setup: &ApprovedSetup, d: &Deployment) -> Result<(), ManagedError> {
    let path = unit_path(d);
    if path.exists() {
        regular_file(&path, 65_536)?;
        if fs::read_to_string(&path).map_err(rejected)? != unit_text(setup, d)? {
            return Err(rejected("protected service definition drift"));
        }
        let dropins = LinuxManagedPlatform::systemctl(&[
            "show".to_owned(),
            "--property=DropInPaths".to_owned(),
            "--value".to_owned(),
            format!("{}.service", d.unit),
        ])?;
        if !dropins.iter().all(u8::is_ascii_whitespace) {
            return Err(rejected(
                "protected service has unapproved systemd overrides",
            ));
        }
    } else if fs::symlink_metadata(&path).is_ok() {
        return Err(rejected("protected unit symlink"));
    }
    Ok(())
}
fn systemctl(arguments: &[String]) -> Result<(), ManagedError> {
    LinuxManagedPlatform::systemctl(arguments).map(|_| ())
}

fn verify_candidate(setup: &ApprovedSetup, d: &Deployment) -> Result<(), ManagedError> {
    let path = d
        .candidate
        .as_ref()
        .ok_or_else(|| rejected("no verified candidate"))?;
    let artifact = &setup
        .protection
        .as_ref()
        .and_then(|p| p.evidence.as_ref())
        .ok_or_else(|| rejected("candidate lacks accepted evidence"))?
        .subject
        .artifact;
    regular_file(path, artifact.size_bytes())?;
    if !artifact.verifies(&fs::read(path).map_err(rejected)?) {
        return Err(rejected("candidate bytes differ from accepted evidence"));
    }
    Ok(())
}
fn stop(platform: &LinuxManagedPlatform, setup: &ApprovedSetup) -> Result<(), ManagedError> {
    let d = decode(platform, setup)?;
    verify_unit(setup, &d)?;
    if let Some(value) = LinuxManagedPlatform::inspect("container", &d.unit)? {
        LinuxManagedPlatform::verify_labels(&value, setup, Some(&setup.recipe.digest))?;
    }
    if unit_path(&d).exists() {
        systemctl(&[
            "disable".to_owned(),
            "--now".to_owned(),
            format!("{}.service", d.unit),
        ])?;
    }
    if LinuxManagedPlatform::inspect("container", &d.unit)?
        .is_some_and(|v| v.pointer("/State/Running").and_then(|v| v.as_bool()) == Some(true))
    {
        return Err(rejected("protected service did not stop"));
    }
    Ok(())
}
pub(super) fn observe(
    platform: &LinuxManagedPlatform,
    setup: &ApprovedSetup,
) -> Result<ManagedObservation, ManagedError> {
    let d = decode(platform, setup)?;
    verify_unit(setup, &d)?;
    let mut running = false;
    if let Some(value) = LinuxManagedPlatform::inspect("container", &d.unit)? {
        LinuxManagedPlatform::verify_labels(&value, setup, Some(&setup.recipe.digest))?;
        running = value.pointer("/State/Running").and_then(|v| v.as_bool()) == Some(true);
        if running {
            let token = fs::read(d.root.join("token")).map_err(rejected)?;
            let config = fs::read(d.root.join("application.json")).map_err(rejected)?;
            if digest(&token) != d.recipe.token_digest
                || config != serde_json::to_vec(d.recipe.application.value()).map_err(rejected)?
            {
                return Err(rejected(
                    "served credential or application configuration drift",
                ));
            }
            let host = crate::worker::verify_limits(&value, &d.recipe.limits)?;
            let ports = serde_json::json!({"8080/tcp":[{"HostIp":"127.0.0.1","HostPort":d.recipe.port.to_string()}]});
            if host.get("PortBindings") != Some(&ports) {
                return Err(rejected(
                    "protected endpoint differs from the private approved binding",
                ));
            }
            if value
                .get("Mounts")
                .and_then(|v| v.as_array())
                .is_none_or(|mounts| {
                    mounts.iter().any(|m| {
                        let destination =
                            m.get("Destination").and_then(|v| v.as_str()).unwrap_or("");
                        ![
                            "/candidate/app.py",
                            "/config/application.json",
                            "/config/token",
                            "/config/clock",
                            "/data",
                            "/tmp",
                        ]
                        .contains(&destination)
                            || (destination != "/data"
                                && destination != "/tmp"
                                && m.get("RW").and_then(|v| v.as_bool()) != Some(false))
                    })
                })
            {
                return Err(rejected(
                    "protected service has unexpected writable or host mounts",
                ));
            }
            let path = d
                .candidate
                .as_ref()
                .ok_or_else(|| rejected("running service has no candidate"))?;
            verify_candidate(setup, &d)?;
            LinuxManagedPlatform::verify_image(&value, &d.recipe.image)?;
            if value
                .pointer("/HostConfig/ReadonlyRootfs")
                .and_then(|v| v.as_bool())
                != Some(true)
                || !value
                    .get("Mounts")
                    .and_then(|v| v.as_array())
                    .is_some_and(|mounts| {
                        mounts.iter().any(|m| {
                            m.get("Destination").and_then(|v| v.as_str())
                                == Some("/candidate/app.py")
                                && m.get("Source").and_then(|v| v.as_str()) == path.to_str()
                                && m.get("RW").and_then(|v| v.as_bool()) == Some(false)
                        })
                    })
            {
                return Err(rejected(
                    "served candidate or isolation differs from verified bytes",
                ));
            }
        }
    }
    Ok(ManagedObservation {
        digest: digest(format!(
            "{}:{}:{}:{running}",
            setup.ownership, setup.recipe.digest, d.generation
        )),
        summary: format!(
            "protected generation {}; exact candidate and running={running}",
            d.generation
        ),
        running,
    })
}
pub(super) fn reconcile(
    platform: &LinuxManagedPlatform,
    record: &InstallationRecord,
    step: ManagedStep,
) -> Result<ManagedObservation, ManagedError> {
    let change = &record
        .pending
        .as_ref()
        .ok_or_else(|| rejected("protected effect needs durable intent"))?
        .change;
    let setup = &change.candidate;
    let d = decode(platform, setup)?;
    match step {
        ManagedStep::Prerequisites => {
            super::diagnose(platform, setup)?;
        }
        ManagedStep::PrepareStorage => {
            ensure_root(&d)?;
            if let Some(value) = LinuxManagedPlatform::inspect("volume", &d.data_volume)? {
                LinuxManagedPlatform::verify_labels(&value, setup, None)?;
            } else {
                LinuxManagedPlatform::podman(&[
                    "volume".to_owned(),
                    "create".to_owned(),
                    format!("--label=org.milkdrift.owner={}", setup.ownership),
                    format!("--label=org.milkdrift.platform={}", setup.platform_owner),
                    d.data_volume.clone(),
                ])?;
            }
        }
        ManagedStep::Configure if d.candidate.is_some() => {
            if change.running {
                verify_candidate(setup, &d)?;
            }
            ensure_root(&d)?;
            let token = fs::read(&d.recipe.token_file).map_err(rejected)?;
            if digest(&token) != d.recipe.token_digest {
                return Err(rejected("credential changed during preparation"));
            }
            immutable(&d.root.join("token"), &token)?;
            immutable(
                &d.root.join("application.json"),
                &serde_json::to_vec(d.recipe.application.value()).map_err(rejected)?,
            )?;
            verify_unit(setup, &d)?;
            immutable(&unit_path(&d), unit_text(setup, &d)?.as_bytes())?;
            systemctl(&["daemon-reload".to_owned()])?;
        }
        ManagedStep::Configure => {}
        ManagedStep::StartService if change.running => {
            super::active_policy(platform, setup)?;
            verify_candidate(setup, &d)?;
            verify_unit(setup, &d)?;
            regular_file(&unit_path(&d), 65_536)?;
            if let Some(value) = LinuxManagedPlatform::inspect("container", &d.unit)? {
                LinuxManagedPlatform::verify_labels(&value, setup, Some(&setup.recipe.digest))?;
            }
            systemctl(&[
                "enable".to_owned(),
                "--now".to_owned(),
                format!("{}.service", d.unit),
            ])?;
            let deadline = Instant::now() + Duration::from_millis(d.recipe.startup_ms);
            while !observe(platform, setup)?.running {
                if Instant::now() >= deadline {
                    return Err(rejected("protected service startup remained uncertain"));
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        }
        ManagedStep::StartService => {}
        ManagedStep::StopService => stop(platform, record.current.as_ref().unwrap_or(setup))?,
        ManagedStep::RemoveConfiguration => {
            let old = record.current.as_ref().unwrap_or(setup);
            stop(platform, old)?;
            let prior = decode(platform, old)?;
            verify_unit(old, &prior)?;
            if unit_path(&prior).exists() {
                fs::remove_file(unit_path(&prior)).map_err(rejected)?;
            }
            systemctl(&["daemon-reload".to_owned()])?;
        }
        ManagedStep::RemoveStorage => {
            if setup
                .resources
                .iter()
                .find(|r| r.name.as_str() == "data")
                .is_some_and(|r| r.disposition == DataDisposition::DeleteOnRemoval)
                && let Some(value) = LinuxManagedPlatform::inspect("volume", &d.data_volume)?
            {
                LinuxManagedPlatform::verify_labels(&value, setup, None)?;
                LinuxManagedPlatform::podman(&[
                    "volume".to_owned(),
                    "rm".to_owned(),
                    d.data_volume.clone(),
                ])?;
            }
        }
        ManagedStep::Verify => {
            let observation = observe(platform, setup)?;
            if observation.running != change.running {
                return Err(rejected(
                    "protected service state differs from intended state",
                ));
            }
            return Ok(observation);
        }
    }
    observe(platform, setup)
}
