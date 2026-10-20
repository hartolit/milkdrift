//! An attached folder remains user data: the worker owns a descriptor and a temporary bind only.

use crate::{platform_error, rejected, units};
use milkdrift_capability_host::{
    AdapterExecutionContext, ProjectProcessAccess, ProjectProcessKind, managed::ManagedError,
};
use milkdrift_persistence::managed::ApprovedSetup;
use std::{fs::File, path::PathBuf};
pub(crate) mod pin;

// A non-root container identity maps to the invoking rootless user's UID/GID. All other
// container identities remain in the private auto allocation; no project ownership is changed.
const PROJECT_ID: u64 = 65_534;

pub(super) struct PinnedProject {
    directory: File,
    requested: PathBuf,
    physical: PathBuf,
    identity: String,
    source: String,
    write: bool,
    #[cfg(target_os = "linux")]
    pin: Option<pin::RootlessPin>,
}

impl PinnedProject {
    pub(super) fn capture(
        context: &AdapterExecutionContext,
        setup: &ApprovedSetup,
    ) -> Result<Option<Self>, ManagedError> {
        let deployment = units::deployment(setup)?;
        let access = context
            .project_process(
                ProjectProcessKind::Podman,
                &format!("managed.{}.worker", deployment.installation),
                "workspace.execute",
                deployment.generation,
            )
            .map_err(rejected)?;
        let (requested, physical, identity, write) = match access {
            None => return Ok(None),
            Some(ProjectProcessAccess::Isolated) => {
                return Err(rejected(
                    "Podman project execution requires project read access",
                ));
            }
            Some(ProjectProcessAccess::Directory {
                requested,
                physical,
                identity,
                write,
            }) => (
                PathBuf::from(requested),
                PathBuf::from(physical),
                identity,
                write,
            ),
        };
        let runtime = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .ok_or_else(|| rejected("Podman project attachment requires an active user session"))?;
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| rejected("Podman project attachment requires a configured user home"))?;
        for protected in [
            deployment.manager_root,
            deployment.quadlet_directory,
            deployment.systemd_directory,
            runtime,
            home.join(".local/share/containers"),
            home.join(".config/containers"),
            PathBuf::from("/proc"),
            PathBuf::from("/sys"),
            PathBuf::from("/dev"),
        ] {
            if physical.starts_with(&protected) || protected.starts_with(&physical) {
                return Err(rejected(
                    "project attachment overlaps manager, engine or kernel state; select a dedicated project folder",
                ));
            }
        }
        Self::open(requested, physical, identity.into(), write).map(Some)
    }

    #[cfg(target_os = "linux")]
    fn open(
        requested: PathBuf,
        physical: PathBuf,
        identity: String,
        write: bool,
    ) -> Result<Self, ManagedError> {
        use rustix::fs::{Mode, OFlags};
        use std::os::{fd::AsRawFd, unix::fs::MetadataExt};
        if !physical.is_absolute()
            || physical.parent().is_none()
            || physical.canonicalize().map_err(rejected)? != physical
            || requested.canonicalize().map_err(rejected)? != physical
        {
            return Err(rejected(
                "project folder no longer resolves to its accepted native identity",
            ));
        }
        let directory = File::from(
            rustix::fs::open(
                &physical,
                OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY | OFlags::NOFOLLOW,
                Mode::empty(),
            )
            .map_err(rejected)?,
        );
        let metadata = directory.metadata().map_err(rejected)?;
        if format!("unix:{}:{}", metadata.dev(), metadata.ino()) != identity {
            return Err(rejected(
                "project folder was replaced before Podman preparation",
            ));
        }
        if rustix::process::getuid().is_root()
            || metadata.uid() != rustix::process::getuid().as_raw()
            || metadata.gid() != rustix::process::getgid().as_raw()
            || metadata.mode() & 0o500 != 0o500
            || (write && metadata.mode() & 0o200 == 0)
        {
            return Err(rejected(
                "Podman project folders must belong to the daemon user's UID and primary GID with owner read/search and selected write access; ownership and labels are never changed automatically",
            ));
        }
        // Refer to the daemon's held descriptor, not a pathname that can be replaced between
        // create and start. The engine must retain this exact source in its inspected mount.
        let source = format!("/proc/{}/fd/{}", std::process::id(), directory.as_raw_fd());
        Ok(Self {
            directory,
            requested,
            physical,
            identity,
            source,
            write,
            pin: None,
        })
    }

    pub(super) fn activate(
        &mut self,
        helper: Option<&std::path::Path>,
        lifetime_ms: u64,
    ) -> Result<(), ManagedError> {
        let helper = helper.ok_or_else(|| rejected(
            "project attachment requires the daemon's configured Rust pin helper; ordinary owned-volume workers remain available"
        ))?;
        self.verify_helper_path(helper)?;
        #[cfg(target_os = "linux")]
        {
            self.revalidate()?;
            let engine =
                crate::LinuxManagedPlatform::podman(&["info".into(), "--format=json".into()])?;
            self.verify_engine_paths(&serde_json::from_slice(&engine).map_err(platform_error)?)?;
            let pin = pin::RootlessPin::start(helper, &self.physical, &self.identity, lifetime_ms)?;
            self.source.clone_from(&pin.source);
            self.pin = Some(pin);
            Ok(())
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (helper, lifetime_ms);
            Err(rejected("project attachment requires Linux"))
        }
    }

    fn verify_helper_path(&self, helper: &std::path::Path) -> Result<(), ManagedError> {
        let helper = helper.canonicalize().map_err(rejected)?;
        if helper.starts_with(&self.physical) {
            return Err(rejected(
                "project attachment contains the daemon pin helper executable; select a dedicated folder outside operator executables",
            ));
        }
        Ok(())
    }

    fn verify_engine_paths(&self, value: &serde_json::Value) -> Result<(), ManagedError> {
        for field in ["graphRoot", "runRoot"] {
            let path = value
                .get("store")
                .and_then(|store| store.get(field))
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| rejected("Podman did not report its protected storage paths"))?;
            let path = PathBuf::from(path).canonicalize().map_err(rejected)?;
            if path.starts_with(&self.physical) || self.physical.starts_with(&path) {
                return Err(rejected(
                    "project attachment overlaps Podman's configured storage or runtime root",
                ));
            }
        }
        Ok(())
    }

    pub(super) fn stop_pin(&mut self) -> Result<(), ManagedError> {
        #[cfg(target_os = "linux")]
        if let Some(pin) = &mut self.pin {
            pin.stop()?;
        }
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    fn open(
        _requested: PathBuf,
        _physical: PathBuf,
        _identity: String,
        _write: bool,
    ) -> Result<Self, ManagedError> {
        Err(rejected(
            "Podman project attachments require Linux native directory identities",
        ))
    }

    pub(super) fn revalidate(&self) -> Result<(), ManagedError> {
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::MetadataExt;
            let held = self.directory.metadata().map_err(rejected)?;
            let current = self.physical.metadata().map_err(rejected)?;
            if self.requested.canonicalize().map_err(rejected)? != self.physical
                || format!("unix:{}:{}", held.dev(), held.ino()) != self.identity
                || format!("unix:{}:{}", current.dev(), current.ino()) != self.identity
            {
                return Err(rejected(
                    "project folder changed after preparation; refusing worker entry",
                ));
            }
            Ok(())
        }
        #[cfg(not(target_os = "linux"))]
        Err(rejected("Podman project attachments require Linux"))
    }

    pub(super) fn arguments(&self) -> Vec<String> {
        vec![
            format!(
                "--userns=auto:size={},uidmapping={PROJECT_ID}:0:1,gidmapping={PROJECT_ID}:0:1",
                crate::recipe::PRIVATE_USER_IDS
            ),
            format!("--user={PROJECT_ID}:{PROJECT_ID}"),
            "--mount".into(),
            format!(
                "type=bind,src={},dst=/project,{},bind-propagation=rprivate,bind-nonrecursive",
                self.source,
                if self.write { "rw" } else { "ro" }
            ),
            "--workdir=/project".into(),
            "--env=HOME=/tmp".into(),
        ]
    }

    pub(super) fn verify(&self, value: &serde_json::Value) -> Result<(), ManagedError> {
        self.revalidate()?;
        if value
            .pointer("/Config/WorkingDir")
            .and_then(serde_json::Value::as_str)
            != Some("/project")
            || value
                .pointer("/Config/User")
                .and_then(serde_json::Value::as_str)
                != Some(format!("{PROJECT_ID}:{PROJECT_ID}").as_str())
            || !project_id_mappings(value.pointer("/HostConfig/IDMappings"))
        {
            return Err(platform_error(
                "attached worker cwd or realized non-root UID/GID mapping differs",
            ));
        }
        let mounts = value
            .get("Mounts")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| platform_error("attached worker actual mounts missing"))?;
        let mut projects = 0;
        for mount in mounts {
            match mount.get("Destination").and_then(serde_json::Value::as_str) {
                Some("/tmp")
                    if mount.get("Type").and_then(serde_json::Value::as_str) == Some("tmpfs") => {}
                Some("/project")
                    if mount.get("Type").and_then(serde_json::Value::as_str) == Some("bind")
                        && mount.get("Source").and_then(serde_json::Value::as_str)
                            == Some(self.source.as_str())
                        && mount.get("RW").and_then(serde_json::Value::as_bool)
                            == Some(self.write)
                        && mount.get("Propagation").and_then(serde_json::Value::as_str)
                            == Some("rprivate") =>
                {
                    projects += 1;
                }
                _ => {
                    return Err(platform_error(
                        "attached worker has an undeclared or changed mount",
                    ));
                }
            }
        }
        if projects != 1 {
            return Err(platform_error(
                "exact project attachment is absent or duplicated",
            ));
        }
        Ok(())
    }
}

pub(super) fn project_id_mappings(value: Option<&serde_json::Value>) -> bool {
    ["UidMap", "GidMap"].into_iter().all(|key| {
        value
            .and_then(|value| value.get(key))
            .and_then(serde_json::Value::as_array)
            .is_some_and(|ranges| {
                // Podman appends explicitly requested mappings after automatic ranges. Their
                // serialized order is not a kernel property; coverage and overlap still are.
                let Some(mut ranges) = ranges
                    .iter()
                    .map(|range| {
                        let parts = range
                            .as_str()?
                            .split(':')
                            .map(str::parse::<u64>)
                            .collect::<Result<Vec<_>, _>>()
                            .ok()?;
                        let [container, host, size] = parts.as_slice() else {
                            return None;
                        };
                        Some((*container, *host, *size))
                    })
                    .collect::<Option<Vec<_>>>()
                else {
                    return false;
                };
                ranges.sort_unstable_by_key(|range| range.0);
                let mut next = 0_u64;
                let mut user = false;
                !ranges.is_empty()
                    && ranges.iter().all(|(container, host, size)| {
                        if *container != next || *size == 0 || host.checked_add(*size).is_none() {
                            return false;
                        }
                        if *host == 0 {
                            if user || *container != PROJECT_ID || *size != 1 {
                                return false;
                            }
                            user = true;
                        }
                        let Some(end) = next.checked_add(*size) else {
                            return false;
                        };
                        next = end;
                        true
                    })
                    && user
                    && next == crate::recipe::PRIVATE_USER_IDS
            })
    })
}

#[cfg(test)]
mod tests;
