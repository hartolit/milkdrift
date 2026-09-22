use crate::{
    LinuxManagerConfig, LinuxRecipe, ModelService, WorkerNetwork, command, digest, platform_error,
    recipe::Deployment, rejected, units,
};
use milkdrift_authority::{
    AccessMode, CapabilityExecutionRequirements, FilesystemScope, NetworkProfileRef,
};
use milkdrift_capability::{
    BoundedJson,
    managed::{
        ManagedName, ManagedResourceKind, ManagedResourceView, RecipeReference, ResourceOwnership,
    },
};
use milkdrift_capability_host::managed::{ManagedError, ManagedPlatform};
use milkdrift_persistence::managed::{
    ApprovedSetup, InstallationRecord, ManagedObservation, ManagedStep, ManagedUse,
    QuiescenceEvidence,
};
use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[cfg(test)]
mod tests;

/// Production rootless Linux adapter. Construction reads approved input files but creates no
/// accounts, services, sockets or containers. Every effect requires an accepted host transition.
pub struct LinuxManagedPlatform {
    config: LinuxManagerConfig,
    recipes: BTreeMap<ManagedName, LinuxRecipe>,
    owner: String,
    pub(crate) active: Mutex<BTreeMap<String, Arc<AtomicBool>>>,
    pub(crate) quiescent: Condvar,
    _lock: fs::File,
}
impl LinuxManagedPlatform {
    /// Freeze approved recipes and the manager's exact machine/account/data-root identity.
    pub fn new(config: LinuxManagerConfig) -> Result<Self, ManagedError> {
        config.validate()?;
        if !cfg!(target_os = "linux") {
            return Err(rejected(
                "managed Linux setup is unsupported on this platform",
            ));
        }
        private_directory(&config.state_root)?;
        private_directory(&config.quadlet_directory)?;
        let lock_path = config.state_root.join("manager.lock");
        if lock_path.exists() {
            regular_file(&lock_path, 0)?;
        }
        let mut options = fs::OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lock = options.open(&lock_path).map_err(rejected)?;
        lock.try_lock()
            .map_err(|_| rejected("another manager owns this platform root"))?;
        let mut recipes = BTreeMap::new();
        for path in &config.recipes {
            regular_file(path, 65_536)?;
            let recipe = LinuxRecipe::from_json(&fs::read(path).map_err(rejected)?)?;
            if recipes.insert(recipe.name.clone(), recipe).is_some() {
                return Err(rejected("duplicate managed recipe name"));
            }
        }
        let owner = owner_identity(&config.state_root)?;
        Ok(Self {
            config,
            recipes,
            owner,
            active: Mutex::new(BTreeMap::new()),
            quiescent: Condvar::new(),
            _lock: lock,
        })
    }
    pub(crate) fn check_owner(&self, setup: &ApprovedSetup) -> Result<Deployment, ManagedError> {
        let d = units::deployment(setup)?;
        if setup.mechanism != "linux-quadlet-v1"
            || setup.platform_owner != self.owner
            || d.manager_root != self.config.state_root
            || d.quadlet_directory != self.config.quadlet_directory
            || owner_identity(&self.config.state_root)? != self.owner
        {
            return Err(rejected(
                "platform owner changed; restored or copied state cannot adopt live resources",
            ));
        }
        let expected = resources(&d)?;
        if setup.resources.len() != expected.len()
            || setup
                .resources
                .iter()
                .zip(&expected)
                .any(|(actual, expected)| {
                    actual.name != expected.name
                        || actual.kind != expected.kind
                        || actual.ownership != expected.ownership
                        || actual.identity != expected.identity
                })
        {
            return Err(rejected(
                "resource inventory differs from exact deployed identities",
            ));
        }
        let mut capabilities = vec![crate::worker::descriptor(setup)?];
        if let Some(model) = crate::model::descriptor(setup)? {
            capabilities.push(model);
        }
        if capabilities != setup.capabilities {
            return Err(rejected(
                "published capabilities differ from exact deployed configuration",
            ));
        }
        private_directory(&self.config.state_root)?;
        private_directory(&self.config.quadlet_directory)?;
        Ok(d)
    }
    pub(crate) fn podman(args: &[String]) -> Result<Vec<u8>, ManagedError> {
        command::checked("/usr/bin/podman", args)
    }
    fn systemctl(args: &[String]) -> Result<Vec<u8>, ManagedError> {
        let mut all = vec!["--user".to_owned()];
        all.extend_from_slice(args);
        command::checked("/usr/bin/systemctl", &all)
    }

    pub(crate) fn inspect(
        kind: &str,
        name: &str,
    ) -> Result<Option<serde_json::Value>, ManagedError> {
        // An explicit existence command distinguishes absence from permission/engine failure.
        let result = command::run(
            Path::new("/usr/bin/podman"),
            &[kind.to_owned(), "exists".to_owned(), name.to_owned()],
            &[],
            Duration::from_secs(10),
            65_536,
            None,
        )?;
        if !result.success {
            // Podman exists returns 1 only for absence. A separate engine health check prevents
            // a missing socket/storage prerequisite from being interpreted as stop evidence.
            Self::podman(&["info".to_owned(), "--format=json".to_owned()])?;
            if result.exit_code != Some(1) {
                return Err(platform_error("platform existence check failed"));
            }
            return Ok(None);
        }
        let bytes = Self::podman(&[kind.to_owned(), "inspect".to_owned(), name.to_owned()])?;
        let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(platform_error)?;
        value
            .as_array()
            .and_then(|a| a.first())
            .cloned()
            .map(Some)
            .ok_or_else(|| platform_error("platform inspection returned no identity"))
    }
    pub(crate) fn verify_labels(
        value: &serde_json::Value,
        setup: &ApprovedSetup,
        recipe: Option<&str>,
    ) -> Result<(), ManagedError> {
        let labels = value
            .get("Labels")
            .or_else(|| value.pointer("/Config/Labels"))
            .ok_or_else(|| platform_error("owned resource has no labels"))?;
        if labels.get("org.milkdrift.owner").and_then(|v| v.as_str())
            != Some(setup.ownership.as_str())
            || labels
                .get("org.milkdrift.platform")
                .and_then(|v| v.as_str())
                != Some(setup.platform_owner.as_str())
            || recipe.is_some_and(|expected| {
                labels.get("org.milkdrift.recipe").and_then(|v| v.as_str()) != Some(expected)
            })
        {
            return Err(platform_error(
                "platform identity has foreign or mismatched ownership/configuration; adoption refused",
            ));
        }
        Ok(())
    }
    fn storage(&self, setup: &ApprovedSetup, remove: bool) -> Result<(), ManagedError> {
        self.check_owner(setup)?;
        for resource in &setup.resources {
            if resource.ownership != ResourceOwnership::Owned
                || !matches!(
                    resource.kind,
                    ManagedResourceKind::WorkingArea
                        | ManagedResourceKind::Staging
                        | ManagedResourceKind::Data
                )
            {
                continue;
            }
            if let Some(value) = Self::inspect("volume", &resource.identity)? {
                Self::verify_labels(&value, setup, None)?;
                if remove
                    && resource.disposition
                        == milkdrift_capability::managed::DataDisposition::DeleteOnRemoval
                {
                    // No --force and no prune: an independently active mount must refuse deletion.
                    Self::podman(&[
                        "volume".to_owned(),
                        "rm".to_owned(),
                        resource.identity.clone(),
                    ])?;
                }
            } else if !remove {
                Self::podman(&[
                    "volume".to_owned(),
                    "create".to_owned(),
                    "--label".to_owned(),
                    format!("org.milkdrift.owner={}", setup.ownership),
                    "--label".to_owned(),
                    format!("org.milkdrift.platform={}", setup.platform_owner),
                    resource.identity.clone(),
                ])?;
                let value = Self::inspect("volume", &resource.identity)?
                    .ok_or_else(|| platform_error("created volume is absent"))?;
                Self::verify_labels(&value, setup, None)?;
            }
        }
        Ok(())
    }
    fn configure(&self, record: &InstallationRecord) -> Result<(), ManagedError> {
        let pending = record
            .pending
            .as_ref()
            .ok_or_else(|| rejected("missing durable intent"))?;
        self.write_unit(&pending.change.candidate, pending.change.running)
    }
    fn write_unit(&self, setup: &ApprovedSetup, running: bool) -> Result<(), ManagedError> {
        let d = self.check_owner(setup)?;
        let Some(text) = units::unit_text(setup, running)? else {
            return Ok(());
        };
        let path = d.quadlet_directory.join(format!("{}.container", d.unit));
        if path.exists() {
            regular_file(&path, 65_536)?;
            let existing = fs::read_to_string(&path).map_err(platform_error)?;
            if existing != text && Some(existing.clone()) != units::unit_text(setup, !running)? {
                return Err(platform_error(
                    "existing Quadlet bytes differ from exact owned configuration",
                ));
            }
            if existing == text {
                Self::verify_generator(&d)?;
                Self::systemctl(&["daemon-reload".to_owned()])?;
                return Ok(());
            }
        } else if fs::symlink_metadata(&path).is_ok() {
            return Err(platform_error("refusing symlink at unit path"));
        }
        let mut temporary =
            tempfile::NamedTempFile::new_in(&d.quadlet_directory).map_err(platform_error)?;
        use std::io::Write;
        temporary
            .write_all(text.as_bytes())
            .map_err(platform_error)?;
        temporary.as_file().sync_all().map_err(platform_error)?;
        temporary.persist(&path).map_err(platform_error)?;
        fs::File::open(&d.quadlet_directory)
            .map_err(platform_error)?
            .sync_all()
            .map_err(platform_error)?;
        Self::verify_generator(&d)?;
        Self::systemctl(&["daemon-reload".to_owned()])?;
        Ok(())
    }
    fn verify_generator(d: &Deployment) -> Result<(), ManagedError> {
        let directory = d
            .quadlet_directory
            .to_str()
            .ok_or_else(|| rejected("non-UTF8 unit directory"))?;
        let output = command::run(
            Path::new("/usr/lib/systemd/system-generators/podman-system-generator"),
            &["--user".to_owned(), "--dryrun".to_owned()],
            &[("QUADLET_UNIT_DIRS", directory)],
            Duration::from_secs(15),
            1_048_576,
            None,
        )?;
        if !output.success
            || !String::from_utf8_lossy(&output.stdout).contains(&format!("{}.service", d.unit))
        {
            return Err(platform_error(
                "installed Quadlet generator rejected the supported definition; inspect local Podman manual",
            ));
        }
        Ok(())
    }
    fn stop(&self, setup: &ApprovedSetup) -> Result<(), ManagedError> {
        let d = self.check_owner(setup)?;
        if !matches!(d.recipe.model_service, ModelService::Owned { .. }) {
            return Ok(());
        }
        self.verify_unit(setup)?;
        if let Some(value) = Self::inspect("container", &d.unit)? {
            Self::verify_labels(&value, setup, Some(&setup.recipe.digest))?;
        }
        let unit_path = d.quadlet_directory.join(format!("{}.container", d.unit));
        if !unit_path.exists() {
            self.write_unit(setup, false)?;
        }
        Self::systemctl(&["stop".to_owned(), format!("{}.service", d.unit)])?;
        let state = Self::systemctl(&[
            "show".to_owned(),
            format!("{}.service", d.unit),
            "--property=ActiveState".to_owned(),
            "--value".to_owned(),
        ])?;
        if !matches!(
            String::from_utf8_lossy(&state).trim(),
            "inactive" | "failed"
        ) {
            return Err(platform_error("service supervisor has not stopped"));
        }
        if let Some(value) = Self::inspect("container", &d.unit)? {
            Self::verify_labels(&value, setup, Some(&setup.recipe.digest))?;
            if value.pointer("/State/Running").and_then(|v| v.as_bool()) != Some(false) {
                return Err(platform_error(
                    "service stop has no physical quiescence proof",
                ));
            }
            Self::podman(&["rm".to_owned(), d.unit.clone()])?;
        }
        Ok(())
    }
    fn verify_unit(&self, setup: &ApprovedSetup) -> Result<(), ManagedError> {
        let d = self.check_owner(setup)?;
        if !matches!(d.recipe.model_service, ModelService::Owned { .. }) {
            return Ok(());
        }
        let path = d.quadlet_directory.join(format!("{}.container", d.unit));
        if !path.exists() {
            return Ok(());
        }
        regular_file(&path, 65_536)?;
        let text = fs::read_to_string(path).map_err(platform_error)?;
        if Some(text.clone()) != units::unit_text(setup, true)?
            && Some(text) != units::unit_text(setup, false)?
        {
            return Err(platform_error("owned unit drift detected"));
        }
        Ok(())
    }
    fn remove_configuration(&self, setup: &ApprovedSetup) -> Result<(), ManagedError> {
        let d = self.check_owner(setup)?;
        if !matches!(d.recipe.model_service, ModelService::Owned { .. }) {
            return Ok(());
        }
        self.verify_unit(setup)?;
        if Self::inspect("container", &d.unit)?.is_some() {
            return Err(platform_error(
                "owned container remains; stop must be proven before configuration removal",
            ));
        }
        let path = d.quadlet_directory.join(format!("{}.container", d.unit));
        if path.exists() {
            fs::remove_file(path).map_err(platform_error)?;
        }
        Self::systemctl(&["daemon-reload".to_owned()])?;
        Ok(())
    }
}

fn resources(d: &Deployment) -> Result<Vec<ManagedResourceView>, ManagedError> {
    let recipe = &d.recipe;
    let mut resources = Vec::new();
    for (name, kind) in [
        ("working", ManagedResourceKind::WorkingArea),
        ("staging", ManagedResourceKind::Staging),
        ("data", ManagedResourceKind::Data),
    ] {
        resources.push(ManagedResourceView {
            name: ManagedName::new(name).map_err(rejected)?,
            kind,
            ownership: ResourceOwnership::Owned,
            identity: format!("{}-{name}", d.volume_prefix),
            disposition: recipe.data_disposition,
        });
    }
    resources.push(ManagedResourceView {
        name: ManagedName::new("toolchain").map_err(rejected)?,
        kind: ManagedResourceKind::Prerequisite,
        ownership: ResourceOwnership::Shared,
        identity: recipe.worker_image.clone(),
        disposition: milkdrift_capability::managed::DataDisposition::Preserve,
    });
    match &recipe.model_service {
        ModelService::Disabled {} => {}
        ModelService::Attached { api_base, .. } => resources.push(ManagedResourceView {
            name: ManagedName::new("model").map_err(rejected)?,
            kind: ManagedResourceKind::Service,
            ownership: ResourceOwnership::Attached,
            identity: api_base.clone(),
            disposition: recipe.data_disposition,
        }),
        ModelService::Owned { model, image, .. } => {
            for (name, kind, class, identity) in [
                (
                    "model",
                    ManagedResourceKind::Service,
                    ResourceOwnership::Owned,
                    d.unit.clone(),
                ),
                (
                    "weights",
                    ManagedResourceKind::Prerequisite,
                    ResourceOwnership::Shared,
                    model.display().to_string(),
                ),
                (
                    "server",
                    ManagedResourceKind::Prerequisite,
                    ResourceOwnership::Shared,
                    image.clone(),
                ),
            ] {
                resources.push(ManagedResourceView {
                    name: ManagedName::new(name).map_err(rejected)?,
                    kind,
                    ownership: class,
                    identity,
                    disposition: recipe.data_disposition,
                });
            }
        }
    }
    Ok(resources)
}

impl ManagedPlatform for LinuxManagedPlatform {
    fn plan(
        &self,
        installation: &ManagedName,
        reference: &RecipeReference,
        ownership: &str,
        generation: u64,
    ) -> Result<ApprovedSetup, ManagedError> {
        if !milkdrift_contracts::is_canonical_blake3_digest(ownership) || generation == 0 {
            return Err(rejected(
                "exact ownership digest and nonzero generation required",
            ));
        }
        let recipe = self
            .recipes
            .get(&reference.name)
            .ok_or_else(|| rejected("recipe is not approved by this manager"))?;
        if recipe.reference()? != *reference {
            return Err(rejected("approved recipe digest differs"));
        }
        let prefix = units::volume_prefix(&self.owner, ownership);
        let d = Deployment {
            recipe: recipe.clone(),
            installation: installation.clone(),
            generation,
            unit: format!("{prefix}g{generation}"),
            volume_prefix: prefix.clone(),
            manager_root: self.config.state_root.clone(),
            quadlet_directory: self.config.quadlet_directory.clone(),
        };
        let resources = resources(&d)?;
        let mut setup = ApprovedSetup {
            recipe: reference.clone(),
            mechanism: "linux-quadlet-v1".to_owned(),
            configuration: BoundedJson::new(serde_json::to_value(&d).map_err(rejected)?)
                .map_err(rejected)?,
            platform_owner: self.owner.clone(),
            ownership: ownership.to_owned(),
            resources,
            capabilities: Vec::new(),
        };
        setup.capabilities.push(crate::worker::descriptor(&setup)?);
        if let Some(descriptor) = crate::model::descriptor(&setup)? {
            setup.capabilities.push(descriptor);
        }
        setup.validate().map_err(rejected)?;
        Ok(setup)
    }
    fn diagnose(&self, setup: &ApprovedSetup) -> Result<Vec<String>, ManagedError> {
        let d = self.check_owner(setup)?;
        let bytes = Self::podman(&["info".to_owned(), "--format=json".to_owned()])?;
        let info: serde_json::Value = serde_json::from_slice(&bytes).map_err(platform_error)?;
        let version = info
            .pointer("/version/Version")
            .or_else(|| info.pointer("/version/version"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| rejected("Podman version unavailable"))?;
        let parts: Vec<_> = version.split('.').collect();
        if parts.first() != Some(&"5")
            || parts
                .get(1)
                .and_then(|p| p.parse::<u32>().ok())
                .is_none_or(|minor| minor < 4)
        {
            return Err(rejected(
                "supported mechanism requires Podman 5.4..5.x; consult its installed manual before extending support",
            ));
        }
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
        Self::systemctl(&["show-environment".to_owned()])?;
        verify_subordinate_ids(&info, &d.recipe.model_service)?;
        #[cfg(target_os = "linux")]
        if matches!(d.recipe.model_service, ModelService::Owned { .. }) {
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
        let memory = fs::read_to_string("/proc/meminfo")
            .map_err(rejected)?
            .lines()
            .find_map(|line| {
                line.strip_prefix("MemAvailable:")
                    .and_then(|v| v.split_whitespace().next())
                    .and_then(|v| v.parse::<u64>().ok())
            })
            .ok_or_else(|| rejected("available shared RAM could not be observed"))?
            .saturating_mul(1024);
        if d.recipe.memory_bytes > memory {
            return Err(rejected(
                "approved memory exceeds observed available shared RAM; GPU reservation is not additional memory",
            ));
        }
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
            ..
        } = &d.recipe.model_service
        {
            Self::podman(&["image".to_owned(), "inspect".to_owned(), image.clone()])?;
            verify_model(model, model_digest, *model_bytes)?;
            if let crate::InferenceBackend::Vulkan { render_device } = backend {
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
    fn reconcile(
        &self,
        record: &InstallationRecord,
        step: ManagedStep,
    ) -> Result<ManagedObservation, ManagedError> {
        let p = record
            .pending
            .as_ref()
            .ok_or_else(|| rejected("platform effect requires durable intent"))?;
        let setup = &p.change.candidate;
        let d = self.check_owner(setup)?;
        match step {
            ManagedStep::Prerequisites => {
                self.diagnose(setup)?;
            }
            ManagedStep::PrepareStorage => {
                self.storage(setup, false)?;
                crate::worker::verify_setup(self, setup, true)?;
            }
            ManagedStep::StopService => self.stop(record.current.as_ref().unwrap_or(setup))?,
            ManagedStep::Configure => self.configure(record)?,
            ManagedStep::StartService => {
                if matches!(d.recipe.model_service, ModelService::Owned { .. }) {
                    self.verify_unit(setup)?;
                    regular_file(
                        &d.quadlet_directory.join(format!("{}.container", d.unit)),
                        65_536,
                    )?;
                    let container = Self::inspect("container", &d.unit)?;
                    start_owned(setup, container.as_ref(), || {
                        if container.as_ref().is_some_and(|c| {
                            c.pointer("/State/Running").and_then(|v| v.as_bool()) == Some(false)
                        }) {
                            // An interrupted run may leave an owned stopped container. Non-forced
                            // removal refuses if it started meanwhile; never replace a running one.
                            Self::podman(&["rm".to_owned(), d.unit.clone()])?;
                        }
                        Self::systemctl(&["start".to_owned(), format!("{}.service", d.unit)])?;
                        Ok(())
                    })?;
                }
            }
            ManagedStep::Verify => {
                crate::worker::verify_setup(self, setup, false)?;
                let observation = if p.change.running
                    && matches!(d.recipe.model_service, ModelService::Owned { .. })
                {
                    crate::service::verify(self, setup)?
                } else {
                    self.observe(setup)?
                };
                if matches!(d.recipe.model_service, ModelService::Owned { .. })
                    && observation.running != p.change.running
                {
                    return Err(platform_error(
                        "service observation differs from declared desired state",
                    ));
                }
                return Ok(observation);
            }
            ManagedStep::RemoveConfiguration => {
                self.remove_configuration(record.current.as_ref().unwrap_or(setup))?
            }
            ManagedStep::RemoveStorage => self.storage(setup, true)?,
        }
        Ok(ManagedObservation {
            digest: digest(format!(
                "{}:{step:?}:{}",
                setup.ownership, setup.recipe.digest
            )),
            summary: format!(
                "verified {step:?}; attached services and shared prerequisites retained"
            ),
            running: if step == ManagedStep::StartService
                && matches!(d.recipe.model_service, ModelService::Owned { .. })
            {
                crate::service::ready(self, setup)?.running
            } else {
                self.observe(setup)?.running
            },
        })
    }
    fn observe(&self, setup: &ApprovedSetup) -> Result<ManagedObservation, ManagedError> {
        let d = self.check_owner(setup)?;
        self.verify_unit(setup)?;
        let mut running = false;
        if let ModelService::Owned { port, .. } = &d.recipe.model_service {
            match Self::inspect("container", &d.unit)? {
                Some(value) => {
                    Self::verify_labels(&value, setup, Some(&setup.recipe.digest))?;
                    running =
                        value.pointer("/State/Running").and_then(|v| v.as_bool()) == Some(true);
                    if running {
                        crate::worker::verify_container(&value, setup, false)?;
                        let client = reqwest::blocking::Client::builder()
                            .timeout(Duration::from_secs(5))
                            .no_proxy()
                            .redirect(reqwest::redirect::Policy::none())
                            .build()
                            .map_err(platform_error)?;
                        if !client
                            .get(format!("http://127.0.0.1:{port}/health"))
                            .send()
                            .map_err(platform_error)?
                            .status()
                            .is_success()
                        {
                            return Err(platform_error(
                                "owned llama-server health verification failed",
                            ));
                        }
                    }
                }
                None => running = false,
            }
        }
        Ok(ManagedObservation {
            digest: digest(format!(
                "{}:{}:{running}",
                setup.ownership, setup.recipe.digest
            )),
            summary: format!(
                "exact owned configuration observed; declared model service running={running}; host reboot remains a separate qualification"
            ),
            running,
        })
    }
    fn requirements(
        &self,
        setup: &ApprovedSetup,
    ) -> Result<CapabilityExecutionRequirements, ManagedError> {
        let d = self.check_owner(setup)?;
        let mut requirements = CapabilityExecutionRequirements::default();
        // These are administrative permissions. Workers use the separately scoped process
        // capability and receive no host manager-path authority from this declaration.
        requirements.filesystem.push(
            FilesystemScope::from_canonical_host_path(
                &self.config.state_root,
                std::collections::BTreeSet::from([AccessMode::Read, AccessMode::Write]),
            )
            .map_err(rejected)?,
        );
        requirements.filesystem.push(
            FilesystemScope::from_canonical_host_path(
                &self.config.quadlet_directory,
                std::collections::BTreeSet::from([AccessMode::Read, AccessMode::Write]),
            )
            .map_err(rejected)?,
        );
        if let ModelService::Owned { model, port, .. } = &d.recipe.model_service {
            requirements.filesystem.push(
                FilesystemScope::from_canonical_host_path(
                    model,
                    std::collections::BTreeSet::from([AccessMode::Read]),
                )
                .map_err(rejected)?,
            );
            requirements
                .network_profiles
                .insert(NetworkProfileRef::new("managed.service").map_err(rejected)?);
            requirements
                .network_destinations
                .insert(format!("127.0.0.1:{port}"));
        }
        if d.recipe.worker_network == WorkerNetwork::Outbound {
            requirements
                .network_profiles
                .insert(NetworkProfileRef::new("managed.outbound").map_err(rejected)?);
        }
        Ok(requirements)
    }
    fn fence(
        &self,
        setup: &ApprovedSetup,
        usage: &ManagedUse,
    ) -> Result<QuiescenceEvidence, ManagedError> {
        self.check_owner(setup)?;
        let deadline = Instant::now() + Duration::from_secs(50);
        let mut active = self
            .active
            .lock()
            .map_err(|_| platform_error("worker ownership unavailable"))?;
        if let Some(cancel) = active.get(&usage.id) {
            cancel.store(true, Ordering::Release);
        }
        while active.contains_key(&usage.id) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(platform_error(
                    "bounded fencing wait expired; creator has not quiesced",
                ));
            }
            let (next, _) = self
                .quiescent
                .wait_timeout(active, remaining)
                .map_err(|_| platform_error("worker ownership unavailable"))?;
            active = next;
        }
        // The durable Fencing claim prevents any new entry. Do not serialize unrelated creators
        // behind potentially slow engine inspection.
        drop(active);
        if usage
            .binding
            .resources
            .iter()
            .any(|r| r.resource.as_str() == "model")
        {
            let resource = setup
                .resources
                .iter()
                .find(|r| r.name.as_str() == "model")
                .ok_or_else(|| rejected("model identity absent"))?;
            if resource.ownership == ResourceOwnership::Owned {
                self.write_unit(setup, false)?;
                self.stop(setup)?;
            }
            // Removing an attachment cannot stop the remote service. After the local request
            // owner has left, explicit resolution relinquishes only this host's attachment hold.
            return Ok(QuiescenceEvidence {
                physical_identity: resource.identity.clone(),
                observation_digest: digest(format!(
                    "{}:{}:locally-fenced",
                    setup.ownership, resource.identity
                )),
                disrupted: true,
            });
        }
        let name = format!("mdtask-{}", usage.id);
        if let Some(value) = Self::inspect("container", &name)? {
            Self::verify_labels(&value, setup, Some(&setup.recipe.digest))?;
            Self::podman(&["rm".to_owned(), "--force".to_owned(), name.clone()])?;
        }
        if Self::inspect("container", &name)?.is_some() {
            return Err(platform_error("task container remains after fencing"));
        }
        Ok(QuiescenceEvidence {
            physical_identity: name.clone(),
            observation_digest: digest(format!("{}:{name}:absent", setup.ownership)),
            disrupted: true,
        })
    }
}

fn verify_subordinate_ids(
    info: &serde_json::Value,
    service: &ModelService,
) -> Result<(), ManagedError> {
    // The owned model and a worker need separate simultaneous auto user namespaces.
    let required = if matches!(service, ModelService::Owned { .. }) {
        131_072
    } else {
        65_536
    };
    for mapping in ["/host/idMappings/uidmap", "/host/idMappings/gidmap"] {
        if !info
            .pointer(mapping)
            .and_then(|v| v.as_array())
            .is_some_and(|maps| {
                maps.iter().any(|m| {
                    m.get("size")
                        .and_then(|n| n.as_u64())
                        .is_some_and(|size| size >= required)
                })
            })
        {
            return Err(rejected(
                "rootless setup requires 65536 subordinate UIDs/GIDs per simultaneous worker and owned service",
            ));
        }
    }
    Ok(())
}

fn start_owned(
    setup: &ApprovedSetup,
    container: Option<&serde_json::Value>,
    start: impl FnOnce() -> Result<(), ManagedError>,
) -> Result<(), ManagedError> {
    if let Some(container) = container {
        LinuxManagedPlatform::verify_labels(container, setup, Some(&setup.recipe.digest))?;
    }
    // Quadlet's default run command uses --replace. Refuse a foreign collision before invoking
    // the supervisor; the generated --replace=false also protects the gap after this inspection.
    start()
}

fn owner_identity(root: &Path) -> Result<String, ManagedError> {
    #[cfg(target_os = "linux")]
    {
        let uid = rustix::process::getuid().as_raw();
        if uid == 0 {
            return Err(rejected("managed setup refuses rootful execution"));
        }
        Ok(digest(format!(
            "{}:{uid}:{}",
            fs::read_to_string("/etc/machine-id")
                .map_err(rejected)?
                .trim(),
            fs::canonicalize(root).map_err(rejected)?.display()
        )))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = root;
        Err(rejected("managed Linux setup is unsupported"))
    }
}
pub(crate) fn private_directory(path: &Path) -> Result<(), ManagedError> {
    let canonical = fs::canonicalize(path).map_err(|_| {
        rejected("bootstrap must create the declared private manager directories first")
    })?;
    if canonical != path || !crate::recipe::safe_absolute(path) {
        return Err(rejected(
            "manager paths must be canonical and cannot contain symlinks or interpolation",
        ));
    }
    let metadata = fs::symlink_metadata(path).map_err(rejected)?;
    if !metadata.is_dir() {
        return Err(rejected("manager path must be a directory"));
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.uid() != rustix::process::getuid().as_raw()
            || metadata.permissions().mode() & 0o077 != 0
        {
            return Err(rejected(
                "manager directories must be owned by the manager UID with mode 0700",
            ));
        }
    }
    Ok(())
}
pub(crate) fn regular_file(path: &Path, max: u64) -> Result<(), ManagedError> {
    let m = fs::symlink_metadata(path).map_err(rejected)?;
    if !m.is_file() || m.len() > max || fs::canonicalize(path).map_err(rejected)? != path {
        return Err(rejected(
            "expected bounded canonical regular file without symlinks",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if m.nlink() != 1 || m.permissions().mode() & 0o022 != 0 {
            return Err(rejected(
                "manager input cannot be hardlinked or writable by another identity",
            ));
        }
    }
    Ok(())
}
fn verify_model(path: &Path, expected: &str, size: u64) -> Result<(), ManagedError> {
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
