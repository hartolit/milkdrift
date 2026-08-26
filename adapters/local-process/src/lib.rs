//! Safe argument-vector local process capability adapter.
//!
//! The crate owns OS process/filesystem interaction but no runtime state or redb layout.
//! Durable input/output access is injected through `milkdrift-capability-host`.

mod config;
mod process;

pub use config::{
    CapturePolicy, EnvironmentPolicy, FilesystemAccessMode, FilesystemRoot, InputFileRule,
    MAX_PROCESS_PROFILE_BYTES, OutputRule, OverflowAction, PROCESS_PROFILE_SCHEMA_VERSION_V1,
    PlatformSupport, ProcessLimits, ProcessProfile, ProcessProfileDocument, ProcessProfileError,
    RestartPolicy, StdinMode, SubstitutionSource, WorkingDirectoryMode,
};
pub use process::LocalProcessAdapter;
