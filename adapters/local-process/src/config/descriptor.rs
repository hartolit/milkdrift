//! Descriptor bytes and policy digests are derived from the validated immutable profile.
use super::{
    PROCESS_PROFILE_SCHEMA_VERSION_V2, ProcessProfile, ProcessProfileError,
    VerifiedExecutableIdentity, WorkingDirectoryMode,
};
use milkdrift_capability::{
    AdmissionConstraints, BoundedJson, CapabilityCategory, CapabilityDescriptor, DescriptorBuilder,
    ExtensionKey, FeatureContract, FeatureId, Locality, OperationContract, SchemaContract,
    SchemaId, StreamingMode, TrustZone,
};
use milkdrift_contracts::{JsonLimits, canonical_json_bytes};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

impl ProcessProfile {
    /// Generates an immutable, capability-host-compatible descriptor.
    pub(crate) fn descriptor(
        &self,
        implementation: &VerifiedExecutableIdentity,
    ) -> Result<CapabilityDescriptor, ProcessProfileError> {
        let profile_digest = self.profile_digest()?;
        let execution_policy_digest = self.execution_policy_digest()?;
        let schema_value = BoundedJson::new(json!({
            "type": "object",
            "additionalProperties": true,
            "x-milkdrift-process-profile-schema": PROCESS_PROFILE_SCHEMA_VERSION_V2
        }))
        .map_err(|error| ProcessProfileError::Descriptor(error.to_string()))?;
        let input = SchemaContract::new(
            SchemaId::new("milkdrift.process.input")
                .map_err(|error| ProcessProfileError::Descriptor(error.to_string()))?,
            1,
            schema_value.clone(),
        )
        .map_err(|error| ProcessProfileError::Descriptor(error.to_string()))?;
        let output = SchemaContract::new(
            SchemaId::new("milkdrift.process.output")
                .map_err(|error| ProcessProfileError::Descriptor(error.to_string()))?,
            1,
            schema_value,
        )
        .map_err(|error| ProcessProfileError::Descriptor(error.to_string()))?;
        let mut streaming = BTreeSet::new();
        streaming.insert(
            if self.stdout.stream_progress || self.stderr.stream_progress {
                StreamingMode::Progress
            } else {
                StreamingMode::None
            },
        );
        let feature_ids = [
            "milkdrift.process.argv",
            "milkdrift.process.materialized-inputs",
            "milkdrift.process.declared-outputs",
        ];
        let mut features = BTreeMap::new();
        for identity in feature_ids {
            let identity = FeatureId::new(identity)
                .map_err(|error| ProcessProfileError::Descriptor(error.to_string()))?;
            features.insert(identity.clone(), FeatureContract::new(identity, None));
        }
        if self.platform.owned_process_group {
            let identity = FeatureId::new("milkdrift.process.owned-group")
                .map_err(|error| ProcessProfileError::Descriptor(error.to_string()))?;
            features.insert(identity.clone(), FeatureContract::new(identity, None));
        }
        let operation = OperationContract::new(
            input,
            output,
            streaming,
            self.cancellation,
            self.idempotency,
            self.side_effect,
            features,
        )
        .map_err(|error| ProcessProfileError::Descriptor(error.to_string()))?;
        let extension_key = ExtensionKey::new("org.milkdrift/process-profile")
            .map_err(|error| ProcessProfileError::Descriptor(error.to_string()))?;
        let mut profile_extension = json!({
            "schema_version": PROCESS_PROFILE_SCHEMA_VERSION_V2,
            "profile_id": self.profile_id,
            "profile_revision": self.revision,
            "profile_digest": profile_digest,
            "execution_policy_digest": execution_policy_digest,
            "execution_trust": self.trust_class,
            "implementation": implementation,
            "owned_process_group": self.platform.owned_process_group,
            "descendant_escape_prevention": self.platform.descendant_escape_prevention,
            "terminal_group_observation": self.platform.terminal_group_observation,
            "resource_limits_are_observational": true
        });
        if matches!(self.working_directory, WorkingDirectoryMode::Project) {
            profile_extension
                .as_object_mut()
                .ok_or_else(|| {
                    ProcessProfileError::Descriptor("profile metadata is not an object".into())
                })?
                .insert("project_directory".into(), json!(cfg!(target_os = "linux")));
        }
        let extension_value = BoundedJson::new(profile_extension)
            .map_err(|error| ProcessProfileError::Descriptor(error.to_string()))?;
        DescriptorBuilder::new(
            self.capability.clone(),
            self.descriptor_revision,
            CapabilityCategory::Process,
            AdmissionConstraints::new(self.max_concurrent, 0)
                .map_err(|error| ProcessProfileError::Descriptor(error.to_string()))?,
            Locality::Local,
        )
        .provider_profile(self.provider_profile.clone())
        .operations(BTreeMap::from([(self.operation.clone(), operation)]))
        .execution_trust(self.trust_class)
        .trust_zones(BTreeSet::from([TrustZone::new("local-process").map_err(
            |error| ProcessProfileError::Descriptor(error.to_string()),
        )?]))
        .labels(BTreeSet::from(["trusted host process".to_owned()]))
        .extensions(BTreeMap::from([(extension_key, extension_value)]))
        .build()
        .map_err(|error| ProcessProfileError::Descriptor(error.to_string()))
    }
    fn profile_digest(&self) -> Result<String, ProcessProfileError> {
        let bytes = canonical_json_bytes(
            self,
            JsonLimits {
                maximum_depth: 48,
                maximum_string_bytes: 32_768,
                maximum_key_bytes: 192,
                maximum_container_items: 4_096,
            },
        )
        .map_err(|error| ProcessProfileError::Json(format!("{error:?}")))?;
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"milkdrift.process-profile.v2\0");
        hasher.update(&bytes);
        Ok(format!("b3_{}", hasher.finalize()))
    }

    fn execution_policy_digest(&self) -> Result<String, ProcessProfileError> {
        let policy = json!({
            "schema_version": PROCESS_PROFILE_SCHEMA_VERSION_V2,
            "operation": self.operation,
            "side_effect": self.side_effect,
            "idempotency": self.idempotency,
            "cancellation": self.cancellation,
            "trust_class": self.trust_class,
            "arguments": self.arguments,
            "substitutions": self.substitutions,
            "working_directory": self.working_directory,
            "filesystem_roots": self.filesystem_roots,
            "inputs": self.inputs,
            "environment": self.environment,
            "stdin": self.stdin,
            "stdout": self.stdout,
            "stderr": self.stderr,
            "outputs": self.outputs,
            "limits": self.limits,
            "restart": self.restart,
            "platform": self.platform,
            "max_concurrent": self.max_concurrent,
        });
        let bytes = canonical_json_bytes(
            &policy,
            JsonLimits {
                maximum_depth: 48,
                maximum_string_bytes: 32_768,
                maximum_key_bytes: 192,
                maximum_container_items: 4_096,
            },
        )
        .map_err(|error| ProcessProfileError::Json(format!("{error:?}")))?;
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"milkdrift.process-execution-policy.v1\0");
        hasher.update(&bytes);
        Ok(format!("b3_{}", hasher.finalize()))
    }
}
