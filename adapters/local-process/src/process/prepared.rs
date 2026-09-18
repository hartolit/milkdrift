//! Freeze bounded process inputs and launch configuration before durable entry intent.
use super::{
    AdapterError, AdapterInvocation, ErrorClass, InputMaterialization, Lifecycle,
    LocalProcessAdapter, Ordering, OsString, PathBuf, SensitiveSecret, bounded,
    materialize_arguments, prepare_working_directory, stdin_bytes,
};
use milkdrift_capability_host::MaterializedExecution;

pub(super) struct PreparedProcess {
    pub(super) workspace: Box<dyn MaterializedExecution>,
    pub(super) working_directory: PathBuf,
    pub(super) arguments: Vec<OsString>,
    pub(super) stdin_bytes: Option<Vec<u8>>,
    pub(super) environment: Vec<(OsString, OsString)>,
    pub(super) resolved_secrets: Vec<SensitiveSecret>,
}

pub(super) struct PreparationFailure {
    pub(super) class: ErrorClass,
    pub(super) code: String,
    pub(super) detail: String,
}

impl PreparationFailure {
    fn new(class: ErrorClass, code: impl Into<String>, detail: &str) -> Self {
        Self {
            class,
            code: code.into(),
            detail: detail.to_owned(),
        }
    }
    pub(super) fn adapter_error(&self) -> AdapterError {
        AdapterError::rejected(format!("{}: {}", self.code, self.detail))
    }
}

impl From<AdapterError> for PreparationFailure {
    fn from(error: AdapterError) -> Self {
        Self::new(
            ErrorClass::Adapter,
            "process_preparation_failed",
            error.summary(),
        )
    }
}

impl LocalProcessAdapter {
    pub(super) fn prepare_process(
        &self,
        invocation: &AdapterInvocation<'_>,
    ) -> Result<PreparedProcess, PreparationFailure> {
        let request = invocation.request();
        if !matches!(
            self.lifecycle.load(Ordering::SeqCst),
            value if value == Lifecycle::Started as u8 || value == Lifecycle::Draining as u8
        ) {
            return Err(PreparationFailure::new(
                ErrorClass::Unsupported,
                "process_host_not_accepting",
                "local process generation is not accepting work",
            ));
        }
        if let Some(failure) = self.latched_identity_failure()? {
            return Err(PreparationFailure::new(
                ErrorClass::Adapter,
                failure.code(),
                "registered tool generation is unavailable after identity invalidation",
            ));
        }
        if invocation.resolution().capability() != &self.profile.capability
            || invocation.resolution().descriptor_revision() != self.profile.descriptor_revision
            || invocation.resolution().operation() != &self.profile.operation
            || request.capability() != &self.profile.capability
            || request.operation() != &self.profile.operation
            || request.provider_profile() != self.profile.provider_profile.as_ref()
        {
            return Err(PreparationFailure::new(
                ErrorClass::InvalidRequest,
                "profile_selection_mismatch",
                "invocation does not equal the configured process generation",
            ));
        }
        let Some(context) = invocation.context() else {
            return Err(PreparationFailure::new(
                ErrorClass::InvalidRequest,
                "missing_execution_provenance",
                "process execution requires exact durable run provenance",
            ));
        };
        if request.inputs().len() > 120 {
            return Err(PreparationFailure::new(
                ErrorClass::InvalidRequest,
                "input_provenance_bound",
                "process invocation exceeds the exact artifact provenance input bound",
            ));
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
                return Err(PreparationFailure::new(
                    ErrorClass::InvalidRequest,
                    "invalid_materialization_rule",
                    &bounded(&error.to_string()),
                ));
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
                return Err(PreparationFailure::new(
                    ErrorClass::InvalidRequest,
                    "materialization_failed",
                    &bounded(&error.to_string()),
                ));
            }
        };
        let canonical_root = match workspace.root().canonicalize() {
            Ok(root) => root,
            Err(error) => {
                return Err(PreparationFailure::new(
                    ErrorClass::Adapter,
                    "execution_root_unavailable",
                    &format!("execution root cannot be canonicalized: {:?}", error.kind()),
                ));
            }
        };
        if !self
            .writable_roots
            .iter()
            .any(|allowed| canonical_root.starts_with(allowed))
        {
            return Err(PreparationFailure::new(
                ErrorClass::Authorization,
                "execution_root_denied",
                "isolated execution root is outside configured read-write roots",
            ));
        }
        let working_directory = match prepare_working_directory(
            &canonical_root,
            &self.profile.working_directory,
            self.authorized_host_working_directory.as_deref(),
        ) {
            Ok(path) => path,
            Err(message) => {
                return Err(PreparationFailure::new(
                    ErrorClass::InvalidRequest,
                    "working_directory_rejected",
                    &message,
                ));
            }
        };
        let arguments = match materialize_arguments(&self.profile, request, workspace.as_ref()) {
            Ok(arguments) => arguments,
            Err(message) => {
                return Err(PreparationFailure::new(
                    ErrorClass::InvalidRequest,
                    "argument_substitution_rejected",
                    &message,
                ));
            }
        };
        let stdin_bytes = match stdin_bytes(&self.profile, workspace.as_ref()) {
            Ok(bytes) => bytes,
            Err(message) => {
                return Err(PreparationFailure::new(
                    ErrorClass::InvalidRequest,
                    "stdin_rejected",
                    &message,
                ));
            }
        };
        let mut resolved_secrets = Vec::new();
        let environment = match self.resolve_environment(&mut resolved_secrets) {
            Ok(environment) => environment,
            Err(message) => {
                return Err(PreparationFailure::new(
                    ErrorClass::Authentication,
                    "secret_resolution_failed",
                    &message,
                ));
            }
        };

        self.revalidate_identity().map_err(|failure| {
            PreparationFailure::new(
                ErrorClass::Adapter,
                failure.code(),
                "executable identity changed before process preparation",
            )
        })?;
        Ok(PreparedProcess {
            workspace,
            working_directory,
            arguments,
            stdin_bytes,
            environment,
            resolved_secrets,
        })
    }
}
