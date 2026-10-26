//! Project task connector choices from the profile that owns process materialization.

use std::collections::BTreeMap;

use milkdrift_capability::{TaskAuthoringContract, TaskAuthoringPort};

use crate::config::{ProcessProfile, StdinMode, SubstitutionSource};

impl ProcessProfile {
    pub(crate) fn task_authoring(&self) -> TaskAuthoringContract {
        let mut inputs = BTreeMap::new();
        for input in &self.inputs {
            inputs.insert(input.input.clone(), self.limits.max_file_bytes);
        }
        for source in self.substitutions.values() {
            if let SubstitutionSource::InputText { input } = source {
                inputs
                    .entry(input.clone())
                    .and_modify(|bound| *bound = (*bound).min(32_768))
                    .or_insert(32_768);
            }
        }
        if let StdinMode::Input { input, max_bytes } = &self.stdin {
            inputs
                .entry(input.clone())
                .and_modify(|bound| *bound = (*bound).min(*max_bytes))
                .or_insert(*max_bytes);
        }
        let mut outputs = self
            .outputs
            .iter()
            .map(|output| TaskAuthoringPort {
                name: output.name.clone(),
                label: output.name.replace(['_', '-'], " "),
                required: output.required,
                accepts_artifacts: true,
                media_type: Some(output.media_type.clone()),
                max_bytes: self
                    .limits
                    .max_file_bytes
                    .min(self.limits.max_total_output_bytes),
            })
            .collect::<Vec<_>>();
        for (capture, label) in [
            (&self.stdout, "Standard output"),
            (&self.stderr, "Standard error"),
        ] {
            if let Some(name) = &capture.artifact_name {
                outputs.push(TaskAuthoringPort {
                    name: name.clone(),
                    label: label.into(),
                    required: true,
                    accepts_artifacts: true,
                    media_type: Some("application/octet-stream".into()),
                    max_bytes: capture.max_capture_bytes,
                });
            }
        }
        let default_output = match outputs.as_slice() {
            [output] => Some(output.name.clone()),
            _ => None,
        };
        TaskAuthoringContract {
            input_media: milkdrift_capability::TaskInputMedia::Any,
            label: self.profile_id.replace(['_', '-'], " "),
            inputs: inputs
                .into_iter()
                .map(|(name, max_bytes)| TaskAuthoringPort {
                    label: name.replace(['_', '-'], " "),
                    required: true,
                    accepts_artifacts: !self.substitutions.values().any(|source| matches!(source, SubstitutionSource::InputText { input } if input == &name)),
                    name,
                    media_type: None,
                    max_bytes,
                })
                .collect(),
            allows_additional_inputs: false,
            outputs,
            default_output,
        }
    }
}
