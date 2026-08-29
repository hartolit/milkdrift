use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

use milkdrift_capability::{InvocationRequest, InvocationValueReference};
use milkdrift_capability_host::MaterializedExecution;

use crate::config::{
    ProcessProfile, StdinMode, SubstitutionSource, WorkingDirectoryMode, placeholders,
};

pub(super) fn materialize_arguments(
    profile: &ProcessProfile,
    request: &InvocationRequest,
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
                        .map_err(|error| super::bounded(&error.to_string()))?,
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
        for name in placeholders(template).map_err(|error| super::bounded(&error.to_string()))? {
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

pub(super) fn stdin_bytes(
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

pub(super) fn prepare_working_directory(
    root: &Path,
    mode: &WorkingDirectoryMode,
) -> Result<PathBuf, String> {
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
