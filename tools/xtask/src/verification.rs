use std::error::Error;
use std::fmt;
use std::path::Path;

use cargo_metadata::Metadata;

use crate::workspace::{
    CUDA_FEATURE, HardwareSuite, WorkspaceInventoryIssue, benchmark_inventory,
    cuda_feature_package_inventory, cuda_hardware_target_inventory, domain_package_inventory,
    hardware_suite_inventory, load_metadata, workspace_package_inventory,
};

const PORTABLE_TARGETS: [&str; 2] = ["wasm32-unknown-unknown", "thumbv7em-none-eabihf"];

/// One exact Cargo invocation generated from canonical workspace ownership metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CargoCommand {
    arguments: Vec<String>,
}

impl CargoCommand {
    fn new(arguments: &[&str]) -> Self {
        Self {
            arguments: arguments
                .iter()
                .map(|argument| (*argument).to_owned())
                .collect(),
        }
    }

    fn benchmark(package: &str, target: &str) -> Self {
        Self {
            arguments: [
                "bench", "--locked", "-p", package, "--bench", target, "--no-run",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        }
    }

    fn packages(prefix: &[&str], packages: &[String], suffix: &[&str]) -> Self {
        let mut arguments = prefix
            .iter()
            .chain(suffix.iter())
            .map(|argument| (*argument).to_owned())
            .collect::<Vec<_>>();
        let suffix_arguments = arguments.split_off(prefix.len());
        for package in packages {
            arguments.push("-p".to_owned());
            arguments.push(package.clone());
        }
        arguments.extend(suffix_arguments);
        Self { arguments }
    }

    fn hardware_target(prefix: &[&str], hardware_target: &HardwareSuite, suffix: &[&str]) -> Self {
        let mut arguments = prefix
            .iter()
            .map(|argument| (*argument).to_owned())
            .collect::<Vec<_>>();
        arguments.extend([
            "-p".to_owned(),
            hardware_target.package().to_owned(),
            "--features".to_owned(),
            hardware_target.feature().to_owned(),
            "--test".to_owned(),
            hardware_target.target().to_owned(),
        ]);
        arguments.extend(suffix.iter().map(|argument| (*argument).to_owned()));
        Self { arguments }
    }

    /// Arguments passed to Cargo, beginning with the Cargo subcommand.
    #[must_use]
    pub fn arguments(&self) -> &[String] {
        &self.arguments
    }
}

/// One independently runnable native verification boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerificationComponent {
    /// Architecture, hygiene, formatting, and locked metadata.
    Structure,
    /// Workspace all-target check.
    Check,
    /// Workspace tests and doctests.
    Test,
    /// Workspace all-target Clippy with warnings denied.
    Clippy,
    /// Workspace rustdoc with warnings denied.
    Docs,
    /// Exact metadata-registered benchmark compilation.
    Benches,
    /// Scheduled exploratory Clippy nursery findings.
    Nursery,
}

impl VerificationComponent {
    /// Components included in the local canonical composite, in execution order.
    pub const CANONICAL: [Self; 6] = [
        Self::Structure,
        Self::Check,
        Self::Test,
        Self::Clippy,
        Self::Docs,
        Self::Benches,
    ];

    /// Parses one exact command-line component name.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "structure" => Some(Self::Structure),
            "check" => Some(Self::Check),
            "test" => Some(Self::Test),
            "clippy" => Some(Self::Clippy),
            "docs" => Some(Self::Docs),
            "benches" => Some(Self::Benches),
            "nursery" => Some(Self::Nursery),
            _ => None,
        }
    }

    /// Returns the exact command-line component name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Structure => "structure",
            Self::Check => "check",
            Self::Test => "test",
            Self::Clippy => "clippy",
            Self::Docs => "docs",
            Self::Benches => "benches",
            Self::Nursery => "nursery",
        }
    }
}

/// One policy or Cargo operation in a native verification component.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VerificationOperation {
    /// Validate the workspace architecture and dependency policy.
    Architecture,
    /// Validate repository operational hygiene.
    Hygiene,
    /// Execute the exact Cargo command.
    Cargo(CargoCommand),
}

/// The ordered operations owned by one independently runnable component.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerificationPlan {
    component: VerificationComponent,
    operations: Vec<VerificationOperation>,
}

impl VerificationPlan {
    /// The component represented by this plan.
    #[must_use]
    pub const fn component(&self) -> VerificationComponent {
        self.component
    }

    /// Ordered operations executed by the component.
    #[must_use]
    pub fn operations(&self) -> &[VerificationOperation] {
        &self.operations
    }
}

/// An error that prevents generation of a canonical Cargo command plan.
#[derive(Debug)]
pub struct CommandPlanError {
    message: String,
    source: Option<cargo_metadata::Error>,
}

impl CommandPlanError {
    fn metadata(context: &str, error: cargo_metadata::Error) -> Self {
        Self {
            message: format!("could not load locked Cargo metadata for {context}: {error}"),
            source: Some(error),
        }
    }

    fn inventory(context: &str, issues: Vec<WorkspaceInventoryIssue>) -> Self {
        Self {
            message: format!(
                "could not derive {context}: {}",
                issues
                    .into_iter()
                    .map(|issue| format!("{} / {}: {}", issue.package, issue.target, issue.reason))
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
            source: None,
        }
    }

    fn invalid(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            source: None,
        }
    }
}

impl fmt::Display for CommandPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for CommandPlanError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source
            .as_ref()
            .map(|error| error as &(dyn Error + 'static))
    }
}

/// Backward-compatible name for errors generated by maintained-benchmark planning.
pub type BenchmarkPlanError = CommandPlanError;

/// Loads locked Cargo metadata and returns sorted exact commands for maintained bench targets.
///
/// Each command has the form `cargo bench --locked -p PACKAGE --bench TARGET --no-run`; command
/// generation never uses `--workspace` and fails if the owning-package registry is inconsistent
/// with Cargo's actual bench targets.
///
/// # Errors
///
/// Returns an error if locked metadata cannot be loaded or the bidirectional benchmark registry is
/// invalid.
pub fn benchmark_command_plan(
    manifest_path: &Path,
) -> Result<Vec<CargoCommand>, BenchmarkPlanError> {
    let metadata = load_metadata(manifest_path, true)
        .map_err(|error| CommandPlanError::metadata("benchmark planning", error))?;
    benchmark_command_plan_for_metadata(&metadata)
}

/// Returns sorted exact maintained-benchmark commands from loaded Cargo metadata.
///
/// # Errors
///
/// Returns an error if the bidirectional benchmark registry is invalid.
pub fn benchmark_command_plan_for_metadata(
    metadata: &Metadata,
) -> Result<Vec<CargoCommand>, BenchmarkPlanError> {
    let inventory = benchmark_inventory(metadata).map_err(|issues| CommandPlanError {
        message: issues
            .into_iter()
            .map(|issue| format!("{} / {}: {}", issue.package, issue.target, issue.reason))
            .collect::<Vec<_>>()
            .join("; "),
        source: None,
    })?;

    Ok(inventory
        .into_iter()
        .map(|benchmark| CargoCommand::benchmark(benchmark.package(), benchmark.target()))
        .collect())
}

/// Loads locked Cargo metadata and plans the complete local canonical native gate.
///
/// # Errors
///
/// Returns an error if locked metadata, role ownership, or benchmark ownership is invalid.
pub fn native_verification_plan(
    manifest_path: &Path,
) -> Result<Vec<VerificationPlan>, CommandPlanError> {
    let metadata = load_metadata(manifest_path, true)
        .map_err(|error| CommandPlanError::metadata("native verification planning", error))?;
    VerificationComponent::CANONICAL
        .into_iter()
        .map(|component| verification_component_plan_for_metadata(&metadata, component))
        .collect()
}

/// Loads locked Cargo metadata and plans one independently runnable native component.
///
/// # Errors
///
/// Returns an error if locked metadata, role ownership, or benchmark ownership is invalid.
pub fn verification_component_plan(
    manifest_path: &Path,
    component: VerificationComponent,
) -> Result<VerificationPlan, CommandPlanError> {
    let metadata = load_metadata(manifest_path, true).map_err(|error| {
        CommandPlanError::metadata(
            &format!("{} verification planning", component.as_str()),
            error,
        )
    })?;
    verification_component_plan_for_metadata(&metadata, component)
}

/// Plans one native component from already loaded locked Cargo metadata.
///
/// # Errors
///
/// Returns an error if a package role or the benchmark inventory is invalid.
pub fn verification_component_plan_for_metadata(
    metadata: &Metadata,
    component: VerificationComponent,
) -> Result<VerificationPlan, CommandPlanError> {
    let packages = workspace_package_inventory(metadata)
        .map_err(|issues| CommandPlanError::inventory("native workspace ownership", issues))?;
    let operations =
        match component {
            VerificationComponent::Structure => vec![
                VerificationOperation::Architecture,
                VerificationOperation::Hygiene,
                VerificationOperation::Cargo(CargoCommand::new(&["fmt", "--all", "--", "--check"])),
                VerificationOperation::Cargo(CargoCommand::new(&[
                    "metadata",
                    "--locked",
                    "--format-version",
                    "1",
                    "--no-deps",
                ])),
            ],
            VerificationComponent::Check => vec![VerificationOperation::Cargo(
                CargoCommand::packages(&["check", "--locked"], &packages, &["--all-targets"]),
            )],
            VerificationComponent::Test => vec![VerificationOperation::Cargo(
                CargoCommand::packages(&["test", "--locked"], &packages, &[]),
            )],
            VerificationComponent::Clippy => {
                vec![VerificationOperation::Cargo(CargoCommand::packages(
                    &["clippy", "--locked"],
                    &packages,
                    &["--all-targets", "--", "-D", "warnings"],
                ))]
            }
            VerificationComponent::Docs => vec![VerificationOperation::Cargo(
                CargoCommand::packages(&["doc", "--locked"], &packages, &["--no-deps"]),
            )],
            VerificationComponent::Benches => benchmark_command_plan_for_metadata(metadata)?
                .into_iter()
                .map(VerificationOperation::Cargo)
                .collect(),
            VerificationComponent::Nursery => {
                vec![VerificationOperation::Cargo(CargoCommand::packages(
                    &["clippy", "--locked"],
                    &packages,
                    &["--all-targets", "--", "-D", "clippy::nursery"],
                ))]
            }
        };
    Ok(VerificationPlan {
        component,
        operations,
    })
}

/// Loads locked Cargo metadata and plans the exact portable domain-library check.
///
/// # Errors
///
/// Returns an error if `target` is not one of the two maintained portable targets, metadata cannot
/// be loaded with `--locked`, any workspace role is invalid, or no domain-role package exists.
pub fn portable_command_plan(
    manifest_path: &Path,
    target: &str,
) -> Result<Vec<CargoCommand>, CommandPlanError> {
    validate_portable_target(target)?;
    let metadata = load_metadata(manifest_path, true)
        .map_err(|error| CommandPlanError::metadata("portable planning", error))?;
    portable_command_plan_for_metadata(&metadata, target)
}

/// Plans the exact portable domain-library check from already loaded Cargo metadata.
///
/// # Errors
///
/// Returns an error if `target` is unsupported, any workspace role is invalid, or no domain-role
/// package exists.
pub fn portable_command_plan_for_metadata(
    metadata: &Metadata,
    target: &str,
) -> Result<Vec<CargoCommand>, CommandPlanError> {
    validate_portable_target(target)?;
    let packages = domain_package_inventory(metadata)
        .map_err(|issues| CommandPlanError::inventory("portable domain ownership", issues))?;
    Ok(vec![CargoCommand::packages(
        &["check", "--locked", "--target", target, "--lib"],
        &packages,
        &[],
    )])
}

/// Loads locked Cargo metadata and plans CUDA checks plus test compilation.
///
/// The plan checks and compiles every package declaring the exact `cuda` feature, then compiles
/// each exact harness-free CUDA hardware target in a separate Cargo invocation.
///
/// # Errors
///
/// Returns an error if locked metadata cannot be loaded, no exact `cuda` owner exists, or any CUDA
/// hardware target declaration is invalid.
pub fn cuda_compile_command_plan(
    manifest_path: &Path,
) -> Result<Vec<CargoCommand>, CommandPlanError> {
    let metadata = load_metadata(manifest_path, true)
        .map_err(|error| CommandPlanError::metadata("CUDA compile planning", error))?;
    cuda_compile_command_plan_for_metadata(&metadata)
}

/// Plans CUDA checks plus test compilation from already loaded Cargo metadata.
///
/// # Errors
///
/// Returns an error if no exact `cuda` owner exists or any CUDA hardware target declaration is
/// invalid.
pub fn cuda_compile_command_plan_for_metadata(
    metadata: &Metadata,
) -> Result<Vec<CargoCommand>, CommandPlanError> {
    let packages = cuda_feature_package_inventory(metadata)
        .map_err(|issues| CommandPlanError::inventory("exact CUDA feature ownership", issues))?;
    let hardware_targets = cuda_hardware_target_inventory(metadata)
        .map_err(|issues| CommandPlanError::inventory("exact CUDA hardware ownership", issues))?;

    let mut commands = vec![
        CargoCommand::packages(
            &["check", "--locked"],
            &packages,
            &["--all-targets", "--features", CUDA_FEATURE],
        ),
        CargoCommand::packages(
            &["test", "--locked"],
            &packages,
            &["--features", CUDA_FEATURE, "--no-run"],
        ),
    ];
    commands.extend(
        hardware_targets.iter().map(|target| {
            CargoCommand::hardware_target(&["test", "--locked"], target, &["--no-run"])
        }),
    );
    Ok(commands)
}

/// Loads locked Cargo metadata and plans all strict CUDA Clippy invocations.
///
/// The plan lints every package declaring the exact `cuda` feature together, then lints each exact
/// CUDA hardware target separately with warnings denied.
///
/// # Errors
///
/// Returns an error if locked metadata cannot be loaded, no exact `cuda` owner exists, or any CUDA
/// hardware target declaration is invalid.
pub fn cuda_clippy_command_plan(
    manifest_path: &Path,
) -> Result<Vec<CargoCommand>, CommandPlanError> {
    let metadata = load_metadata(manifest_path, true)
        .map_err(|error| CommandPlanError::metadata("CUDA Clippy planning", error))?;
    cuda_clippy_command_plan_for_metadata(&metadata)
}

/// Plans all strict CUDA Clippy invocations from already loaded Cargo metadata.
///
/// # Errors
///
/// Returns an error if no exact `cuda` owner exists or any CUDA hardware target declaration is
/// invalid.
pub fn cuda_clippy_command_plan_for_metadata(
    metadata: &Metadata,
) -> Result<Vec<CargoCommand>, CommandPlanError> {
    let packages = cuda_feature_package_inventory(metadata)
        .map_err(|issues| CommandPlanError::inventory("exact CUDA feature ownership", issues))?;
    let hardware_targets = cuda_hardware_target_inventory(metadata)
        .map_err(|issues| CommandPlanError::inventory("exact CUDA hardware ownership", issues))?;

    let mut commands = vec![CargoCommand::packages(
        &["clippy", "--locked"],
        &packages,
        &[
            "--all-targets",
            "--features",
            CUDA_FEATURE,
            "--",
            "-D",
            "warnings",
        ],
    )];
    commands.extend(hardware_targets.iter().map(|target| {
        CargoCommand::hardware_target(&["clippy", "--locked"], target, &["--", "-D", "warnings"])
    }));
    Ok(commands)
}

/// Loads locked Cargo metadata and plans all release CUDA hardware-suite invocations.
///
/// Every exact harness-free `cuda_hardware` target runs separately. The generated commands do not
/// append libtest filters or arguments, and process execution inherits the caller's environment.
///
/// # Errors
///
/// Returns an error if locked metadata cannot be loaded, no exact hardware target exists, or any
/// candidate hardware target declaration is invalid.
pub fn cuda_hardware_command_plan(
    manifest_path: &Path,
) -> Result<Vec<CargoCommand>, CommandPlanError> {
    let metadata = load_metadata(manifest_path, true)
        .map_err(|error| CommandPlanError::metadata("CUDA hardware planning", error))?;
    cuda_hardware_command_plan_for_metadata(&metadata)
}

/// Plans all release CUDA hardware-suite invocations from already loaded Cargo metadata.
///
/// # Errors
///
/// Returns an error if no exact hardware target exists or any candidate target declaration is
/// invalid.
pub fn cuda_hardware_command_plan_for_metadata(
    metadata: &Metadata,
) -> Result<Vec<CargoCommand>, CommandPlanError> {
    hardware_profile_command_plan_for_metadata(metadata, CUDA_FEATURE)
}

/// Loads locked Cargo metadata and plans one declared hardware profile.
///
/// # Errors
///
/// Returns an error if the profile is unknown or any registered suite is invalid.
pub fn hardware_profile_command_plan(
    manifest_path: &Path,
    profile: &str,
) -> Result<Vec<CargoCommand>, CommandPlanError> {
    let metadata = load_metadata(manifest_path, true)
        .map_err(|error| CommandPlanError::metadata("hardware profile planning", error))?;
    hardware_profile_command_plan_for_metadata(&metadata, profile)
}

/// Plans all release test invocations registered for one hardware profile.
///
/// # Errors
///
/// Returns an error if the profile is unknown or any registered suite is invalid.
pub fn hardware_profile_command_plan_for_metadata(
    metadata: &Metadata,
    profile: &str,
) -> Result<Vec<CargoCommand>, CommandPlanError> {
    let hardware_targets = hardware_suite_inventory(metadata, profile).map_err(|issues| {
        CommandPlanError::inventory(&format!("`{profile}` hardware profile ownership"), issues)
    })?;
    Ok(hardware_targets
        .iter()
        .map(|target| {
            CargoCommand::hardware_target(
                &["test", "--release", "--locked"],
                target,
                target.runner().execution_arguments(),
            )
        })
        .collect())
}

/// Returns whether a portable target name is maintained by the command planner.
#[must_use]
pub fn is_supported_portable_target(target: &str) -> bool {
    PORTABLE_TARGETS.contains(&target)
}

fn validate_portable_target(target: &str) -> Result<(), CommandPlanError> {
    if is_supported_portable_target(target) {
        Ok(())
    } else {
        Err(CommandPlanError::invalid(format!(
            "unsupported portable target `{target}`; expected {} or {}",
            PORTABLE_TARGETS[0], PORTABLE_TARGETS[1]
        )))
    }
}
