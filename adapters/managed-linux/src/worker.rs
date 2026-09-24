use crate::{
    LinuxManagedPlatform, WorkerNetwork, command, digest, platform_error, rejected, units,
};
use milkdrift_authority::{CapabilityExecutionRequirements, NetworkProfileRef};
use milkdrift_capability::managed::{
    MANAGED_BINDING_EXTENSION, ManagedBinding, ManagedName, ResourceRequirement,
};
use milkdrift_capability::{
    AdmissionBound, AdmissionConstraints, AdmissionUnit, BoundedJson, CancellationAcknowledgement,
    CancellationBehavior, CancellationRequest, CapabilityCategory, CapabilityDescriptor,
    CapabilityId, CapabilityObservation, DescriptorBuilder, ErrorClass, ExecutionTrustClass,
    ExtensionKey, IdempotencyBehavior, InvocationAdmissionEnvelope, InvocationEvent,
    InvocationEventKind, InvocationFailure, InvocationTerminal, InvocationValueReference, Locality,
    OperationContract, OperationId, SchemaContract, SchemaId, SideEffectClass, StreamingMode,
    TerminalStatus,
};
use milkdrift_capability_host::{
    AdapterError, AdapterInvocation, AdapterReporter, CapabilityAdapter, InvocationDataAccess,
    MaterializationLimits, managed::ManagedError,
};
use milkdrift_persistence::managed::{
    ApprovedSetup, ManagedResourceStore, QuiescenceEvidence, managed_use_id,
};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

/// One verified installed worker generation. Systemd owns persistent services; this adapter owns
/// every temporary task container through output capture, cancellation and verified removal.
pub struct ManagedWorkerAdapter {
    platform: Arc<LinuxManagedPlatform>,
    store: Arc<dyn ManagedResourceStore>,
    setup: ApprovedSetup,
    data: Arc<dyn InvocationDataAccess>,
    active: Arc<Mutex<BTreeSet<String>>>,
}
impl ManagedWorkerAdapter {
    /// Construct only from a verified resource inventory in the daemon composition.
    pub fn new(
        platform: Arc<LinuxManagedPlatform>,
        store: Arc<dyn ManagedResourceStore>,
        setup: ApprovedSetup,
        data: Arc<dyn InvocationDataAccess>,
    ) -> Result<Self, ManagedError> {
        platform.check_owner(&setup)?;
        Ok(Self {
            platform,
            store,
            setup,
            data,
            active: Arc::new(Mutex::new(BTreeSet::new())),
        })
    }
}

pub(crate) fn descriptor(setup: &ApprovedSetup) -> Result<CapabilityDescriptor, ManagedError> {
    let d = units::deployment(setup)?;
    let binding = ManagedBinding {
        installation: d.installation.clone(),
        generation: d.generation,
        recipe_digest: setup.recipe.digest.clone(),
        resources: vec![ResourceRequirement {
            resource: ManagedName::new("working").map_err(rejected)?,
            mutation: true,
        }],
    };
    let schema = SchemaContract::new(
        SchemaId::new("milkdrift.managed.worker").map_err(rejected)?,
        2,
        BoundedJson::new(serde_json::json!({"type":"object"})).map_err(rejected)?,
    )
    .map_err(rejected)?;
    let contract = OperationContract::new(
        schema.clone(),
        schema,
        BTreeSet::from([StreamingMode::None]),
        CancellationBehavior::BestEffort,
        IdempotencyBehavior::Unsupported,
        SideEffectClass::NonIdempotentWrite,
        BTreeMap::new(),
    )
    .map_err(rejected)?;
    DescriptorBuilder::new(
        CapabilityId::new(format!("managed.{}.worker", d.installation)).map_err(rejected)?,
        d.generation,
        CapabilityCategory::Process,
        AdmissionConstraints::new(1, 0).map_err(rejected)?,
        Locality::Local,
    )
    .execution_trust(ExecutionTrustClass::SandboxedProcess)
    .operations(BTreeMap::from([(
        OperationId::new("workspace.execute").map_err(rejected)?,
        contract,
    )]))
    .extensions(BTreeMap::from([(
        ExtensionKey::new(MANAGED_BINDING_EXTENSION).map_err(rejected)?,
        BoundedJson::new(serde_json::to_value(binding).map_err(rejected)?).map_err(rejected)?,
    )]))
    .build()
    .map_err(rejected)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkerRequest {
    argv: Vec<String>,
    #[serde(default)]
    stdout_artifact: bool,
}
fn parse(invocation: &AdapterInvocation<'_>) -> Result<WorkerRequest, AdapterError> {
    let input = invocation
        .request()
        .inputs()
        .iter()
        .find(|i| i.name() == "command")
        .ok_or_else(|| AdapterError::rejected("command input is required"))?;
    let InvocationValueReference::Inline { value } = input.value() else {
        return Err(AdapterError::rejected(
            "command must be bounded inline JSON",
        ));
    };
    let request: WorkerRequest =
        serde_json::from_value(value.value().clone()).map_err(adapter_failure)?;
    if request.argv.is_empty()
        || request.argv.iter().any(|a| a.contains('\0'))
        || !request.argv[0].starts_with('/')
    {
        return Err(AdapterError::rejected(
            "worker command needs an absolute container executable and NUL-free arguments",
        ));
    }
    Ok(request)
}
fn adapter_failure(e: impl std::fmt::Display) -> AdapterError {
    AdapterError::rejected(e.to_string())
}

fn result_document(
    stdout: &str,
    stderr: &str,
    success: bool,
    exit_code: Option<i32>,
) -> serde_json::Value {
    serde_json::json!({"stdout":stdout,"stderr":stderr,"success":success,"exit_code":exit_code,"mutable_tooling":"workspace experiments require explicit image promotion"})
}

pub(crate) fn output_artifact_limit(raw: u64) -> Result<u64, ManagedError> {
    // Each input byte needs at most six JSON bytes, including invalid UTF-8 replacement. False
    // and the smallest signed exit code give the longest fixed fields in this result document.
    let overhead = serde_json::to_vec(&result_document("", "", false, Some(i32::MIN)))
        .map_err(rejected)?
        .len() as u64;
    raw.checked_mul(6)
        .and_then(|n| n.checked_add(overhead))
        .filter(|n| *n <= isize::MAX as u64)
        .ok_or_else(|| rejected("output_bytes cannot fit its encoded result allocation"))
}

impl CapabilityAdapter for ManagedWorkerAdapter {
    fn accepts_direct_inputs(&self) -> bool {
        true
    }
    fn admission_envelope(
        &self,
        invocation: &AdapterInvocation<'_>,
    ) -> Result<InvocationAdmissionEnvelope, AdapterError> {
        let request = parse(invocation)?;
        let d = units::deployment(&self.setup).map_err(adapter_failure)?;
        Ok(InvocationAdmissionEnvelope::new(
            AdmissionUnit::Unknown,
            AdmissionBound::NotApplicable,
            AdmissionBound::NotApplicable,
            AdmissionBound::Bounded(
                output_artifact_limit(d.recipe.output_bytes)
                    .map_err(adapter_failure)?
                    .checked_add(if request.stdout_artifact {
                        d.recipe.output_bytes
                    } else {
                        0
                    })
                    .ok_or_else(|| {
                        AdapterError::rejected("combined worker artifact bound overflow")
                    })?,
            ),
            AdmissionBound::NotApplicable,
        ))
    }
    fn authority_requirements(&self) -> CapabilityExecutionRequirements {
        let mut requirements = CapabilityExecutionRequirements::default();
        if units::deployment(&self.setup)
            .is_ok_and(|d| d.recipe.worker_network == WorkerNetwork::Outbound)
            && let Ok(profile) = NetworkProfileRef::new("managed.outbound")
        {
            requirements.network_profiles.insert(profile);
        }
        requirements
    }
    fn start(&self) -> Result<(), AdapterError> {
        self.platform
            .check_owner(&self.setup)
            .map(|_| ())
            .map_err(adapter_failure)
    }
    fn execute(
        &self,
        invocation: &AdapterInvocation<'_>,
        reporter: &dyn AdapterReporter,
    ) -> Result<(), AdapterError> {
        let request = parse(invocation)?;
        let d = self
            .platform
            .check_owner(&self.setup)
            .map_err(adapter_failure)?;
        let id = managed_use_id(invocation.request().invocation());
        // Register the creator before recording physical intent. A resolver first cancels and
        // joins this ownership scope; it cannot release an absence check ahead of late creation.
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut active = self
                .platform
                .active
                .lock()
                .map_err(|_| AdapterError::unavailable("worker ownership unavailable"))?;
            if active.len() >= milkdrift_capability::managed::MAX_MANAGED_USES
                || active.contains_key(&id)
            {
                return Err(AdapterError::rejected("duplicate or excess active worker"));
            }
            active.insert(id.clone(), cancel.clone());
            self.active
                .lock()
                .map_err(|_| AdapterError::unavailable("worker ownership unavailable"))?
                .insert(id.clone());
        }
        let mut task = OwnedTask {
            platform: self.platform.clone(),
            setup: self.setup.clone(),
            id: id.clone(),
            name: format!("mdtask-{id}"),
            completed_cleanup: false,
            entered: false,
            active: self.active.clone(),
        };
        let usage = self
            .store
            .managed_use(&id)
            .map_err(adapter_failure)?
            .ok_or_else(|| AdapterError::rejected("worker lacks durable generation hold"))?;
        self.store
            .enter_managed_use(&id, usage.claim, &task.name)
            .map_err(adapter_failure)?;
        task.entered = true;
        if cancel.load(Ordering::Acquire) {
            return Err(AdapterError::external_failure(
                "worker cancelled before platform creation; resource use remains retained",
            ));
        }
        let output = run_task(
            &self.setup,
            &task.name,
            &request.argv,
            Some(&cancel),
            Duration::from_millis(d.recipe.task_timeout_ms),
            d.recipe.output_bytes as usize,
        );
        let cleanup = cleanup_task(&self.setup, &task.name);
        if let Err(error) = cleanup {
            return Err(AdapterError::external_failure(error.to_string()));
        }
        task.completed_cleanup = true;
        self.store
            .quiesce_managed_use(
                &id,
                usage.claim,
                &QuiescenceEvidence::PhysicalStop {
                    physical_identity: task.name.clone(),
                    observation_digest: digest(format!(
                        "{}:{}:absent",
                        self.setup.ownership, task.name
                    )),
                    disrupted: cancel.load(Ordering::Acquire),
                },
            )
            .map_err(adapter_failure)?;
        let context = invocation.context().ok_or_else(|| {
            AdapterError::external_failure("durable publication context is absent")
        })?;
        let mut sequence = 1;
        let (status, failure) = match output {
            Ok(output) => {
                let bytes = serde_json::to_vec(&result_document(
                    &String::from_utf8_lossy(&output.stdout),
                    &String::from_utf8_lossy(&output.stderr),
                    output.success,
                    output.exit_code,
                ))
                .map_err(adapter_failure)?;
                // JSON escaping can expand raw output; reserve and enforce the encoded maximum.
                let limit =
                    output_artifact_limit(d.recipe.output_bytes).map_err(adapter_failure)?;
                let artifact = self
                    .data
                    .publish_bytes(
                        context,
                        invocation.request(),
                        "worker_result",
                        "application/vnd.milkdrift.worker+json",
                        &bytes,
                        MaterializationLimits {
                            max_files: 1,
                            max_file_bytes: limit,
                            max_total_bytes: limit,
                            max_path_bytes: 256,
                            max_directory_depth: 8,
                            chunk_bytes: 262_144,
                        },
                    )
                    .map_err(adapter_failure)?;
                reporter.invocation(
                    InvocationEvent::new(
                        invocation.request().invocation().clone(),
                        sequence,
                        InvocationEventKind::Output {
                            name: "worker_result".to_owned(),
                            reference: artifact,
                        },
                    )
                    .map_err(adapter_failure)?,
                )?;
                sequence += 1;
                if output.success && request.stdout_artifact {
                    let artifact = self
                        .data
                        .publish_bytes(
                            context,
                            invocation.request(),
                            "stdout",
                            "application/octet-stream",
                            &output.stdout,
                            MaterializationLimits {
                                max_files: 1,
                                max_file_bytes: d.recipe.output_bytes,
                                max_total_bytes: d.recipe.output_bytes,
                                max_path_bytes: 256,
                                max_directory_depth: 8,
                                chunk_bytes: 262_144,
                            },
                        )
                        .map_err(adapter_failure)?;
                    reporter.invocation(
                        InvocationEvent::new(
                            invocation.request().invocation().clone(),
                            sequence,
                            InvocationEventKind::Output {
                                name: "stdout".to_owned(),
                                reference: artifact,
                            },
                        )
                        .map_err(adapter_failure)?,
                    )?;
                    sequence += 1;
                }
                if output.success {
                    (TerminalStatus::Success, None)
                } else {
                    (
                        TerminalStatus::Failure,
                        Some(
                            InvocationFailure::new(
                                ErrorClass::Provider,
                                false,
                                "worker_exit",
                                "contained worker returned nonzero exit",
                                None,
                            )
                            .map_err(adapter_failure)?,
                        ),
                    )
                }
            }
            Err(_) if cancel.load(Ordering::Acquire) => (TerminalStatus::Cancelled, None),
            Err(error) => (
                TerminalStatus::Failure,
                Some(
                    InvocationFailure::new(
                        ErrorClass::Provider,
                        false,
                        "worker_stopped",
                        milkdrift_contracts::truncate_utf8(&error.to_string(), 256),
                        None,
                    )
                    .map_err(adapter_failure)?,
                ),
            ),
        };
        reporter.invocation(
            InvocationEvent::new(
                invocation.request().invocation().clone(),
                sequence,
                InvocationEventKind::Terminal {
                    terminal: InvocationTerminal::new(
                        status,
                        Vec::new(),
                        failure,
                        None,
                        SideEffectClass::NonIdempotentWrite,
                    )
                    .map_err(adapter_failure)?,
                },
            )
            .map_err(adapter_failure)?,
        )
    }
    fn cancel(
        &self,
        request: &CancellationRequest,
    ) -> Result<CancellationAcknowledgement, AdapterError> {
        let active = self
            .platform
            .active
            .lock()
            .map_err(|_| AdapterError::unavailable("worker ownership unavailable"))?;
        let found = active.get(&managed_use_id(request.invocation()));
        if let Some(flag) = found {
            flag.store(true, Ordering::Release);
        }
        CancellationAcknowledgement::new(request.invocation().clone(), request.request_sequence(), found.is_some(), false, Some("cancellation requested; container removal and durable stop evidence remain separate".to_owned())).map_err(adapter_failure)
    }
    fn health(&self, now: u64) -> Result<CapabilityObservation, AdapterError> {
        let d = units::deployment(&self.setup).map_err(adapter_failure)?;
        let available = self
            .store
            .managed_installation(&d.installation)
            .map_err(adapter_failure)?
            .is_some_and(|r| r.admission_open && r.generation == d.generation);
        CapabilityObservation::new(
            descriptor(&self.setup)
                .map_err(adapter_failure)?
                .identity()
                .clone(),
            now,
            available,
            0,
            "managed worker generation admission observed",
        )
        .map_err(adapter_failure)
    }
    fn begin_drain(&self) -> Result<(), AdapterError> {
        Ok(())
    }
    fn shutdown(&self) -> Result<(), AdapterError> {
        let active = self
            .platform
            .active
            .lock()
            .map_err(|_| AdapterError::unavailable("worker ownership unavailable"))?;
        let mine = self
            .active
            .lock()
            .map_err(|_| AdapterError::unavailable("worker ownership unavailable"))?;
        for id in mine.iter() {
            if let Some(flag) = active.get(id) {
                flag.store(true, Ordering::Release);
            }
        }
        // Host generation permits retain this adapter until execution cleanup finishes. Persistent
        // services belong to systemd and are deliberately unaffected by host shutdown.
        Ok(())
    }
}

struct OwnedTask {
    platform: Arc<LinuxManagedPlatform>,
    setup: ApprovedSetup,
    id: String,
    name: String,
    completed_cleanup: bool,
    entered: bool,
    active: Arc<Mutex<BTreeSet<String>>>,
}
impl Drop for OwnedTask {
    fn drop(&mut self) {
        if self.entered && !self.completed_cleanup {
            let _ = cleanup_task(&self.setup, &self.name);
        }
        if let Ok(mut active) = self.platform.active.lock() {
            active.remove(&self.id);
            if let Ok(mut mine) = self.active.lock() {
                mine.remove(&self.id);
            }
            self.platform.quiescent.notify_all();
        }
    }
}

fn task_arguments(
    setup: &ApprovedSetup,
    name: &str,
    argv: &[String],
) -> Result<Vec<String>, ManagedError> {
    let d = units::deployment(setup)?;
    let volume = setup
        .resources
        .iter()
        .find(|r| r.name.as_str() == "working")
        .ok_or_else(|| rejected("working volume missing"))?;
    let mut args = vec![
        "create".to_owned(),
        "--name".to_owned(),
        name.to_owned(),
        "--pull=never".to_owned(),
        "--label".to_owned(),
        format!("org.milkdrift.owner={}", setup.ownership),
        "--label".to_owned(),
        format!("org.milkdrift.platform={}", setup.platform_owner),
        "--label".to_owned(),
        format!("org.milkdrift.recipe={}", setup.recipe.digest),
        "--read-only".to_owned(),
        "--read-only-tmpfs=false".to_owned(),
        "--cap-drop=all".to_owned(),
        "--security-opt=no-new-privileges".to_owned(),
        format!("--userns=auto:size={}", crate::recipe::PRIVATE_USER_IDS),
        format!("--memory={}", d.recipe.worker_limits.memory_bytes),
        format!("--memory-swap={}", d.recipe.worker_limits.memory_bytes),
        format!("--cpus={}", d.recipe.worker_limits.cpus()),
        format!("--pids-limit={}", d.recipe.worker_limits.pids),
        "--network".to_owned(),
        match d.recipe.worker_network {
            WorkerNetwork::None => "none",
            WorkerNetwork::Outbound => "pasta:--no-map-gw",
        }
        .to_owned(),
        "--volume".to_owned(),
        format!("{}:/workspace:rw,U", volume.identity),
        "--workdir=/workspace".to_owned(),
        "--env=HOME=/workspace/home".to_owned(),
        format!("--tmpfs={}", d.recipe.worker_limits.temporary_mount()),
        "--entrypoint".to_owned(),
        argv.first()
            .cloned()
            .ok_or_else(|| rejected("empty command"))?,
        d.recipe.worker_image.clone(),
    ];
    args.extend_from_slice(&argv[1..]);
    Ok(args)
}
fn run_task(
    setup: &ApprovedSetup,
    name: &str,
    argv: &[String],
    cancel: Option<&AtomicBool>,
    timeout: Duration,
    output_limit: usize,
) -> Result<command::CommandOutput, ManagedError> {
    let d = units::deployment(setup)?;
    let working = format!("{}-working", d.volume_prefix);
    let volume = LinuxManagedPlatform::inspect("volume", &working)?.ok_or_else(|| {
        platform_error("owned working volume is missing; refusing implicit replacement")
    })?;
    LinuxManagedPlatform::verify_labels(&volume, setup, None)?;
    if LinuxManagedPlatform::inspect("container", name)?.is_some() {
        return Err(platform_error(
            "task identity already exists; recovery must inspect it before reuse",
        ));
    }
    LinuxManagedPlatform::podman(&task_arguments(setup, name, argv)?)?;
    let value = LinuxManagedPlatform::inspect("container", name)?
        .ok_or_else(|| platform_error("created task identity missing"))?;
    LinuxManagedPlatform::verify_labels(&value, setup, Some(&setup.recipe.digest))?;
    verify_container(&value, setup, true)?;
    command::run(
        Path::new("/usr/bin/podman"),
        &["start".to_owned(), "--attach".to_owned(), name.to_owned()],
        &[],
        timeout,
        output_limit,
        cancel,
    )
}
fn cleanup_task(setup: &ApprovedSetup, name: &str) -> Result<(), ManagedError> {
    if let Some(value) = LinuxManagedPlatform::inspect("container", name)? {
        LinuxManagedPlatform::verify_labels(&value, setup, Some(&setup.recipe.digest))?;
        LinuxManagedPlatform::podman(&["rm".to_owned(), "--force".to_owned(), name.to_owned()])?;
    }
    if LinuxManagedPlatform::inspect("container", name)?.is_some() {
        return Err(platform_error(
            "task cleanup did not prove container absence",
        ));
    }
    Ok(())
}

pub(crate) fn verify_container(
    value: &serde_json::Value,
    setup: &ApprovedSetup,
    worker: bool,
) -> Result<(), ManagedError> {
    let d = units::deployment(setup)?;
    let limits = if worker {
        &d.recipe.worker_limits
    } else {
        match &d.recipe.model_service {
            crate::ModelService::Owned { limits, .. } => limits,
            _ => return Err(platform_error("service inspection requires an owned model")),
        }
    };
    let image = if worker {
        &d.recipe.worker_image
    } else {
        match &d.recipe.model_service {
            crate::ModelService::Owned { image, .. } => image,
            _ => return Err(platform_error("service inspection requires an owned model")),
        }
    };
    LinuxManagedPlatform::verify_image(value, image)?;
    let host = verify_limits(value, limits)?;
    if !worker {
        let crate::ModelService::Owned { port, .. } = d.recipe.model_service else {
            return Err(platform_error("owned service configuration missing"));
        };
        let expected_ports =
            serde_json::json!({"8080/tcp":[{"HostIp":"127.0.0.1","HostPort":port.to_string()}]});
        if host.get("PortBindings") != Some(&expected_ports) {
            return Err(platform_error(
                "service exposure differs from its single approved loopback port",
            ));
        }
    }
    let mounts = value
        .get("Mounts")
        .and_then(|v| v.as_array())
        .ok_or_else(|| platform_error("actual mounts missing"))?;
    let expected_destination = if worker {
        "/workspace"
    } else {
        "/models/model.gguf"
    };
    if mounts
        .iter()
        .filter(|m| m.get("Destination").and_then(|v| v.as_str()) == Some(expected_destination))
        .count()
        != 1
    {
        return Err(platform_error(
            "exact required resource mount is absent or duplicated",
        ));
    }
    if worker
        && d.recipe.worker_network == WorkerNetwork::None
        && host.get("NetworkMode").and_then(|v| v.as_str()) != Some("none")
    {
        return Err(platform_error(
            "offline worker has an unexpected network attachment",
        ));
    }
    if host.get("MemorySwap").and_then(|v| v.as_u64()) != Some(limits.memory_bytes) {
        return Err(platform_error(
            "swap limit differs from the approved memory budget",
        ));
    }
    for m in mounts {
        let dest = m.get("Destination").and_then(|v| v.as_str()).unwrap_or("");
        if dest == "/tmp" && m.get("Type").and_then(|v| v.as_str()) == Some("tmpfs") {
            continue;
        }
        if worker {
            if dest != "/workspace"
                || m.get("Name").and_then(|v| v.as_str())
                    != Some(format!("{}-working", d.volume_prefix).as_str())
                || m.get("RW").and_then(|v| v.as_bool()) != Some(true)
            {
                return Err(platform_error("worker has an unexpected host mount"));
            }
        } else if dest != "/models/model.gguf"
            || m.get("RW").and_then(|v| v.as_bool()) != Some(false)
        {
            return Err(platform_error(
                "service has an unexpected writable or host mount",
            ));
        }
        if !worker
            && let crate::ModelService::Owned { model, .. } = &d.recipe.model_service
            && m.get("Source").and_then(|v| v.as_str()) != model.to_str()
        {
            return Err(platform_error(
                "service model mount is not the exact approved shared input",
            ));
        }
    }
    let devices = host.get("Devices").and_then(|v| v.as_array());
    let expected_device = match (&d.recipe.model_service, worker) {
        (
            crate::ModelService::Owned {
                backend: crate::InferenceBackend::Vulkan { render_device, .. },
                ..
            },
            false,
        ) => render_device.to_str(),
        _ => None,
    };
    if let Some(expected) = expected_device {
        if devices.is_none_or(|devices| {
            devices.len() != 1
                || devices[0].get("PathOnHost").and_then(|v| v.as_str()) != Some(expected)
                || devices[0].get("PathInContainer").and_then(|v| v.as_str()) != Some(expected)
        }) {
            return Err(platform_error(
                "service does not have exactly its approved render device",
            ));
        }
    } else if devices.is_some_and(|d| !d.is_empty()) {
        return Err(platform_error("container has unapproved devices"));
    }
    // Podman serializes an empty capability set as either null or []; absence is not evidence.
    if !value
        .get("EffectiveCaps")
        .is_some_and(|caps| caps.is_null() || caps.as_array().is_some_and(Vec::is_empty))
    {
        return Err(platform_error(
            "effective container capabilities are not empty",
        ));
    }
    Ok(())
}

// Shared observed kernel contract for worker, model and protected application containers.
pub(crate) fn verify_limits<'a>(
    value: &'a serde_json::Value,
    limits: &crate::ContainerLimits,
) -> Result<&'a serde_json::Value, ManagedError> {
    let host = value
        .get("HostConfig")
        .ok_or_else(|| platform_error("container enforcement inspection missing"))?;
    let temporary = host.get("Tmpfs").and_then(|v| v.as_object());
    let options = temporary
        .and_then(|v| v.get("/tmp"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .split(',')
        .collect::<BTreeSet<_>>();
    let size = format!("size={}", limits.temporary_bytes);
    if temporary.is_none_or(|v| v.len() != 1)
        || !["rw", "nodev", "nosuid", size.as_str()]
            .into_iter()
            .all(|v| options.contains(v))
    {
        return Err(platform_error(
            "actual temporary filesystem differs from its independent configured limit",
        ));
    }
    let has_no_new_privileges = host
        .get("SecurityOpt")
        .and_then(|v| v.as_array())
        .is_some_and(|a| {
            a.iter().any(|v| {
                v.as_str()
                    .is_some_and(|s| s.starts_with("no-new-privileges"))
            })
        });
    if host.get("Privileged").and_then(|v| v.as_bool()) != Some(false)
        || host.get("ReadonlyRootfs").and_then(|v| v.as_bool()) != Some(true)
        || host.get("Memory").and_then(|v| v.as_u64()) != Some(limits.memory_bytes)
        || host.get("PidsLimit").and_then(|v| v.as_u64()) != Some(u64::from(limits.pids))
        || host
            .get("CpuQuota")
            .and_then(|v| v.as_u64())
            .zip(host.get("CpuPeriod").and_then(|v| v.as_u64()))
            .is_none_or(|(quota, period)| {
                quota.saturating_mul(100) != period.saturating_mul(u64::from(limits.cpu_percent))
            })
        || !has_no_new_privileges
        || host
            .get("NetworkMode")
            .and_then(|v| v.as_str())
            .is_none_or(|n| n == "host")
        || !private_id_mappings(host.get("IDMappings"))
    {
        return Err(platform_error(
            "actual container namespace/resource enforcement differs from approved profile",
        ));
    }
    if host.get("MemorySwap").and_then(|v| v.as_u64()) != Some(limits.memory_bytes)
        || !value
            .get("EffectiveCaps")
            .is_some_and(|v| v.is_null() || v.as_array().is_some_and(Vec::is_empty))
    {
        return Err(platform_error(
            "actual swap or effective capabilities differ from the protected profile",
        ));
    }
    Ok(host)
}

fn private_id_mappings(value: Option<&serde_json::Value>) -> bool {
    // Inspect reports mappings in the rootless parent namespace. Offset zero is the manager's
    // identity; every container ID must instead map into subordinate IDs. UsernsMode is empty
    // in Podman 6 even for --userns=auto, so verify the realized maps rather than that hint.
    ["UidMap", "GidMap"].into_iter().all(|key| {
        value
            .and_then(|v| v.get(key))
            .and_then(|v| v.as_array())
            .is_some_and(|ranges| {
                let mut next = 0_u64;
                !ranges.is_empty()
                    && ranges.iter().all(|range| {
                        let Some(range) = range.as_str() else {
                            return false;
                        };
                        let parts: Vec<_> = range.split(':').map(str::parse::<u64>).collect();
                        let [Ok(container), Ok(host), Ok(size)] = parts.as_slice() else {
                            return false;
                        };
                        if *container != next
                            || *host == 0
                            || *size == 0
                            || host.checked_add(*size).is_none()
                        {
                            return false;
                        }
                        let Some(end) = next.checked_add(*size) else {
                            return false;
                        };
                        next = end;
                        true
                    })
                    && next >= crate::recipe::PRIVATE_USER_IDS
            })
    })
}

#[cfg(test)]
mod tests;

pub(crate) fn verify_setup(
    platform: &LinuxManagedPlatform,
    setup: &ApprovedSetup,
    initialize: bool,
) -> Result<(), ManagedError> {
    let d = platform.check_owner(setup)?;
    let name = format!("{}probe{}", d.volume_prefix, d.generation);
    if let Some(value) = LinuxManagedPlatform::inspect("container", &name)? {
        LinuxManagedPlatform::verify_labels(&value, setup, Some(&setup.recipe.digest))?;
        if value.pointer("/State/Running").and_then(|v| v.as_bool()) == Some(true) {
            return Err(platform_error(
                "interrupted verification probe is still running",
            ));
        }
        cleanup_task(setup, &name)?;
    }
    // Platform setup creates only the writable home. Applications initialize their own content
    // through ordinary authorized worker commands; reapply never copies an embedded brief.
    let initialize_script = if initialize {
        "mkdir -p /workspace/home;"
    } else {
        ""
    };
    let network_probe = if d.recipe.worker_network == WorkerNetwork::None {
        "test $(wc -l < /proc/net/route) -eq 1;"
    } else {
        ""
    };
    let enforcement = format!(
        "test $(cat /sys/fs/cgroup/memory.max) -eq {}; test $(cat /sys/fs/cgroup/memory.swap.max) -eq 0; test $(cat /sys/fs/cgroup/pids.max) -eq {}; set -- $(cat /sys/fs/cgroup/cpu.max); test $((100 * $1)) -eq $(({} * $2)); grep -Eq '^CapEff:[[:space:]]+0+$' /proc/self/status; grep -Eq '^NoNewPrivs:[[:space:]]+1$' /proc/self/status; grep -Eq '^Seccomp:[[:space:]]+2$' /proc/self/status; if touch /etc/milkdrift-worker-denial 2>/dev/null; then exit 70; fi; {network_probe}",
        d.recipe.worker_limits.memory_bytes,
        d.recipe.worker_limits.pids,
        d.recipe.worker_limits.cpu_percent
    );
    let runtime = std::env::var("XDG_RUNTIME_DIR").map_err(rejected)?;
    if !crate::recipe::safe_absolute(Path::new(&runtime)) {
        return Err(rejected(
            "systemd runtime directory must be a safe absolute path",
        ));
    }
    let script = format!(
        "set -eux; {initialize_script} {enforcement} test -w /workspace; test ! -e /run/podman/podman.sock; test ! -e {runtime}/podman/podman.sock; test ! -e {}; test ! -e {}; test ! -e {}; test -r /sys/fs/cgroup/memory.max; test -r /sys/fs/cgroup/pids.max; test -r /sys/fs/cgroup/cpu.max; printf 'managed-protection-ok\\n'",
        d.manager_root.display(),
        d.quadlet_directory.display(),
        d.systemd_directory.display()
    );
    let result = run_task(
        setup,
        &name,
        &["/bin/sh".to_owned(), "-c".to_owned(), script],
        None,
        command::HELPER_TIMEOUT,
        command::HELPER_OUTPUT_BYTES,
    );
    let cleanup = cleanup_task(setup, &name);
    cleanup?;
    let output = result?;
    if !output.success || !String::from_utf8_lossy(&output.stdout).contains("managed-protection-ok")
    {
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        let tail: String = diagnostic
            .chars()
            .rev()
            .take(300)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        return Err(platform_error(format!(
            "worker protection probe failed (exit {:?}); generation remains unpublished: {}",
            output.exit_code, tail
        )));
    }
    Ok(())
}
