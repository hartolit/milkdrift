use std::fs;

use milkdrift_capability::{ArtifactReference, InvocationEventKind, InvocationRequest};
use milkdrift_capability_host::{
    AdapterExecutionContext, AdapterReporter, InvocationDataAccess, MaterializedExecution,
};

use crate::config::ProcessProfile;

use super::{bounded, reporting::report};

#[allow(clippy::too_many_arguments)]
pub(super) fn publish(
    profile: &ProcessProfile,
    data: &dyn InvocationDataAccess,
    context: &AdapterExecutionContext,
    request: &InvocationRequest,
    workspace: &dyn MaterializedExecution,
    stdout: &[u8],
    stderr: &[u8],
    reporter: &dyn AdapterReporter,
    sequence: &mut u64,
) -> Result<Vec<ArtifactReference>, String> {
    let mut planned_count = profile.outputs.len();
    planned_count = planned_count
        .saturating_add(usize::from(profile.stdout.artifact_name.is_some()))
        .saturating_add(usize::from(profile.stderr.artifact_name.is_some()));
    if planned_count > usize::from(profile.limits.max_output_files) {
        return Err("declared output count exceeds the publication bound".to_owned());
    }
    let mut total = u64::try_from(stdout.len())
        .ok()
        .and_then(|value| value.checked_add(u64::try_from(stderr.len()).ok()?))
        .ok_or_else(|| "captured output byte accounting overflow".to_owned())?;
    for output in &profile.outputs {
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
    if total > profile.limits.max_total_output_bytes {
        return Err("declared outputs exceed the aggregate publication bound".to_owned());
    }
    let mut outputs = Vec::new();
    for (capture, bytes) in [(&profile.stdout, stdout), (&profile.stderr, stderr)] {
        if let Some(name) = &capture.artifact_name {
            let reference = data
                .publish_bytes(
                    context,
                    request,
                    name,
                    "application/octet-stream",
                    bytes,
                    profile.limits.materialization(),
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
    for output in &profile.outputs {
        if !output.required
            && fs::symlink_metadata(workspace.root().join(&output.relative_path))
                .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
        {
            continue;
        }
        let reference = data
            .publish_file(
                context,
                request,
                workspace,
                &output.name,
                &output.relative_path,
                &output.media_type,
                profile.limits.materialization(),
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
