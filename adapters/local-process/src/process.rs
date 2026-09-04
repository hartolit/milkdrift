use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
        mpsc::sync_channel,
    },
    time::Instant,
};

use milkdrift_authority::{
    AccessMode, AuthorityBudget, CapabilityExecutionRequirements, FilesystemScope, SensitiveSecret,
};
use milkdrift_capability::{
    AdmissionBound, CancellationAcknowledgement, CancellationRequest, CapabilityDescriptor,
    CapabilityObservation, ErrorClass, InvocationAdmissionEnvelope, InvocationEventKind,
    InvocationId, InvocationTerminal, TerminalStatus,
};
use milkdrift_capability_host::{
    AdapterError, AdapterInvocation, AdapterReporter, CapabilityAdapter, InputMaterialization,
    InvocationDataAccess, SecretResolver,
};

use crate::config::{
    FilesystemAccessMode, OverflowAction, ProcessProfile, ProcessProfileError,
    VerifiedExecutableIdentity, WorkingDirectoryMode,
};

mod identity;
mod monitor;
mod outputs;
mod platform;
mod prepare;
mod reporting;
mod spawn;
mod streams;

use identity::{ExecutableBinding, IdentityFailure};
use monitor::monitor_process;
use platform::{ActiveRegistration, ProcessControl, terminate_child_immediately};
use prepare::{materialize_arguments, prepare_working_directory, stdin_bytes};
use reporting::{TerminalReportContext, exit_failure, report_rejected, usage};
use streams::{
    Stream, join_io, join_reader, os_bytes_len, redact_capture, secret_os_string, spawn_reader,
    spawn_stdin_writer,
};

const STREAM_CHANNEL_MESSAGES: usize = 16;

/// Production local-process adapter for one immutable validated profile generation.
pub struct LocalProcessAdapter {
    profile: ProcessProfile,
    descriptor: CapabilityDescriptor,
    executable: PathBuf,
    executable_identity: VerifiedExecutableIdentity,
    executable_roots: Vec<PathBuf>,
    writable_roots: Vec<PathBuf>,
    authorized_host_working_directory: Option<PathBuf>,
    authority_requirements: CapabilityExecutionRequirements,
    data: Arc<dyn InvocationDataAccess>,
    secrets: Arc<dyn SecretResolver>,
    lifecycle: AtomicU8,
    identity_failure: Mutex<Option<IdentityFailure>>,
    active: Arc<Mutex<BTreeMap<InvocationId, Arc<ProcessControl>>>>,
}

impl LocalProcessAdapter {
    /// Canonicalizes configured host paths and creates one adapter generation.
    pub fn new(
        profile: ProcessProfile,
        data: Arc<dyn InvocationDataAccess>,
        secrets: Arc<dyn SecretResolver>,
    ) -> Result<Self, ProcessProfileError> {
        let ExecutableBinding {
            canonical_path: executable,
            evidence: executable_identity,
        } = identity::bind(&profile)?;
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
            authority_filesystem.push(
                FilesystemScope::from_canonical_host_path(&root, access)
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
        let authorized_host_working_directory = match &profile.working_directory {
            WorkingDirectoryMode::AuthorizedHostPath { path } => {
                let metadata = std::fs::symlink_metadata(path).map_err(|error| {
                    ProcessProfileError::Invalid(format!(
                        "authorized host working directory cannot be inspected: {:?}",
                        error.kind()
                    ))
                })?;
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(ProcessProfileError::Invalid(
                        "authorized host working directory is not a plain directory".to_owned(),
                    ));
                }
                let canonical = path.canonicalize().map_err(|error| {
                    ProcessProfileError::Invalid(format!(
                        "authorized host working directory cannot be canonicalized: {:?}",
                        error.kind()
                    ))
                })?;
                if !writable_roots
                    .iter()
                    .any(|allowed| canonical.starts_with(allowed))
                {
                    return Err(ProcessProfileError::Invalid(
                        "authorized host working directory is outside every read-write root"
                            .to_owned(),
                    ));
                }
                Some(canonical)
            }
            WorkingDirectoryMode::IsolatedRoot
            | WorkingDirectoryMode::IsolatedSubdirectory { .. } => None,
        };
        let descriptor = profile.descriptor(&executable_identity)?;
        Ok(Self {
            profile,
            descriptor,
            executable,
            executable_identity,
            executable_roots,
            writable_roots,
            authorized_host_working_directory,
            authority_requirements,
            data,
            secrets,
            lifecycle: AtomicU8::new(Lifecycle::Created as u8),
            identity_failure: Mutex::new(None),
            active: Arc::new(Mutex::new(BTreeMap::new())),
        })
    }

    /// Immutable descriptor constructed from verified executable and profile facts.
    #[must_use]
    pub const fn descriptor(&self) -> &CapabilityDescriptor {
        &self.descriptor
    }

    fn latched_identity_failure(&self) -> Result<Option<IdentityFailure>, AdapterError> {
        self.identity_failure
            .lock()
            .map(|failure| *failure)
            .map_err(|_error| AdapterError::unavailable("tool_identity_state_unavailable"))
    }

    fn revalidate_identity(&self) -> Result<VerifiedExecutableIdentity, IdentityFailure> {
        let mut failure = self
            .identity_failure
            .lock()
            .map_err(|_error| IdentityFailure::ReadFailed)?;
        if let Some(failure) = *failure {
            return Err(failure);
        }
        match identity::revalidate(
            &self.profile,
            &self.executable,
            &self.executable_identity,
            &self.executable_roots,
        ) {
            Ok(evidence) => Ok(evidence),
            Err(observed) => {
                *failure = Some(observed);
                Err(observed)
            }
        }
    }

    fn execute_inner(
        &self,
        invocation: &AdapterInvocation<'_>,
        reporter: &dyn AdapterReporter,
    ) -> Result<(), AdapterError> {
        let request = invocation.request();
        let mut sequence = 1_u64;
        if !matches!(
            self.lifecycle.load(Ordering::SeqCst),
            value if value == Lifecycle::Started as u8 || value == Lifecycle::Draining as u8
        ) {
            return report_rejected(
                reporter,
                request.invocation(),
                &mut sequence,
                ErrorClass::Unsupported,
                "process_host_not_accepting",
                "local process generation is not accepting work",
            );
        }
        if let Some(failure) = self.latched_identity_failure()? {
            return report_rejected(
                reporter,
                request.invocation(),
                &mut sequence,
                ErrorClass::Adapter,
                failure.code(),
                "registered tool generation is unavailable after identity invalidation",
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
        let working_directory = match prepare_working_directory(
            &canonical_root,
            &self.profile.working_directory,
            self.authorized_host_working_directory.as_deref(),
        ) {
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

        let pre_entry_identity = match self.revalidate_identity() {
            Ok(identity) => identity,
            Err(failure) => {
                return report_rejected(
                    reporter,
                    request.invocation(),
                    &mut sequence,
                    ErrorClass::Adapter,
                    failure.code(),
                    "executable identity changed before external process entry",
                );
            }
        };

        let spawn_started = Instant::now();
        let mut child = match spawn::spawn(
            &self.executable,
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

        let mut reports = TerminalReportContext::new(
            reporter,
            request.invocation(),
            &mut sequence,
            self.profile.side_effect,
            spawn_started,
        );
        reports.report(InvocationEventKind::Progress {
            message: format!(
                "local process started; pre-entry identity {} verified",
                pre_entry_identity.identity_digest
            ),
            completed_units: None,
            total_units: None,
        })?;

        let lifecycle = monitor_process(
            &mut child,
            &control,
            stream_receiver,
            &mut reports,
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
            return reports.failure(ErrorClass::Adapter, "process_io_failed", &message);
        }
        redact_capture(&mut observed.stdout, &resolved_secrets);
        redact_capture(&mut observed.stderr, &resolved_secrets);
        drop(resolved_secrets);

        if let Some(termination) = observed.termination {
            return reports.for_termination(termination, observed.termination_confirmed);
        }
        let Some(status) = observed.status else {
            return reports.uncertain(
                "process_terminal_unobserved",
                "process outcome could not be observed after external entry",
            );
        };
        if !status.success() {
            let (code, message) = exit_failure(&status);
            return reports.failure(ErrorClass::Provider, &code, &message);
        }
        if observed.stdout_overflow
            && self.profile.stdout.overflow_action == OverflowAction::Terminate
            || observed.stderr_overflow
                && self.profile.stderr.overflow_action == OverflowAction::Terminate
        {
            return reports.failure(
                ErrorClass::Adapter,
                "process_output_overflow",
                "process output exceeded a terminate-on-overflow bound",
            );
        }
        let outputs = match outputs::publish(
            &self.profile,
            self.data.as_ref(),
            context,
            request,
            workspace.as_ref(),
            &observed,
            &mut reports,
        ) {
            Ok(outputs) => outputs,
            Err(message) => {
                return reports.failure(ErrorClass::Adapter, "output_publication_failed", &message);
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
        reports.report(InvocationEventKind::Terminal { terminal })
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
}

impl CapabilityAdapter for LocalProcessAdapter {
    fn authority_requirements(&self) -> CapabilityExecutionRequirements {
        self.authority_requirements.clone()
    }

    fn admission_envelope(
        &self,
        _invocation: &AdapterInvocation<'_>,
    ) -> Result<InvocationAdmissionEnvelope, AdapterError> {
        Ok(InvocationAdmissionEnvelope::new(
            AdmissionBound::NotApplicable,
            AdmissionBound::NotApplicable,
            AdmissionBound::Bounded(self.profile.limits.max_total_output_bytes),
            AdmissionBound::NotApplicable,
        ))
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
        let identity = if lifecycle == Lifecycle::Started as u8 {
            self.revalidate_identity()
        } else {
            Err(IdentityFailure::PathUnavailable)
        };
        let available = lifecycle == Lifecycle::Started as u8 && identity.is_ok();
        let summary = if lifecycle != Lifecycle::Started as u8 {
            "process_generation_not_accepting"
        } else {
            match identity {
                Ok(_) => "tool_identity_verified",
                Err(failure) => failure.code(),
            }
        };
        CapabilityObservation::new(
            self.profile.capability.clone(),
            observed_at_unix_ms,
            available,
            current_load,
            summary,
        )
        .map_err(|error| AdapterError::unavailable(error.to_string()))
    }

    fn begin_drain(&self) -> Result<(), AdapterError> {
        loop {
            let prior = self.lifecycle.load(Ordering::SeqCst);
            if prior == Lifecycle::Draining as u8 {
                return Ok(());
            }
            if prior != Lifecycle::Started as u8 {
                return Err(AdapterError::rejected(
                    "process adapter must be started before drain",
                ));
            }
            if self
                .lifecycle
                .compare_exchange(
                    prior,
                    Lifecycle::Draining as u8,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
                .is_ok()
            {
                return Ok(());
            }
        }
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

fn bounded(value: &str) -> String {
    milkdrift_contracts::truncate_utf8(value, 4_096).to_owned()
}
