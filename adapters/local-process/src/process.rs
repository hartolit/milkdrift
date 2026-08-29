use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{OsStr, OsString},
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, Ordering},
        mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use milkdrift_authority::{
    AccessMode, AuthorityBudget, CapabilityExecutionRequirements, FilesystemScope, SensitiveSecret,
};
use milkdrift_capability::{
    CancellationAcknowledgement, CancellationRequest, CapabilityObservation, ErrorClass,
    InvocationEvent, InvocationEventKind, InvocationFailure, InvocationId, InvocationTerminal,
    InvocationValueReference, SideEffectClass, TerminalStatus, UsageObservation,
};
use milkdrift_capability_host::{
    AdapterError, AdapterInvocation, AdapterReporter, CapabilityAdapter, InputMaterialization,
    InvocationDataAccess, MaterializedExecution, SecretResolver,
};

use crate::config::{
    FilesystemAccessMode, OverflowAction, ProcessProfile, ProcessProfileError, StdinMode,
    SubstitutionSource, WorkingDirectoryMode, placeholders,
};

const STREAM_READ_BYTES: usize = 8 * 1024;
const STREAM_CHANNEL_MESSAGES: usize = 16;
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Production local-process adapter for one immutable validated profile generation.
pub struct LocalProcessAdapter {
    profile: ProcessProfile,
    executable: PathBuf,
    executable_roots: Vec<PathBuf>,
    writable_roots: Vec<PathBuf>,
    authority_requirements: CapabilityExecutionRequirements,
    data: Arc<dyn InvocationDataAccess>,
    secrets: Arc<dyn SecretResolver>,
    lifecycle: AtomicU8,
    active: Arc<Mutex<BTreeMap<InvocationId, Arc<ProcessControl>>>>,
}

impl LocalProcessAdapter {
    /// Canonicalizes configured host paths and creates one adapter generation.
    pub fn new(
        profile: ProcessProfile,
        data: Arc<dyn InvocationDataAccess>,
        secrets: Arc<dyn SecretResolver>,
    ) -> Result<Self, ProcessProfileError> {
        let executable = profile.executable.canonicalize().map_err(|error| {
            ProcessProfileError::Invalid(format!(
                "configured executable cannot be canonicalized: {:?}",
                error.kind()
            ))
        })?;
        if !executable.is_file() {
            return Err(ProcessProfileError::Invalid(
                "configured executable is not a regular file".to_owned(),
            ));
        }
        let mut executable_roots = Vec::new();
        let mut writable_roots = Vec::new();
        let mut authority_filesystem = Vec::new();
        for configured in &profile.filesystem_roots {
            let root = configured.path.canonicalize().map_err(|error| {
                ProcessProfileError::Invalid(format!(
                    "configured filesystem root cannot be canonicalized: {:?}",
                    error.kind()
                ))
            })?;
            if !root.is_dir() {
                return Err(ProcessProfileError::Invalid(
                    "configured filesystem root is not a directory".to_owned(),
                ));
            }
            let access = match configured.access {
                FilesystemAccessMode::Execute => {
                    executable_roots.push(root.clone());
                    BTreeSet::from([AccessMode::Execute])
                }
                FilesystemAccessMode::ReadWrite => {
                    writable_roots.push(root.clone());
                    BTreeSet::from([AccessMode::Read, AccessMode::Write])
                }
                FilesystemAccessMode::ReadOnly => BTreeSet::from([AccessMode::Read]),
            };
            let root_text = root.to_str().ok_or_else(|| {
                ProcessProfileError::Invalid(
                    "canonical filesystem root is not valid UTF-8 for authority".to_owned(),
                )
            })?;
            authority_filesystem.push(
                FilesystemScope::new(root_text, access)
                    .map_err(|error| ProcessProfileError::Invalid(error.to_string()))?,
            );
        }
        let artifact_bytes = profile
            .limits
            .max_total_materialized_bytes
            .checked_add(profile.limits.max_total_output_bytes)
            .ok_or_else(|| {
                ProcessProfileError::Invalid(
                    "process artifact authority byte ceiling overflows".to_owned(),
                )
            })?;
        let authority_requirements = CapabilityExecutionRequirements {
            filesystem: authority_filesystem,
            secrets: profile.environment.secrets.values().cloned().collect(),
            budget: AuthorityBudget {
                duration_ms: Some(profile.limits.wall_timeout_ms),
                invocations: Some(1),
                artifact_bytes: Some(artifact_bytes),
                concurrency: Some(1),
                ..AuthorityBudget::default()
            },
            ..CapabilityExecutionRequirements::default()
        };
        if !executable_roots
            .iter()
            .any(|root| executable.starts_with(root))
        {
            return Err(ProcessProfileError::Invalid(
                "canonical executable is outside every executable root".to_owned(),
            ));
        }
        Ok(Self {
            profile,
            executable,
            executable_roots,
            writable_roots,
            authority_requirements,
            data,
            secrets,
            lifecycle: AtomicU8::new(Lifecycle::Created as u8),
            active: Arc::new(Mutex::new(BTreeMap::new())),
        })
    }

    /// Immutable validated profile.
    #[must_use]
    pub const fn profile(&self) -> &ProcessProfile {
        &self.profile
    }

    fn execute_inner(
        &self,
        invocation: &AdapterInvocation<'_>,
        reporter: &dyn AdapterReporter,
    ) -> Result<(), AdapterError> {
        let request = invocation.request();
        let mut sequence = 1_u64;
        if self.lifecycle.load(Ordering::SeqCst) != Lifecycle::Started as u8 {
            return report_rejected(
                reporter,
                request.invocation(),
                &mut sequence,
                ErrorClass::Unsupported,
                "process_host_not_accepting",
                "local process generation is not accepting work",
            );
        }
        if invocation.resolution().capability() != &self.profile.capability
            || invocation.resolution().descriptor_revision() != self.profile.descriptor_revision
            || invocation.resolution().operation() != &self.profile.operation
            || request.capability() != &self.profile.capability
            || request.operation() != &self.profile.operation
            || request.provider_profile() != self.profile.provider_profile.as_ref()
        {
            return report_rejected(
                reporter,
                request.invocation(),
                &mut sequence,
                ErrorClass::InvalidRequest,
                "profile_selection_mismatch",
                "invocation does not equal the configured process generation",
            );
        }
        let Some(context) = invocation.context() else {
            return report_rejected(
                reporter,
                request.invocation(),
                &mut sequence,
                ErrorClass::InvalidRequest,
                "missing_execution_provenance",
                "process execution requires exact durable run provenance",
            );
        };
        if request.inputs().len() > 120 {
            return report_rejected(
                reporter,
                request.invocation(),
                &mut sequence,
                ErrorClass::InvalidRequest,
                "input_provenance_bound",
                "process invocation exceeds the exact artifact provenance input bound",
            );
        }
        let specifications = match self
            .profile
            .inputs
            .iter()
            .map(|input| InputMaterialization::new(&input.input, &input.relative_path))
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(specifications) => specifications,
            Err(error) => {
                return report_rejected(
                    reporter,
                    request.invocation(),
                    &mut sequence,
                    ErrorClass::InvalidRequest,
                    "invalid_materialization_rule",
                    &bounded(&error.to_string()),
                );
            }
        };
        let workspace = match self.data.materialize(
            context,
            request,
            &specifications,
            self.profile.limits.materialization(),
        ) {
            Ok(workspace) => workspace,
            Err(error) => {
                return report_rejected(
                    reporter,
                    request.invocation(),
                    &mut sequence,
                    ErrorClass::InvalidRequest,
                    "materialization_failed",
                    &bounded(&error.to_string()),
                );
            }
        };
        let canonical_root = match workspace.root().canonicalize() {
            Ok(root) => root,
            Err(error) => {
                return report_rejected(
                    reporter,
                    request.invocation(),
                    &mut sequence,
                    ErrorClass::Adapter,
                    "execution_root_unavailable",
                    &format!("execution root cannot be canonicalized: {:?}", error.kind()),
                );
            }
        };
        if !self
            .writable_roots
            .iter()
            .any(|allowed| canonical_root.starts_with(allowed))
        {
            return report_rejected(
                reporter,
                request.invocation(),
                &mut sequence,
                ErrorClass::Authorization,
                "execution_root_denied",
                "isolated execution root is outside configured read-write roots",
            );
        }
        let working_directory =
            match prepare_working_directory(&canonical_root, &self.profile.working_directory) {
                Ok(path) => path,
                Err(message) => {
                    return report_rejected(
                        reporter,
                        request.invocation(),
                        &mut sequence,
                        ErrorClass::InvalidRequest,
                        "working_directory_rejected",
                        &message,
                    );
                }
            };
        let arguments = match materialize_arguments(&self.profile, request, workspace.as_ref()) {
            Ok(arguments) => arguments,
            Err(message) => {
                return report_rejected(
                    reporter,
                    request.invocation(),
                    &mut sequence,
                    ErrorClass::InvalidRequest,
                    "argument_substitution_rejected",
                    &message,
                );
            }
        };
        let stdin_bytes = match stdin_bytes(&self.profile, workspace.as_ref()) {
            Ok(bytes) => bytes,
            Err(message) => {
                return report_rejected(
                    reporter,
                    request.invocation(),
                    &mut sequence,
                    ErrorClass::InvalidRequest,
                    "stdin_rejected",
                    &message,
                );
            }
        };
        let mut resolved_secrets = Vec::new();
        let environment = match self.resolve_environment(&mut resolved_secrets) {
            Ok(environment) => environment,
            Err(message) => {
                return report_rejected(
                    reporter,
                    request.invocation(),
                    &mut sequence,
                    ErrorClass::Authentication,
                    "secret_resolution_failed",
                    &message,
                );
            }
        };

        let spawn_started = Instant::now();
        let mut child = match self.spawn(
            &working_directory,
            &arguments,
            &environment,
            stdin_bytes.is_some(),
        ) {
            Ok(child) => child,
            Err(error) => {
                return report_rejected(
                    reporter,
                    request.invocation(),
                    &mut sequence,
                    ErrorClass::Adapter,
                    "process_spawn_failed",
                    &format!("process spawn failed: {:?}", error.kind()),
                );
            }
        };
        let control = Arc::new(ProcessControl::new(&child));
        let _registration = match ActiveRegistration::insert(
            self.active.clone(),
            request.invocation().clone(),
            control.clone(),
        ) {
            Ok(registration) => registration,
            Err(error) => {
                terminate_child_immediately(&mut child, &control);
                return Err(AdapterError::external_failure(error));
            }
        };
        let stdin_thread = spawn_stdin_writer(child.stdin.take(), stdin_bytes);
        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => {
                terminate_child_immediately(&mut child, &control);
                return Err(AdapterError::external_failure(
                    "spawned process has no owned stdout pipe",
                ));
            }
        };
        let stderr = match child.stderr.take() {
            Some(stderr) => stderr,
            None => {
                terminate_child_immediately(&mut child, &control);
                return Err(AdapterError::external_failure(
                    "spawned process has no owned stderr pipe",
                ));
            }
        };
        let (stream_sender, stream_receiver) = sync_channel(STREAM_CHANNEL_MESSAGES);
        let stdout_thread = spawn_reader(
            Stream::Stdout,
            stdout,
            self.profile.stdout.max_capture_bytes,
            stream_sender.clone(),
        );
        let stderr_thread = spawn_reader(
            Stream::Stderr,
            stderr,
            self.profile.stderr.max_capture_bytes,
            stream_sender,
        );
        drop(environment);

        report(
            reporter,
            request.invocation(),
            &mut sequence,
            InvocationEventKind::Progress {
                message: "local process started".to_owned(),
                completed_units: None,
                total_units: None,
            },
        )?;

        let lifecycle = monitor_process(
            &mut child,
            &control,
            stream_receiver,
            reporter,
            request.invocation(),
            &mut sequence,
            &self.profile,
            spawn_started,
        );
        let stdin_result = join_io(stdin_thread, "stdin writer");
        let stdout_result = join_reader(stdout_thread, "stdout reader");
        let stderr_result = join_reader(stderr_thread, "stderr reader");
        let mut observed = match lifecycle {
            Ok(observed) => observed,
            Err(error) => {
                terminate_child_immediately(&mut child, &control);
                let _ = child.wait();
                return Err(error);
            }
        };
        if let Err(message) = stdin_result.and(stdout_result).and(stderr_result) {
            return terminal_failure(
                reporter,
                request.invocation(),
                &mut sequence,
                ErrorClass::Adapter,
                "process_io_failed",
                &message,
                self.profile.side_effect,
                spawn_started,
            );
        }
        redact_capture(&mut observed.stdout, &resolved_secrets);
        redact_capture(&mut observed.stderr, &resolved_secrets);
        drop(resolved_secrets);

        if let Some(termination) = observed.termination {
            return terminal_for_termination(
                reporter,
                request.invocation(),
                &mut sequence,
                termination,
                self.profile.side_effect,
                spawn_started,
                observed.group_absent,
            );
        }
        let Some(status) = observed.status else {
            return terminal_uncertain(
                reporter,
                request.invocation(),
                &mut sequence,
                "process_terminal_unobserved",
                "process outcome could not be observed after external entry",
                self.profile.side_effect,
                spawn_started,
            );
        };
        if !status.success() {
            let (code, message) = exit_failure(&status);
            return terminal_failure(
                reporter,
                request.invocation(),
                &mut sequence,
                ErrorClass::Provider,
                &code,
                &message,
                self.profile.side_effect,
                spawn_started,
            );
        }
        if observed.stdout_overflow
            && self.profile.stdout.overflow_action == OverflowAction::Terminate
            || observed.stderr_overflow
                && self.profile.stderr.overflow_action == OverflowAction::Terminate
        {
            return terminal_failure(
                reporter,
                request.invocation(),
                &mut sequence,
                ErrorClass::Adapter,
                "process_output_overflow",
                "process output exceeded a terminate-on-overflow bound",
                self.profile.side_effect,
                spawn_started,
            );
        }
        let outputs = match self.publish_outputs(
            context,
            request,
            workspace.as_ref(),
            &observed.stdout,
            &observed.stderr,
            reporter,
            &mut sequence,
        ) {
            Ok(outputs) => outputs,
            Err(message) => {
                return terminal_failure(
                    reporter,
                    request.invocation(),
                    &mut sequence,
                    ErrorClass::Adapter,
                    "output_publication_failed",
                    &message,
                    self.profile.side_effect,
                    spawn_started,
                );
            }
        };
        let terminal = InvocationTerminal::new(
            TerminalStatus::Success,
            outputs,
            None,
            usage(spawn_started),
            self.profile.side_effect,
        )
        .map_err(|error| AdapterError::external_failure(error.to_string()))?;
        report(
            reporter,
            request.invocation(),
            &mut sequence,
            InvocationEventKind::Terminal { terminal },
        )
    }

    fn resolve_environment(
        &self,
        resolved: &mut Vec<SensitiveSecret>,
    ) -> Result<Vec<(OsString, OsString)>, String> {
        let mut values = Vec::with_capacity(
            self.profile
                .environment
                .allowed_non_secret
                .len()
                .saturating_add(self.profile.environment.secrets.len()),
        );
        for name in &self.profile.environment.allowed_non_secret {
            let Some(value) = std::env::var_os(name) else {
                continue;
            };
            if os_bytes_len(&value) > self.profile.environment.max_value_bytes {
                return Err(format!(
                    "allowlisted environment variable '{name}' exceeds its byte bound"
                ));
            }
            values.push((OsString::from(name), value));
        }
        for (name, reference) in &self.profile.environment.secrets {
            let secret = self.secrets.resolve(reference).map_err(|_error| {
                format!("secret reference '{reference}' for environment '{name}' is unavailable")
            })?;
            if secret.is_empty() || secret.len() > self.profile.environment.max_value_bytes {
                return Err(format!(
                    "secret reference '{reference}' for environment '{name}' violates its byte bound"
                ));
            }
            let value = secret.expose(secret_os_string)?;
            values.push((OsString::from(name), value));
            resolved.push(secret);
        }
        Ok(values)
    }

    fn spawn(
        &self,
        working_directory: &Path,
        arguments: &[OsString],
        environment: &[(OsString, OsString)],
        piped_stdin: bool,
    ) -> std::io::Result<Child> {
        let mut command = Command::new(&self.executable);
        command
            .args(arguments)
            .current_dir(working_directory)
            .env_clear()
            .stdin(if piped_stdin {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (name, value) in environment {
            command.env(name, value);
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        command.spawn()
    }

    #[allow(clippy::too_many_arguments)]
    fn publish_outputs(
        &self,
        context: &milkdrift_capability_host::AdapterExecutionContext,
        request: &milkdrift_capability::InvocationRequest,
        workspace: &dyn MaterializedExecution,
        stdout: &[u8],
        stderr: &[u8],
        reporter: &dyn AdapterReporter,
        sequence: &mut u64,
    ) -> Result<Vec<milkdrift_capability::ArtifactReference>, String> {
        let mut planned_count = self.profile.outputs.len();
        planned_count = planned_count
            .saturating_add(usize::from(self.profile.stdout.artifact_name.is_some()))
            .saturating_add(usize::from(self.profile.stderr.artifact_name.is_some()));
        if planned_count > usize::from(self.profile.limits.max_output_files) {
            return Err("declared output count exceeds the publication bound".to_owned());
        }
        let mut total = u64::try_from(stdout.len())
            .ok()
            .and_then(|value| value.checked_add(u64::try_from(stderr.len()).ok()?))
            .ok_or_else(|| "captured output byte accounting overflow".to_owned())?;
        for output in &self.profile.outputs {
            match fs::symlink_metadata(workspace.root().join(&output.relative_path)) {
                Ok(metadata) => {
                    total = total
                        .checked_add(metadata.len())
                        .ok_or_else(|| "declared output byte accounting overflow".to_owned())?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound && !output.required => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Err(format!("required output '{}' is missing", output.name));
                }
                Err(error) => {
                    return Err(format!(
                        "declared output '{}' cannot be inspected: {:?}",
                        output.name,
                        error.kind()
                    ));
                }
            }
        }
        if total > self.profile.limits.max_total_output_bytes {
            return Err("declared outputs exceed the aggregate publication bound".to_owned());
        }
        let mut outputs = Vec::new();
        for (capture, bytes) in [
            (&self.profile.stdout, stdout),
            (&self.profile.stderr, stderr),
        ] {
            if let Some(name) = &capture.artifact_name {
                let reference = self
                    .data
                    .publish_bytes(
                        context,
                        request,
                        name,
                        "application/octet-stream",
                        bytes,
                        self.profile.limits.materialization(),
                    )
                    .map_err(|error| bounded(&error.to_string()))?;
                report(
                    reporter,
                    request.invocation(),
                    sequence,
                    InvocationEventKind::Output {
                        name: name.clone(),
                        reference: reference.clone(),
                    },
                )
                .map_err(|error| error.to_string())?;
                outputs.push(reference);
            }
        }
        for output in &self.profile.outputs {
            if !output.required
                && fs::symlink_metadata(workspace.root().join(&output.relative_path))
                    .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
            {
                continue;
            }
            let reference = self
                .data
                .publish_file(
                    context,
                    request,
                    workspace,
                    &output.name,
                    &output.relative_path,
                    &output.media_type,
                    self.profile.limits.materialization(),
                )
                .map_err(|error| bounded(&error.to_string()))?;
            report(
                reporter,
                request.invocation(),
                sequence,
                InvocationEventKind::Output {
                    name: output.name.clone(),
                    reference: reference.clone(),
                },
            )
            .map_err(|error| error.to_string())?;
            outputs.push(reference);
        }
        Ok(outputs)
    }
}

impl CapabilityAdapter for LocalProcessAdapter {
    fn authority_requirements(&self) -> CapabilityExecutionRequirements {
        self.authority_requirements.clone()
    }

    fn start(&self) -> Result<(), AdapterError> {
        self.lifecycle
            .compare_exchange(
                Lifecycle::Created as u8,
                Lifecycle::Started as u8,
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .map(|_prior| ())
            .map_err(|_prior| AdapterError::rejected("process adapter is already started"))
    }

    fn execute(
        &self,
        invocation: &AdapterInvocation<'_>,
        reporter: &dyn AdapterReporter,
    ) -> Result<(), AdapterError> {
        self.execute_inner(invocation, reporter)
    }

    fn cancel(
        &self,
        request: &CancellationRequest,
    ) -> Result<CancellationAcknowledgement, AdapterError> {
        let control = self
            .active
            .lock()
            .map_err(|_error| AdapterError::external_failure("process ownership unavailable"))?
            .get(request.invocation())
            .cloned();
        let Some(control) = control else {
            return CancellationAcknowledgement::new(
                request.invocation().clone(),
                request.request_sequence(),
                false,
                false,
                Some("no live process ownership receipt is present".to_owned()),
            )
            .map_err(|error| AdapterError::external_failure(error.to_string()));
        };
        control.cancel_requested.store(true, Ordering::SeqCst);
        let signal = control.request_graceful();
        let (accepted, detail) = match signal {
            Ok(()) => (
                true,
                "termination requested; terminal observation remains pending".to_owned(),
            ),
            Err(message) => (false, bounded(&message)),
        };
        CancellationAcknowledgement::new(
            request.invocation().clone(),
            request.request_sequence(),
            accepted,
            false,
            Some(detail),
        )
        .map_err(|error| AdapterError::external_failure(error.to_string()))
    }

    fn health(&self, observed_at_unix_ms: u64) -> Result<CapabilityObservation, AdapterError> {
        let lifecycle = self.lifecycle.load(Ordering::SeqCst);
        let load = self
            .active
            .lock()
            .map_err(|_error| AdapterError::unavailable("process ownership unavailable"))?
            .len();
        let current_load = u32::try_from(load).unwrap_or(u32::MAX);
        let available = lifecycle == Lifecycle::Started as u8
            && self.executable.is_file()
            && self
                .executable_roots
                .iter()
                .any(|root| self.executable.starts_with(root));
        CapabilityObservation::new(
            self.profile.capability.clone(),
            observed_at_unix_ms,
            available,
            current_load,
            if available {
                "configured executable is available"
            } else {
                "process generation is draining, stopped, or unavailable"
            },
        )
        .map_err(|error| AdapterError::unavailable(error.to_string()))
    }

    fn begin_drain(&self) -> Result<(), AdapterError> {
        let prior = self
            .lifecycle
            .swap(Lifecycle::Draining as u8, Ordering::SeqCst);
        if prior == Lifecycle::Stopped as u8 {
            return Err(AdapterError::rejected("process adapter is already stopped"));
        }
        Ok(())
    }

    fn shutdown(&self) -> Result<(), AdapterError> {
        self.lifecycle
            .store(Lifecycle::Stopped as u8, Ordering::SeqCst);
        let controls = self
            .active
            .lock()
            .map_err(|_error| AdapterError::external_failure("process ownership unavailable"))?
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for control in controls {
            control.cancel_requested.store(true, Ordering::SeqCst);
            let _ = control.request_force();
        }
        Ok(())
    }
}

#[repr(u8)]
enum Lifecycle {
    Created = 0,
    Started = 1,
    Draining = 2,
    Stopped = 3,
}

struct ProcessControl {
    cancel_requested: AtomicBool,
    #[cfg(unix)]
    process_group: rustix::process::Pid,
}

impl ProcessControl {
    fn new(child: &Child) -> Self {
        Self {
            cancel_requested: AtomicBool::new(false),
            #[cfg(unix)]
            process_group: rustix::process::Pid::from_child(child),
        }
    }

    fn request_graceful(&self) -> Result<(), String> {
        #[cfg(unix)]
        {
            signal_group(self.process_group, rustix::process::Signal::TERM)
        }
        #[cfg(not(unix))]
        {
            Ok(())
        }
    }

    fn request_force(&self) -> Result<(), String> {
        #[cfg(unix)]
        {
            signal_group(self.process_group, rustix::process::Signal::KILL)
        }
        #[cfg(not(unix))]
        {
            Ok(())
        }
    }

    fn group_absent(&self) -> bool {
        #[cfg(unix)]
        {
            match rustix::process::test_kill_process_group(self.process_group) {
                Ok(()) => false,
                Err(error) => error == rustix::io::Errno::SRCH,
            }
        }
        #[cfg(not(unix))]
        {
            false
        }
    }
}

#[cfg(unix)]
fn signal_group(
    group: rustix::process::Pid,
    signal: rustix::process::Signal,
) -> Result<(), String> {
    match rustix::process::kill_process_group(group, signal) {
        Ok(()) => Ok(()),
        Err(error) if error == rustix::io::Errno::SRCH => Ok(()),
        Err(error) => Err(format!("process-group signal failed: {error}")),
    }
}

struct ActiveRegistration {
    active: Arc<Mutex<BTreeMap<InvocationId, Arc<ProcessControl>>>>,
    invocation: InvocationId,
}

impl ActiveRegistration {
    fn insert(
        active: Arc<Mutex<BTreeMap<InvocationId, Arc<ProcessControl>>>>,
        invocation: InvocationId,
        control: Arc<ProcessControl>,
    ) -> Result<Self, String> {
        let mut owners = active
            .lock()
            .map_err(|_error| "process ownership state is unavailable".to_owned())?;
        if owners.insert(invocation.clone(), control).is_some() {
            return Err("invocation already owns a live local process".to_owned());
        }
        drop(owners);
        Ok(Self { active, invocation })
    }
}

impl Drop for ActiveRegistration {
    fn drop(&mut self) {
        if let Ok(mut active) = self.active.lock() {
            active.remove(&self.invocation);
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Stream {
    Stdout,
    Stderr,
}

enum StreamMessage {
    Data(Stream, Vec<u8>),
    Overflow(Stream),
    Closed(Stream),
    Failed(Stream, std::io::ErrorKind),
}

struct ProcessObservation {
    status: Option<ExitStatus>,
    termination: Option<Termination>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    stdout_overflow: bool,
    stderr_overflow: bool,
    group_absent: bool,
}

#[derive(Clone, Copy)]
enum Termination {
    Cancelled,
    TimedOut,
    OutputOverflow,
    UnexpectedDescendants,
    Unresolved,
}

#[allow(clippy::too_many_arguments)]
fn monitor_process(
    child: &mut Child,
    control: &ProcessControl,
    receiver: Receiver<StreamMessage>,
    reporter: &dyn AdapterReporter,
    invocation: &InvocationId,
    sequence: &mut u64,
    profile: &ProcessProfile,
    started: Instant,
) -> Result<ProcessObservation, AdapterError> {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut stdout_overflow = false;
    let mut stderr_overflow = false;
    let mut stdout_closed = false;
    let mut stderr_closed = false;
    let mut stdout_progress = 0_u16;
    let mut stderr_progress = 0_u16;
    let mut termination = None;
    let mut graceful_at = None;
    let mut forced_at = None;
    let mut next_heartbeat = started + Duration::from_millis(profile.limits.heartbeat_interval_ms);
    let wall_deadline = started + Duration::from_millis(profile.limits.wall_timeout_ms);
    let mut status = None;
    loop {
        match receiver.recv_timeout(POLL_INTERVAL) {
            Ok(StreamMessage::Data(stream, bytes)) => {
                let (capture, policy, count) = match stream {
                    Stream::Stdout => (&mut stdout, &profile.stdout, &mut stdout_progress),
                    Stream::Stderr => (&mut stderr, &profile.stderr, &mut stderr_progress),
                };
                capture.extend_from_slice(&bytes);
                if policy.stream_progress && *count < policy.max_progress_events {
                    let message = progress_message(stream, &bytes);
                    report(
                        reporter,
                        invocation,
                        sequence,
                        InvocationEventKind::Progress {
                            message,
                            completed_units: None,
                            total_units: None,
                        },
                    )?;
                    *count = count.saturating_add(1);
                }
            }
            Ok(StreamMessage::Overflow(stream)) => {
                let terminate = match stream {
                    Stream::Stdout => {
                        stdout_overflow = true;
                        profile.stdout.overflow_action == OverflowAction::Terminate
                    }
                    Stream::Stderr => {
                        stderr_overflow = true;
                        profile.stderr.overflow_action == OverflowAction::Terminate
                    }
                };
                if terminate && termination.is_none() {
                    termination = Some(Termination::OutputOverflow);
                }
            }
            Ok(StreamMessage::Closed(Stream::Stdout)) => stdout_closed = true,
            Ok(StreamMessage::Closed(Stream::Stderr)) => stderr_closed = true,
            Ok(StreamMessage::Failed(stream, kind)) => {
                let stream_name = match stream {
                    Stream::Stdout => "stdout",
                    Stream::Stderr => "stderr",
                };
                return Err(AdapterError::external_failure(format!(
                    "{stream_name} reader failed: {kind:?}"
                )));
            }
            Err(RecvTimeoutError::Disconnected) => {
                stdout_closed = true;
                stderr_closed = true;
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
        if status.is_none() {
            status = child.try_wait().map_err(|error| {
                AdapterError::external_failure(format!("process wait failed: {:?}", error.kind()))
            })?;
        }
        let now = Instant::now();
        if control.cancel_requested.load(Ordering::SeqCst) && termination.is_none() {
            termination = Some(Termination::Cancelled);
        }
        if now >= wall_deadline && termination.is_none() && status.is_none() {
            termination = Some(Termination::TimedOut);
        }
        if termination.is_some() && status.is_none() && graceful_at.is_none() {
            control
                .request_graceful()
                .map_err(AdapterError::external_failure)?;
            #[cfg(not(unix))]
            child
                .kill()
                .map_err(|error| AdapterError::external_failure(error.to_string()))?;
            graceful_at = Some(now);
        }
        if status.is_none()
            && graceful_at.is_some_and(|at| {
                now.duration_since(at)
                    >= Duration::from_millis(profile.limits.graceful_termination_ms)
            })
            && forced_at.is_none()
        {
            control
                .request_force()
                .map_err(AdapterError::external_failure)?;
            child
                .kill()
                .map_err(|error| AdapterError::external_failure(error.to_string()))?;
            forced_at = Some(now);
        }
        if status.is_none()
            && forced_at.is_some_and(|at| {
                now.duration_since(at)
                    >= Duration::from_millis(profile.limits.forced_termination_ms)
            })
        {
            termination = Some(Termination::Unresolved);
            break;
        }
        if now >= next_heartbeat && status.is_none() {
            reporter.heartbeat()?;
            next_heartbeat = now + Duration::from_millis(profile.limits.heartbeat_interval_ms);
        }
        if status.is_some() && stdout_closed && stderr_closed {
            break;
        }
    }
    if status.is_none() {
        status = child.try_wait().map_err(|error| {
            AdapterError::external_failure(format!("final process wait failed: {:?}", error.kind()))
        })?;
    }
    if termination.is_none() && status.is_some() && !control.group_absent() {
        termination = Some(Termination::UnexpectedDescendants);
        control
            .request_graceful()
            .map_err(AdapterError::external_failure)?;
        if !wait_for_group_absence(
            control,
            Duration::from_millis(profile.limits.graceful_termination_ms),
        ) {
            control
                .request_force()
                .map_err(AdapterError::external_failure)?;
        }
    }
    let group_absent = if termination.is_some() {
        wait_for_group_absence(
            control,
            Duration::from_millis(profile.limits.forced_termination_ms),
        )
    } else {
        control.group_absent()
    };
    Ok(ProcessObservation {
        status,
        termination,
        stdout,
        stderr,
        stdout_overflow,
        stderr_overflow,
        group_absent,
    })
}

fn spawn_reader<R: Read + Send + 'static>(
    stream: Stream,
    mut reader: R,
    maximum: u64,
    sender: SyncSender<StreamMessage>,
) -> JoinHandle<Result<(), String>> {
    thread::spawn(move || {
        let mut accepted = 0_u64;
        let mut overflow_sent = false;
        let mut buffer = [0_u8; STREAM_READ_BYTES];
        loop {
            let count = match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => count,
                Err(error) => {
                    let _ = sender.send(StreamMessage::Failed(stream, error.kind()));
                    return Err(format!("stream read failed: {:?}", error.kind()));
                }
            };
            let remaining = maximum.saturating_sub(accepted);
            let take = usize::try_from(remaining).unwrap_or(usize::MAX).min(count);
            if take != 0 {
                sender
                    .send(StreamMessage::Data(stream, buffer[..take].to_vec()))
                    .map_err(|_error| "stream receiver disconnected".to_owned())?;
                accepted = accepted.saturating_add(u64::try_from(take).unwrap_or(u64::MAX));
            }
            if take < count && !overflow_sent {
                sender
                    .send(StreamMessage::Overflow(stream))
                    .map_err(|_error| "stream receiver disconnected".to_owned())?;
                overflow_sent = true;
            }
        }
        sender
            .send(StreamMessage::Closed(stream))
            .map_err(|_error| "stream receiver disconnected".to_owned())?;
        Ok(())
    })
}

fn spawn_stdin_writer(
    stdin: Option<std::process::ChildStdin>,
    bytes: Option<Vec<u8>>,
) -> Option<JoinHandle<Result<(), String>>> {
    stdin.zip(bytes).map(|(mut stdin, bytes)| {
        thread::spawn(move || {
            stdin
                .write_all(&bytes)
                .map_err(|error| format!("stdin write failed: {:?}", error.kind()))?;
            stdin
                .flush()
                .map_err(|error| format!("stdin flush failed: {:?}", error.kind()))
        })
    })
}

fn join_io(thread: Option<JoinHandle<Result<(), String>>>, name: &str) -> Result<(), String> {
    match thread {
        Some(thread) => thread.join().map_err(|_panic| format!("{name} panicked"))?,
        None => Ok(()),
    }
}

fn join_reader(thread: JoinHandle<Result<(), String>>, name: &str) -> Result<(), String> {
    thread.join().map_err(|_panic| format!("{name} panicked"))?
}

fn materialize_arguments(
    profile: &ProcessProfile,
    request: &milkdrift_capability::InvocationRequest,
    workspace: &dyn MaterializedExecution,
) -> Result<Vec<OsString>, String> {
    if profile.arguments.len() > usize::from(profile.limits.max_argv_entries) {
        return Err("argument template count exceeds the configured bound".to_owned());
    }
    let mut resolved = BTreeMap::new();
    for (name, source) in &profile.substitutions {
        let value = match source {
            SubstitutionSource::InputText { input: input_name } => {
                let input = request
                    .inputs()
                    .iter()
                    .find(|candidate| candidate.name() == input_name)
                    .ok_or_else(|| format!("required inline input '{input_name}' is missing"))?;
                let InvocationValueReference::Inline { value } = input.value() else {
                    return Err(format!("input '{input_name}' is not an inline value"));
                };
                match value.value() {
                    serde_json::Value::String(value) => value.clone(),
                    serde_json::Value::Bool(_)
                    | serde_json::Value::Number(_)
                    | serde_json::Value::Null => serde_json::to_string(value.value())
                        .map_err(|error| bounded(&error.to_string()))?,
                    serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
                        return Err(format!(
                            "input '{input_name}' must be a scalar for argv substitution"
                        ));
                    }
                }
            }
            SubstitutionSource::InputPath { input } => workspace
                .input_path(input)
                .ok_or_else(|| format!("materialized input path '{input}' is unavailable"))?
                .to_str()
                .ok_or_else(|| format!("materialized input path '{input}' is not UTF-8"))?
                .to_owned(),
            SubstitutionSource::ConfigValue { value } => value.clone(),
            SubstitutionSource::ExecutionRoot => workspace
                .root()
                .to_str()
                .ok_or_else(|| "execution root is not UTF-8".to_owned())?
                .to_owned(),
            SubstitutionSource::InvocationId => request.invocation().as_str().to_owned(),
            SubstitutionSource::IdempotencyKey => request
                .idempotency_key()
                .ok_or_else(|| "required stable idempotency key is missing".to_owned())?
                .as_str()
                .to_owned(),
        };
        if value.contains('\0') || value.len() > 32_768 {
            return Err(format!("substitution '{name}' violates its byte bound"));
        }
        resolved.insert(name.as_str(), value);
    }
    let mut arguments = Vec::with_capacity(profile.arguments.len());
    let mut total = 0_u64;
    for template in &profile.arguments {
        let mut argument = template.clone();
        for name in placeholders(template).map_err(|error| bounded(&error.to_string()))? {
            let value = resolved
                .get(name)
                .ok_or_else(|| format!("unknown placeholder '{name}'"))?;
            argument = argument.replace(&format!("{{{{{name}}}}}"), value);
        }
        if argument.contains('\0') {
            return Err("a final argument contains NUL".to_owned());
        }
        total = total
            .checked_add(
                u64::try_from(argument.len())
                    .map_err(|_error| "argument byte accounting overflow".to_owned())?,
            )
            .ok_or_else(|| "argument byte accounting overflow".to_owned())?;
        if total > profile.limits.max_argv_bytes {
            return Err("final argument vector exceeds its aggregate byte bound".to_owned());
        }
        arguments.push(OsString::from(argument));
    }
    Ok(arguments)
}

fn stdin_bytes(
    profile: &ProcessProfile,
    workspace: &dyn MaterializedExecution,
) -> Result<Option<Vec<u8>>, String> {
    match &profile.stdin {
        StdinMode::Disabled => Ok(None),
        StdinMode::Input { input, max_bytes } => {
            let path = workspace
                .input_path(input)
                .ok_or_else(|| format!("stdin input '{input}' is not materialized"))?;
            let metadata = fs::symlink_metadata(path)
                .map_err(|error| format!("stdin input cannot be inspected: {:?}", error.kind()))?;
            if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
                return Err("stdin input is not a regular materialized file".to_owned());
            }
            if metadata.len() > *max_bytes {
                return Err("stdin input exceeds its configured byte bound".to_owned());
            }
            fs::read(path)
                .map(Some)
                .map_err(|error| format!("stdin input cannot be read: {:?}", error.kind()))
        }
    }
}

fn prepare_working_directory(root: &Path, mode: &WorkingDirectoryMode) -> Result<PathBuf, String> {
    match mode {
        WorkingDirectoryMode::IsolatedRoot => Ok(root.to_path_buf()),
        WorkingDirectoryMode::IsolatedSubdirectory { relative_path } => {
            let path = root.join(relative_path);
            fs::create_dir_all(&path).map_err(|error| {
                format!("working directory cannot be created: {:?}", error.kind())
            })?;
            let canonical = path.canonicalize().map_err(|error| {
                format!(
                    "working directory cannot be canonicalized: {:?}",
                    error.kind()
                )
            })?;
            if !canonical.starts_with(root) || !canonical.is_dir() {
                return Err("working directory escapes the isolated root".to_owned());
            }
            Ok(canonical)
        }
    }
}

fn progress_message(stream: Stream, bytes: &[u8]) -> String {
    let prefix = match stream {
        Stream::Stdout => "stdout: ",
        Stream::Stderr => "stderr: ",
    };
    let value = String::from_utf8_lossy(bytes);
    let mut message = format!("{prefix}{value}");
    if message.len() > 4_096 {
        let mut end = 4_096;
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        message.truncate(end);
    }
    message
}

fn report(
    reporter: &dyn AdapterReporter,
    invocation: &InvocationId,
    sequence: &mut u64,
    kind: InvocationEventKind,
) -> Result<(), AdapterError> {
    let event = InvocationEvent::new(invocation.clone(), *sequence, kind)
        .map_err(|error| AdapterError::external_failure(error.to_string()))?;
    reporter.invocation(event)?;
    *sequence = sequence
        .checked_add(1)
        .ok_or_else(|| AdapterError::external_failure("invocation sequence overflow"))?;
    Ok(())
}

fn report_rejected(
    reporter: &dyn AdapterReporter,
    invocation: &InvocationId,
    sequence: &mut u64,
    class: ErrorClass,
    code: &str,
    message: &str,
) -> Result<(), AdapterError> {
    let failure = InvocationFailure::new(class, false, code, bounded(message), None)
        .map_err(|error| AdapterError::rejected(error.to_string()))?;
    let terminal = InvocationTerminal::new(
        TerminalStatus::Rejected,
        Vec::new(),
        Some(failure),
        None,
        SideEffectClass::None,
    )
    .map_err(|error| AdapterError::rejected(error.to_string()))?;
    report(
        reporter,
        invocation,
        sequence,
        InvocationEventKind::Terminal { terminal },
    )
}

#[allow(clippy::too_many_arguments)]
fn terminal_failure(
    reporter: &dyn AdapterReporter,
    invocation: &InvocationId,
    sequence: &mut u64,
    class: ErrorClass,
    code: &str,
    message: &str,
    side_effect: SideEffectClass,
    started: Instant,
) -> Result<(), AdapterError> {
    let failure = InvocationFailure::new(class, false, code, bounded(message), None)
        .map_err(|error| AdapterError::external_failure(error.to_string()))?;
    let terminal = InvocationTerminal::new(
        TerminalStatus::Failure,
        Vec::new(),
        Some(failure),
        usage(started),
        side_effect,
    )
    .map_err(|error| AdapterError::external_failure(error.to_string()))?;
    report(
        reporter,
        invocation,
        sequence,
        InvocationEventKind::Terminal { terminal },
    )
}

#[allow(clippy::too_many_arguments)]
fn terminal_uncertain(
    reporter: &dyn AdapterReporter,
    invocation: &InvocationId,
    sequence: &mut u64,
    code: &str,
    message: &str,
    side_effect: SideEffectClass,
    started: Instant,
) -> Result<(), AdapterError> {
    let failure = InvocationFailure::new(ErrorClass::Unknown, false, code, bounded(message), None)
        .map_err(|error| AdapterError::external_failure(error.to_string()))?;
    let terminal = InvocationTerminal::new(
        TerminalStatus::Uncertain,
        Vec::new(),
        Some(failure),
        usage(started),
        side_effect,
    )
    .map_err(|error| AdapterError::external_failure(error.to_string()))?;
    report(
        reporter,
        invocation,
        sequence,
        InvocationEventKind::Terminal { terminal },
    )
}

#[allow(clippy::too_many_arguments)]
fn terminal_for_termination(
    reporter: &dyn AdapterReporter,
    invocation: &InvocationId,
    sequence: &mut u64,
    termination: Termination,
    side_effect: SideEffectClass,
    started: Instant,
    group_absent: bool,
) -> Result<(), AdapterError> {
    match termination {
        Termination::Cancelled if group_absent => {
            let terminal = InvocationTerminal::new(
                TerminalStatus::Cancelled,
                Vec::new(),
                None,
                usage(started),
                side_effect,
            )
            .map_err(|error| AdapterError::external_failure(error.to_string()))?;
            report(
                reporter,
                invocation,
                sequence,
                InvocationEventKind::Terminal { terminal },
            )
        }
        Termination::Cancelled => terminal_uncertain(
            reporter,
            invocation,
            sequence,
            "process_descendants_unresolved",
            "cancellation was requested but owned process-group disappearance was not proven",
            side_effect,
            started,
        ),
        Termination::TimedOut if group_absent => terminal_failure(
            reporter,
            invocation,
            sequence,
            ErrorClass::Provider,
            "process_timeout",
            "process exceeded its wall timeout and the owned group was terminated",
            side_effect,
            started,
        ),
        Termination::OutputOverflow if group_absent => terminal_failure(
            reporter,
            invocation,
            sequence,
            ErrorClass::Adapter,
            "process_output_overflow",
            "process output exceeded a terminate-on-overflow bound",
            side_effect,
            started,
        ),
        Termination::UnexpectedDescendants if group_absent => terminal_failure(
            reporter,
            invocation,
            sequence,
            ErrorClass::Adapter,
            "process_descendant_contract_violated",
            "the immediate process exited while owned descendants remained; the group was terminated",
            side_effect,
            started,
        ),
        Termination::TimedOut
        | Termination::OutputOverflow
        | Termination::UnexpectedDescendants
        | Termination::Unresolved => terminal_uncertain(
            reporter,
            invocation,
            sequence,
            "process_termination_unresolved",
            "process termination or descendant cleanup could not be proven",
            side_effect,
            started,
        ),
    }
}

fn usage(started: Instant) -> Option<UsageObservation> {
    let duration = u64::try_from(started.elapsed().as_millis()).ok();
    UsageObservation::new(None, None, duration, None, None, BTreeMap::new()).ok()
}

fn exit_failure(status: &ExitStatus) -> (String, String) {
    if let Some(code) = status.code() {
        return (
            "process_nonzero_exit".to_owned(),
            format!("process exited with status code {code}"),
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return (
                "process_signal_exit".to_owned(),
                format!("process terminated by signal {signal}"),
            );
        }
    }
    (
        "process_unknown_exit".to_owned(),
        "process exited without a portable status code".to_owned(),
    )
}

fn redact_capture(capture: &mut Vec<u8>, secrets: &[SensitiveSecret]) {
    for secret in secrets {
        secret.expose(|value| replace_all(capture, value, b"[redacted]"));
    }
}

fn replace_all(target: &mut Vec<u8>, needle: &[u8], replacement: &[u8]) {
    if needle.is_empty() || target.len() < needle.len() {
        return;
    }
    let mut output = Vec::with_capacity(target.len());
    let mut offset = 0_usize;
    while offset < target.len() {
        if target[offset..].starts_with(needle) {
            output.extend_from_slice(replacement);
            offset = offset.saturating_add(needle.len());
        } else {
            output.push(target[offset]);
            offset = offset.saturating_add(1);
        }
    }
    *target = output;
}

#[cfg(unix)]
fn secret_os_string(bytes: &[u8]) -> Result<OsString, String> {
    use std::os::unix::ffi::OsStringExt;
    if bytes.contains(&0) {
        return Err("resolved secret contains NUL".to_owned());
    }
    Ok(OsString::from_vec(bytes.to_vec()))
}

#[cfg(not(unix))]
fn secret_os_string(bytes: &[u8]) -> Result<OsString, String> {
    let value = std::str::from_utf8(bytes)
        .map_err(|_error| "resolved secret is not valid UTF-8 on this platform".to_owned())?;
    if value.contains('\0') {
        return Err("resolved secret contains NUL".to_owned());
    }
    Ok(OsString::from(value))
}

#[cfg(unix)]
fn os_bytes_len(value: &OsStr) -> usize {
    use std::os::unix::ffi::OsStrExt;
    value.as_bytes().len()
}

#[cfg(not(unix))]
fn os_bytes_len(value: &OsStr) -> usize {
    value.to_string_lossy().len()
}

fn terminate_child_immediately(child: &mut Child, control: &ProcessControl) {
    let _ = control.request_force();
    let _ = child.kill();
    let _ = child.wait();
}

fn wait_for_group_absence(control: &ProcessControl, maximum: Duration) -> bool {
    let deadline = Instant::now() + maximum;
    loop {
        if control.group_absent() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(5));
    }
}

fn bounded(value: &str) -> String {
    if value.len() <= 4_096 {
        return value.to_owned();
    }
    let mut end = 4_096;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}
