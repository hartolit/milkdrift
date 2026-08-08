//! Typed JSON report schema, separate from benchmark execution state.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::Serialize;

use crate::fixture::FixtureIdentity;
use crate::memory::ProcessMemory;

pub(crate) const SCHEMA_VERSION: u32 = 3;

#[derive(Serialize)]
pub(crate) struct BaselineReport {
    pub(crate) schema_version: u32,
    pub(crate) metadata: RunMetadata,
    pub(crate) results: BaselineResults,
}

#[derive(Serialize)]
pub(crate) struct RunMetadata {
    pub(crate) git: GitMetadata,
    pub(crate) toolchain: ToolchainMetadata,
    pub(crate) system: SystemMetadata,
    pub(crate) fixture: SyntheticFixtureMetadata,
    pub(crate) workload: WorkloadMetadata,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct GitMetadata {
    pub(crate) head: String,
    pub(crate) head_tree: String,
    pub(crate) dirty: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct ToolchainMetadata {
    pub(crate) rust_version: String,
    pub(crate) cargo_version: String,
    pub(crate) llvm_version: Option<String>,
    pub(crate) rustc_host: String,
    pub(crate) build_profile: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct SystemMetadata {
    pub(crate) os: &'static str,
    pub(crate) kernel: String,
    pub(crate) cpu_model: Option<String>,
    pub(crate) physical_cpu_count: Option<usize>,
    pub(crate) logical_cpu_count: Option<usize>,
    pub(crate) total_memory_bytes: Option<u64>,
    pub(crate) thread_environment: BTreeMap<String, String>,
}

#[derive(Serialize)]
pub(crate) struct SyntheticFixtureMetadata {
    pub(crate) identity: FixtureIdentity,
    pub(crate) backend: &'static str,
    pub(crate) architecture: &'static str,
    pub(crate) format: &'static str,
    pub(crate) vocabulary_size: u32,
    pub(crate) context_capacity: u32,
}

#[derive(Serialize)]
pub(crate) struct WorkloadMetadata {
    pub(crate) warmup_cycles: u32,
    pub(crate) sample_cycles: u32,
    pub(crate) checked_prefill_prompt_tokens: u32,
    pub(crate) generation_prompt_tokens: u32,
    pub(crate) first_token_generation_limit: u32,
    pub(crate) post_first_token_window: u32,
    pub(crate) backpressure_generation_limit: u32,
    pub(crate) backpressure_hold_milliseconds: u64,
    pub(crate) cancellation_generation_limit: u32,
    pub(crate) cancellation_hold_milliseconds: u64,
    pub(crate) sampling_strategy: &'static str,
}

#[derive(Serialize)]
pub(crate) struct BaselineResults {
    pub(crate) synthetic_e0: CycleSet<SyntheticCycle>,
    pub(crate) application_lifecycle: CycleSet<ApplicationLifecycleCycle>,
}

#[derive(Serialize)]
pub(crate) struct CycleSet<T> {
    pub(crate) warmups: Vec<T>,
    pub(crate) samples: Vec<T>,
}

#[derive(Serialize)]
pub(crate) struct ApplicationLifecycleCycle {
    pub(crate) start_ns: u64,
    pub(crate) shutdown_ns: u64,
    pub(crate) rss_before_start: ProcessMemory,
    pub(crate) rss_after_start: ProcessMemory,
    pub(crate) rss_after_shutdown: ProcessMemory,
}

#[derive(Serialize)]
pub(crate) struct SyntheticCycle {
    pub(crate) e0_start_ns: u64,
    pub(crate) model_load_ns: u64,
    pub(crate) load_evidence: SyntheticLoadEvidence,
    pub(crate) checked_prefill_ns: u64,
    pub(crate) first_token_ns: u64,
    pub(crate) post_first_token_proxy_ns: u64,
    pub(crate) backpressure: BackpressureMeasurement,
    pub(crate) cancellation: CancellationMeasurement,
    pub(crate) model_unload_ns: u64,
    pub(crate) shutdown: ShutdownMeasurement,
    pub(crate) snapshots: Vec<SnapshotCheckpoint>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct SyntheticLoadEvidence {
    pub(crate) prepared: PreparedLoadRecord,
    pub(crate) receipt: E0LoadReceiptRecord,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct PreparedLoadRecord {
    pub(crate) configuration_declared_scalar: Option<&'static str>,
    pub(crate) observed_tensor_scalars: Vec<&'static str>,
    pub(crate) planned_execution_scalar: &'static str,
    pub(crate) planned_execution_device: ExecutionDeviceRecord,
    pub(crate) exact_final_footprint: MemoryFootprintRecord,
    pub(crate) loading_peak_footprint: MemoryFootprintRecord,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct E0LoadReceiptRecord {
    pub(crate) actual_execution_scalar: &'static str,
    pub(crate) actual_execution_device: ExecutionDeviceRecord,
    pub(crate) reserved_footprint: MemoryFootprintRecord,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct ExecutionDeviceRecord {
    pub(crate) kind: &'static str,
    pub(crate) id: u64,
}

#[derive(Serialize)]
pub(crate) struct BackpressureMeasurement {
    pub(crate) controlled_hold_ns: u64,
    pub(crate) recovery_to_next_token_ns: u64,
}

#[derive(Serialize)]
pub(crate) struct CancellationMeasurement {
    pub(crate) generated_tokens: u32,
    pub(crate) acknowledgement_ns: u64,
    pub(crate) terminal_ns: u64,
    pub(crate) released_ns: u64,
}

#[expect(
    clippy::struct_field_names,
    reason = "nanosecond suffixes are explicit serialized units"
)]
#[derive(Serialize)]
pub(crate) struct ShutdownMeasurement {
    pub(crate) event_ns: u64,
    pub(crate) join_ns: u64,
    pub(crate) total_ns: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct SnapshotCheckpoint {
    pub(crate) checkpoint: &'static str,
    pub(crate) process_memory: ProcessMemory,
    pub(crate) runtime: RuntimeAccounting,
    pub(crate) models: Vec<ModelAccounting>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct RuntimeAccounting {
    pub(crate) loaded_models: u32,
    pub(crate) active_requests: u32,
    pub(crate) reserved_footprint: MemoryFootprintRecord,
    pub(crate) generation_workspaces: u32,
    pub(crate) reserved_generation_workspace: MemoryFootprintRecord,
    pub(crate) pending_cleanup_models: u32,
    pub(crate) pending_cleanup_sequences: u32,
    pub(crate) exhausted_cleanup_models: u32,
    pub(crate) exhausted_cleanup_sequences: u32,
    pub(crate) last_cleanup_present: bool,
    pub(crate) maintenance_error_present: bool,
    pub(crate) shutting_down: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct MemoryFootprintRecord {
    pub(crate) host_weight_bytes: u64,
    pub(crate) device_weight_bytes: u64,
    pub(crate) host_working_bytes: u64,
    pub(crate) device_working_bytes: u64,
    pub(crate) cache_bytes_per_token: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct ModelAccounting {
    pub(crate) model_id: u64,
    pub(crate) generation: u64,
    pub(crate) lifecycle: &'static str,
    pub(crate) reserved_footprint: MemoryFootprintRecord,
    pub(crate) active_requests: u32,
    pub(crate) pending_cleanup_sequences: u32,
    pub(crate) exhausted_cleanup_sequences: u32,
    pub(crate) degraded: bool,
}

pub(crate) fn duration_ns(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::{
        E0LoadReceiptRecord, ExecutionDeviceRecord, MemoryFootprintRecord, PreparedLoadRecord,
        SCHEMA_VERSION, SyntheticLoadEvidence,
    };

    const CPU: ExecutionDeviceRecord = ExecutionDeviceRecord { kind: "cpu", id: 0 };

    const fn footprint(host_weight_bytes: u64, host_working_bytes: u64) -> MemoryFootprintRecord {
        MemoryFootprintRecord {
            host_weight_bytes,
            device_weight_bytes: 0,
            host_working_bytes,
            device_working_bytes: 0,
            cache_bytes_per_token: 64,
        }
    }

    #[test]
    fn schema_three_serializes_prepared_and_actual_e0_load_facts_separately() -> Result<(), String>
    {
        let evidence = SyntheticLoadEvidence {
            prepared: PreparedLoadRecord {
                configuration_declared_scalar: Some("F32"),
                observed_tensor_scalars: vec!["F32"],
                planned_execution_scalar: "F32",
                planned_execution_device: CPU,
                exact_final_footprint: footprint(4_000, 0),
                loading_peak_footprint: footprint(4_000, 800),
            },
            receipt: E0LoadReceiptRecord {
                actual_execution_scalar: "F32",
                actual_execution_device: CPU,
                reserved_footprint: footprint(4_000, 0),
            },
        };
        let value = serde_json::to_value(evidence).map_err(|error| error.to_string())?;
        assert_eq!(SCHEMA_VERSION, 3);
        assert_eq!(
            value_at(&value, &["prepared", "configuration_declared_scalar"])?.as_str(),
            Some("F32")
        );
        let observed = value_at(&value, &["prepared", "observed_tensor_scalars"])?
            .as_array()
            .ok_or_else(|| "observed tensor scalars were not an array".to_owned())?;
        assert_eq!(observed.first().and_then(Value::as_str), Some("F32"));
        assert_eq!(
            value_at(&value, &["prepared", "planned_execution_scalar"])?.as_str(),
            Some("F32")
        );
        assert_eq!(
            value_at(&value, &["receipt", "actual_execution_scalar"])?.as_str(),
            Some("F32")
        );
        assert_eq!(
            value_at(
                &value,
                &["prepared", "exact_final_footprint", "host_weight_bytes"],
            )?
            .as_u64(),
            Some(4_000)
        );
        assert_eq!(
            value_at(
                &value,
                &["prepared", "loading_peak_footprint", "host_working_bytes",],
            )?
            .as_u64(),
            Some(800)
        );
        assert_eq!(
            value_at(
                &value,
                &["receipt", "reserved_footprint", "host_weight_bytes"],
            )?
            .as_u64(),
            Some(4_000)
        );
        assert!(find_key(&value, "scalar_type").is_none());
        Ok(())
    }

    #[test]
    fn synthetic_schema_history_remains_documented_without_a_legacy_parser() {
        let readme = include_str!("../README.md");
        assert!(readme.contains("**Synthetic schema 1 (historical):**"));
        assert!(readme.contains("**Synthetic schema 2 (historical):**"));
        assert!(readme.contains("**Synthetic schema 3 (current):**"));
    }

    fn value_at<'a>(value: &'a Value, path: &[&str]) -> Result<&'a Value, String> {
        let mut current = value;
        for key in path {
            current = current
                .get(*key)
                .ok_or_else(|| format!("serialized evidence omitted key {key:?} in {path:?}"))?;
        }
        Ok(current)
    }

    fn find_key<'a>(value: &'a Value, expected: &str) -> Option<&'a Value> {
        match value {
            Value::Object(fields) => fields.iter().find_map(|(key, nested)| {
                (key == expected)
                    .then_some(nested)
                    .or_else(|| find_key(nested, expected))
            }),
            Value::Array(values) => values.iter().find_map(|nested| find_key(nested, expected)),
            _ => None,
        }
    }
}
