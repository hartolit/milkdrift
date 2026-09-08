//! Run a byte-pinned executable as a workflow capability using direct OS arguments.
//!
//! Read a [`ProcessProfileDocument`], then construct [`LocalProcessAdapter`] with explicit data
//! and secret ports. Construction verifies executable identity and host paths; register the
//! adapter's descriptor with the capability host. [`WorkingDirectoryMode`] chooses temporary
//! execution or an authorized persistent repository, while [`CapturePolicy`] and [`OutputRule`]
//! choose which results can be published.
//!
//! The child has the daemon account's privileges. Input staging and direct argv do not create
//! a sandbox; [`PlatformSupport`] describes the process ownership this build can observe.

mod config;
mod process;

pub use config::{
    CapturePolicy, EnvironmentPolicy, ExecutableIdentityDeclaration, FilesystemAccessMode,
    FilesystemRoot, InputFileRule, MAX_EXECUTABLE_BYTES, MAX_PROCESS_PROFILE_BYTES, OutputRule,
    OverflowAction, PlatformSupport, ProcessLimits, ProcessProfile, ProcessProfileDocument,
    ProcessProfileError, RestartPolicy, StdinMode, SubstitutionSource, WorkingDirectoryMode,
};
pub use process::LocalProcessAdapter;
