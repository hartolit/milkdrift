//! Slint frontend startup and platform failures.

use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::path::PathBuf;

use application_runtime::ApplicationError;

/// Failure while starting, running, or stopping the Slint frontend.
#[derive(Debug)]
pub enum DesktopError {
    /// Frontend-neutral application runtime failed.
    Application(ApplicationError),
    /// Slint platform or event-loop operation failed.
    Slint(slint::PlatformError),
    /// Slint failed and explicit application shutdown also reported a failure.
    SlintAndShutdown {
        /// Primary Slint construction or event-loop failure.
        slint: slint::PlatformError,
        /// Failure reported while explicitly stopping application workers.
        shutdown: ApplicationError,
    },
    /// A supported per-user data directory could not be resolved.
    MissingDataDirectory,
    /// The application-state directory could not be created.
    CreateDataDirectory(std::io::Error),
    /// Existing state could not be migrated from the legacy application directory.
    MigrateApplicationState {
        /// Existing legacy database path.
        legacy: PathBuf,
        /// Current Milkdrift database path.
        current: PathBuf,
        /// Filesystem failure that prevented inspection or migration.
        source: std::io::Error,
    },
}

impl Display for DesktopError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Application(error) => Display::fmt(error, formatter),
            Self::Slint(error) => write!(formatter, "Slint failure: {error}"),
            Self::SlintAndShutdown { slint, shutdown } => write!(
                formatter,
                "Slint failure: {slint}; application shutdown also failed: {shutdown}",
            ),
            Self::MissingDataDirectory => {
                formatter.write_str("no supported per-user application data directory is available")
            }
            Self::CreateDataDirectory(error) => {
                write!(
                    formatter,
                    "failed to create application data directory: {error}"
                )
            }
            Self::MigrateApplicationState {
                legacy,
                current,
                source,
            } => write!(
                formatter,
                "failed to migrate application state from {} to {}: {source}",
                legacy.display(),
                current.display(),
            ),
        }
    }
}

impl Error for DesktopError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Application(error) => Some(error),
            Self::Slint(error) | Self::SlintAndShutdown { slint: error, .. } => Some(error),
            Self::CreateDataDirectory(error) => Some(error),
            Self::MigrateApplicationState { source, .. } => Some(source),
            Self::MissingDataDirectory => None,
        }
    }
}

impl From<ApplicationError> for DesktopError {
    fn from(value: ApplicationError) -> Self {
        Self::Application(value)
    }
}

impl From<slint::PlatformError> for DesktopError {
    fn from(value: slint::PlatformError) -> Self {
        Self::Slint(value)
    }
}
