use super::*;

pub(crate) const BACKEND_ID: BackendId = BackendId::new(92);
pub(crate) const FAILED_LOAD_LOCATION: TensorFailureLocation =
    TensorFailureLocation::new(2, 7, 0x0123_4567_89ab_cdef, Some(ScalarType::F16));

pub(crate) const fn failed_load_error() -> LoadError {
    LoadError::Backend(BackendLoadFailure::at_tensor(
        backend_failure(5),
        LoadFailureStage::DeviceTransfer,
        FAILED_LOAD_LOCATION,
    ))
}

pub(crate) type TestResult = Result<(), String>;

#[derive(Clone, Copy, Default)]
pub(crate) struct Faults(u64);

impl Faults {
    pub(crate) const WRONG_MODEL_HANDLE: Self = Self(1 << 0);
    pub(crate) const MISMATCHED_METADATA: Self = Self(1 << 1);
    pub(crate) const FAIL_MODEL_CLEANUP: Self = Self(1 << 2);
    pub(crate) const CONTRADICTORY_SEQUENCE_PLAN: Self = Self(1 << 3);
    pub(crate) const WRONG_SEQUENCE_ID: Self = Self(1 << 4);
    pub(crate) const WRONG_SEQUENCE_CAPACITY: Self = Self(1 << 5);
    pub(crate) const FAIL_SEQUENCE_DESTRUCTION: Self = Self(1 << 6);
    pub(crate) const MISMATCHED_DESCRIPTOR: Self = Self(1 << 7);
    pub(crate) const MISSING_MULTIPLE_SEQUENCES: Self = Self(1 << 8);
    pub(crate) const ZERO_VOCABULARY: Self = Self(1 << 9);
    pub(crate) const ZERO_CONTEXT_LENGTH: Self = Self(1 << 10);
    pub(crate) const ZERO_MAXIMUM_CONTEXT: Self = Self(1 << 11);
    pub(crate) const ZERO_MAXIMUM_SEQUENCES: Self = Self(1 << 12);
    pub(crate) const ZERO_MAXIMUM_PREFILL: Self = Self(1 << 13);
    pub(crate) const CONTEXT_EXCEEDS_METADATA: Self = Self(1 << 14);
    pub(crate) const PREFILL_EXCEEDS_CONTEXT: Self = Self(1 << 15);
    pub(crate) const WRONG_DEVICE_ID: Self = Self(1 << 16);
    pub(crate) const WRONG_DEVICE_KIND: Self = Self(1 << 17);
    pub(crate) const WRONG_MODEL_FOOTPRINT: Self = Self(1 << 18);
    pub(crate) const WRONG_EXECUTION_SCALAR: Self = Self(1 << 19);
    pub(crate) const SOURCE_SCALAR_AS_EXECUTION_SCALAR: Self = Self(1 << 20);
    pub(crate) const UNSUPPORTED_ACTUAL_EXECUTION_SCALAR: Self = Self(1 << 21);
    pub(crate) const FAIL_MODEL_CLEANUP_ONCE: Self = Self(1 << 22);
    pub(crate) const WRONG_ACCEPTED_CONFIGURATION: Self = Self(1 << 23);
    pub(crate) const EMPTY_OBSERVED_TENSOR_SET: Self = Self(1 << 24);
    pub(crate) const OVERFLOWING_FINAL_FOOTPRINT: Self = Self(1 << 25);
    pub(crate) const LOADING_PEAK_BELOW_FINAL: Self = Self(1 << 26);
    pub(crate) const FAIL_LOAD: Self = Self(1 << 28);
    pub(crate) const FAIL_FAILED_LOAD_CLEANUP: Self = Self(1 << 29);
    pub(crate) const FAIL_FAILED_LOAD_CLEANUP_ONCE: Self = Self(1 << 30);
    pub(crate) const OVERFLOWING_LOADING_PEAK: Self = Self(1 << 31);
    pub(crate) const RECLASSIFIED_LOADING_PEAK: Self = Self(1 << 32);
    pub(crate) const REPORTED_LARGER_THAN_PEAK: Self = Self(1 << 33);
    pub(crate) const REPORTED_RECLASSIFIED_TO_DEVICE: Self = Self(1 << 34);
    pub(crate) const REPORTED_OVERFLOWING_HOST: Self = Self(1 << 35);
    pub(crate) const REPORTED_OVERFLOWING_DEVICE: Self = Self(1 << 36);
    pub(crate) const REPORTED_SMALLER_THAN_FINAL: Self = Self(1 << 37);
    pub(crate) const MUTATE_MODEL_REPORT_ON_CLEANUP_FAILURE: Self = Self(1 << 38);
    pub(crate) const MUTATE_FAILED_PLAN_ON_CLEANUP_FAILURE: Self = Self(1 << 39);
    pub(crate) const ALTERNATING_PLAN_REPORT: Self = Self(1 << 40);
    pub(crate) const FAIL_MODEL_CLEANUP_TWICE: Self = Self(1 << 41);
    pub(crate) const UNDERREPORTED_SEQUENCE_REPORT: Self = Self(1 << 42);
    pub(crate) const OVERREPORTED_SEQUENCE_REPORT: Self = Self(1 << 43);
    pub(crate) const RECLASSIFIED_SEQUENCE_REPORT: Self = Self(1 << 44);
    pub(crate) const MUTATE_SEQUENCE_REPORT_ON_CLEANUP_FAILURE: Self = Self(1 << 47);
    pub(crate) const MUTATE_SEQUENCE_REPORT_AFTER_PREFILL: Self = Self(1 << 48);
    pub(crate) const MUTATE_SEQUENCE_ID_ON_CLEANUP_FAILURE: Self = Self(1 << 49);
    pub(crate) const MUTATE_SEQUENCE_CAPACITY_ON_CLEANUP_FAILURE: Self = Self(1 << 50);
    pub(crate) const WRONG_INITIAL_SEQUENCE_STATE: Self = Self(1 << 51);
    pub(crate) const WRONG_INITIAL_SEQUENCE_POSITION: Self = Self(1 << 52);

    pub(crate) const fn contains(self, fault: Self) -> bool {
        self.0 & fault.0 != 0
    }

    pub(crate) const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

#[derive(Clone, Copy)]
pub(crate) struct FaultSource {
    pub(crate) source_scalar_type: ScalarType,
    pub(crate) planned_execution_scalar_type: ScalarType,
    pub(crate) faults: Faults,
}

pub(crate) const DEFAULT_SOURCE: FaultSource = FaultSource {
    source_scalar_type: ScalarType::F32,
    planned_execution_scalar_type: ScalarType::F32,
    faults: Faults(0),
};
pub(crate) const BF16_SOURCE_WITH_F32_EXECUTION: FaultSource = FaultSource {
    source_scalar_type: ScalarType::Bf16,
    planned_execution_scalar_type: ScalarType::F32,
    faults: Faults(0),
};

pub(crate) const fn backend_failure(code: u32) -> BackendFailure {
    BackendFailure::new(BACKEND_ID, BackendFailureKind::Internal, code)
}
