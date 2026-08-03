//! Internal configuration validation, type conversion, and worker construction.

use std::num::{NonZeroU32, NonZeroU64, NonZeroUsize};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use candle_backend::CandleScalarType;
use domain_contracts::MemoryBudget;
use hf_hub_adapter::{ArtifactScalarType, HubClientConfiguration};
use host_runtime::ThreadPanicked;
use inference_runtime::{HostedRuntimeConfiguration, RuntimeLimits};
use redb_storage::{
    ApplicationSettings, StoredAcceleratorMemoryPolicy, StoredApplicationDevice, StoredScalarType,
};

use crate::local::LocalInference;
use crate::{
    AcceleratorMemoryPolicy, ApplicationConfigurationField, ApplicationDevice,
    ApplicationDeviceSummary, ApplicationError, ApplicationFailure, ApplicationFailureKind,
    ApplicationHubConfiguration, ApplicationPreferences, ApplicationRuntimeConfiguration,
    ApplicationScalarType, ApplicationTiming,
};

pub const MAXIMUM_SHUTDOWN_OR_JOIN_TIMEOUT: Duration = Duration::from_hours(24);

pub fn hub_configuration(configuration: &ApplicationHubConfiguration) -> HubClientConfiguration {
    HubClientConfiguration {
        cache_directory: configuration.cache_directory.clone(),
        access_token: configuration.access_token.clone(),
        maximum_retries: configuration.maximum_retries,
    }
}

pub fn runtime_memory_budget(
    preferences: &ApplicationPreferences,
    devices: &[ApplicationDeviceSummary],
) -> MemoryBudget {
    let discovered_physical_capacity = devices
        .iter()
        .filter(|summary| matches!(summary.device(), ApplicationDevice::Cuda { .. }))
        .map(|summary| summary.total_memory_bytes().unwrap_or(0))
        .min()
        .unwrap_or(0);
    let device_bytes = match preferences.accelerator_memory_policy {
        AcceleratorMemoryPolicy::Automatic => discovered_physical_capacity,
        AcceleratorMemoryPolicy::Limit { bytes } => bytes.get().min(discovered_physical_capacity),
    };
    MemoryBudget {
        host_bytes: preferences.maximum_host_memory_bytes,
        device_bytes,
    }
}

pub fn create_runtime(
    memory_budget: MemoryBudget,
    configuration: &ApplicationRuntimeConfiguration,
) -> Result<LocalInference, ApplicationError> {
    let maximum_requests = NonZeroU32::new(configuration.maximum_requests).ok_or(
        ApplicationError::InvalidConfiguration(ApplicationConfigurationField::MaximumRequests),
    )?;
    let command_capacity = NonZeroUsize::new(configuration.command_capacity).ok_or(
        ApplicationError::InvalidConfiguration(ApplicationConfigurationField::CommandCapacity),
    )?;
    let event_capacity = NonZeroUsize::new(configuration.event_capacity).ok_or(
        ApplicationError::InvalidConfiguration(ApplicationConfigurationField::EventCapacity),
    )?;
    let token_output_capacity = NonZeroUsize::new(configuration.token_output_capacity).ok_or(
        ApplicationError::InvalidConfiguration(ApplicationConfigurationField::TokenOutputCapacity),
    )?;
    let token_output_record_capacity = NonZeroUsize::new(
        configuration.token_output_record_capacity,
    )
    .ok_or(ApplicationError::InvalidConfiguration(
        ApplicationConfigurationField::TokenOutputRecordCapacity,
    ))?;
    let poll_milliseconds = duration_milliseconds(
        configuration.timing.runtime_poll,
        ApplicationConfigurationField::RuntimePoll,
    )?;
    let poll = NonZeroU64::new(poll_milliseconds).ok_or(ApplicationError::InvalidConfiguration(
        ApplicationConfigurationField::RuntimePoll,
    ))?;
    let limits = RuntimeLimits::new(NonZeroU32::MIN, maximum_requests, memory_budget);
    let hosted = HostedRuntimeConfiguration::new(command_capacity, event_capacity, poll)
        .with_token_output_capacity(token_output_capacity, token_output_record_capacity);
    LocalInference::start(limits, hosted)
}

pub fn validate_configuration(
    configuration: &ApplicationRuntimeConfiguration,
) -> Result<(), ApplicationError> {
    validate_non_zero(
        &configuration.maximum_requests,
        ApplicationConfigurationField::MaximumRequests,
    )?;
    validate_non_zero(
        &configuration.command_capacity,
        ApplicationConfigurationField::CommandCapacity,
    )?;
    validate_non_zero(
        &configuration.event_capacity,
        ApplicationConfigurationField::EventCapacity,
    )?;
    validate_non_zero(
        &configuration.hub_channel_capacity,
        ApplicationConfigurationField::HubChannelCapacity,
    )?;
    validate_non_zero(
        &configuration.token_output_capacity,
        ApplicationConfigurationField::TokenOutputCapacity,
    )?;
    validate_non_zero(
        &configuration.token_output_record_capacity,
        ApplicationConfigurationField::TokenOutputRecordCapacity,
    )?;
    validate_non_zero(
        &configuration.text_output_byte_capacity,
        ApplicationConfigurationField::TextOutputByteCapacity,
    )?;
    validate_non_zero(
        &configuration.text_output_record_capacity,
        ApplicationConfigurationField::TextOutputRecordCapacity,
    )?;
    configuration
        .token_output_capacity
        .checked_add(configuration.token_output_record_capacity)
        .ok_or(ApplicationError::InvalidConfiguration(
            ApplicationConfigurationField::PendingGenerationOutputCapacity,
        ))?;
    validate_timing(&configuration.timing)
}

pub fn validate_preferences(preferences: &ApplicationPreferences) -> Result<(), ApplicationError> {
    if preferences.default_revision.trim().is_empty() {
        return Err(ApplicationError::InvalidConfiguration(
            ApplicationConfigurationField::DefaultRevision,
        ));
    }
    validate_non_zero(
        &preferences.drain_timeout_milliseconds,
        ApplicationConfigurationField::DrainTimeout,
    )
}

pub fn application_preferences(settings: ApplicationSettings) -> ApplicationPreferences {
    ApplicationPreferences {
        default_repository: settings.default_repository,
        default_revision: settings.default_revision,
        maximum_host_memory_bytes: settings.maximum_host_memory_bytes,
        selected_device: match settings.selected_device {
            StoredApplicationDevice::Cpu => ApplicationDevice::Cpu,
            StoredApplicationDevice::Cuda { ordinal } => ApplicationDevice::Cuda { ordinal },
        },
        accelerator_memory_policy: match settings.accelerator_memory_policy {
            StoredAcceleratorMemoryPolicy::Automatic => AcceleratorMemoryPolicy::Automatic,
            StoredAcceleratorMemoryPolicy::Limit { bytes } => {
                AcceleratorMemoryPolicy::Limit { bytes }
            }
        },
        drain_timeout_milliseconds: settings.drain_timeout_milliseconds,
    }
}

pub fn stored_settings(preferences: &ApplicationPreferences) -> ApplicationSettings {
    ApplicationSettings {
        default_repository: preferences.default_repository.clone(),
        default_revision: preferences.default_revision.clone(),
        maximum_host_memory_bytes: preferences.maximum_host_memory_bytes,
        selected_device: match preferences.selected_device {
            ApplicationDevice::Cpu => StoredApplicationDevice::Cpu,
            ApplicationDevice::Cuda { ordinal } => StoredApplicationDevice::Cuda { ordinal },
        },
        accelerator_memory_policy: match preferences.accelerator_memory_policy {
            AcceleratorMemoryPolicy::Automatic => StoredAcceleratorMemoryPolicy::Automatic,
            AcceleratorMemoryPolicy::Limit { bytes } => {
                StoredAcceleratorMemoryPolicy::Limit { bytes }
            }
        },
        drain_timeout_milliseconds: preferences.drain_timeout_milliseconds,
    }
}

pub const fn domain_scalar_type(value: ArtifactScalarType) -> ApplicationScalarType {
    match value {
        ArtifactScalarType::F32 => ApplicationScalarType::F32,
        ArtifactScalarType::F16 => ApplicationScalarType::F16,
        ArtifactScalarType::Bf16 => ApplicationScalarType::Bf16,
    }
}

pub const fn candle_scalar_type(value: ApplicationScalarType) -> CandleScalarType {
    match value {
        ApplicationScalarType::F32 => CandleScalarType::F32,
        ApplicationScalarType::F16 => CandleScalarType::F16,
        ApplicationScalarType::Bf16 => CandleScalarType::Bf16,
    }
}

pub const fn application_scalar_type(
    value: domain_contracts::ScalarType,
) -> Option<ApplicationScalarType> {
    match value {
        domain_contracts::ScalarType::F32 => Some(ApplicationScalarType::F32),
        domain_contracts::ScalarType::F16 => Some(ApplicationScalarType::F16),
        domain_contracts::ScalarType::Bf16 => Some(ApplicationScalarType::Bf16),
        _ => None,
    }
}

pub const fn stored_scalar_type(value: ArtifactScalarType) -> StoredScalarType {
    match value {
        ArtifactScalarType::F32 => StoredScalarType::F32,
        ArtifactScalarType::F16 => StoredScalarType::F16,
        ArtifactScalarType::Bf16 => StoredScalarType::Bf16,
    }
}

pub fn unix_milliseconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

pub fn hub_failure(error: hf_hub_adapter::HubError) -> ApplicationError {
    ApplicationFailure::new(ApplicationFailureKind::Hub, error).into()
}

pub fn storage_failure(error: redb_storage::StorageError) -> ApplicationError {
    ApplicationFailure::new(ApplicationFailureKind::Storage, error).into()
}

pub fn model_source_failure(error: candle_backend::SourceError) -> ApplicationError {
    ApplicationFailure::new(ApplicationFailureKind::ModelSource, error).into()
}

pub fn thread_failure(error: ThreadPanicked) -> ApplicationError {
    ApplicationFailure::new(ApplicationFailureKind::Worker, error).into()
}

fn validate_timing(timing: &ApplicationTiming) -> Result<(), ApplicationError> {
    let fields = [
        (
            timing.runtime_poll,
            ApplicationConfigurationField::RuntimePoll,
        ),
        (
            timing.hub_worker_poll,
            ApplicationConfigurationField::HubWorkerPoll,
        ),
        (
            timing.hub_event_send_timeout,
            ApplicationConfigurationField::HubEventSendTimeout,
        ),
        (
            timing.hub_command_shutdown_timeout,
            ApplicationConfigurationField::HubCommandShutdownTimeout,
        ),
        (
            timing.runtime_shutdown_timeout,
            ApplicationConfigurationField::RuntimeShutdownTimeout,
        ),
        (
            timing.runtime_shutdown_event_poll,
            ApplicationConfigurationField::RuntimeShutdownEventPoll,
        ),
        (
            timing.runtime_join_timeout,
            ApplicationConfigurationField::RuntimeJoinTimeout,
        ),
        (
            timing.runtime_join_poll,
            ApplicationConfigurationField::RuntimeJoinPoll,
        ),
        (
            timing.hub_shutdown_timeout,
            ApplicationConfigurationField::HubShutdownTimeout,
        ),
        (
            timing.hub_shutdown_poll,
            ApplicationConfigurationField::HubShutdownPoll,
        ),
    ];
    for (duration, field) in fields {
        if duration.is_zero() {
            return Err(ApplicationError::InvalidConfiguration(field));
        }
    }

    let deadline_fields = [
        (
            timing.runtime_shutdown_timeout,
            ApplicationConfigurationField::RuntimeShutdownTimeout,
        ),
        (
            timing.runtime_join_timeout,
            ApplicationConfigurationField::RuntimeJoinTimeout,
        ),
        (
            timing.hub_shutdown_timeout,
            ApplicationConfigurationField::HubShutdownTimeout,
        ),
    ];
    for (duration, field) in deadline_fields {
        if duration > MAXIMUM_SHUTDOWN_OR_JOIN_TIMEOUT {
            return Err(ApplicationError::InvalidConfiguration(field));
        }
    }
    Ok(())
}

fn duration_milliseconds(
    duration: Duration,
    field: ApplicationConfigurationField,
) -> Result<u64, ApplicationError> {
    u64::try_from(duration.as_millis())
        .ok()
        .filter(|milliseconds| *milliseconds != 0)
        .ok_or(ApplicationError::InvalidConfiguration(field))
}

fn validate_non_zero<T>(
    value: &T,
    field: ApplicationConfigurationField,
) -> Result<(), ApplicationError>
where
    T: Default + PartialEq,
{
    if value == &T::default() {
        Err(ApplicationError::InvalidConfiguration(field))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU64;

    use super::{application_preferences, runtime_memory_budget, stored_settings};
    use crate::{
        AcceleratorMemoryPolicy, ApplicationDevice, ApplicationDeviceSummary,
        ApplicationPreferences,
    };

    const GIBIBYTE: u64 = 1024 * 1024 * 1024;

    fn cuda_summary(ordinal: u32, total_memory_bytes: u64) -> ApplicationDeviceSummary {
        ApplicationDeviceSummary::discovered(
            ApplicationDevice::Cuda { ordinal },
            format!("CUDA {ordinal} — test"),
            Some(total_memory_bytes),
            Some(total_memory_bytes / 2),
            None,
        )
    }

    #[test]
    fn automatic_accelerator_budget_uses_the_least_selectable_physical_capacity() {
        let preferences = ApplicationPreferences::default();
        let devices = [
            ApplicationDeviceSummary::cpu(),
            cuda_summary(0, 12 * GIBIBYTE),
            cuda_summary(1, 6 * GIBIBYTE),
        ];

        let budget = runtime_memory_budget(&preferences, &devices);

        assert_eq!(budget.host_bytes, preferences.maximum_host_memory_bytes);
        assert_eq!(budget.device_bytes, 6 * GIBIBYTE);
    }

    #[test]
    fn missing_cuda_capacity_fails_closed_to_zero_budget() {
        let devices = [
            ApplicationDeviceSummary::cpu(),
            ApplicationDeviceSummary::discovered(
                ApplicationDevice::Cuda { ordinal: 0 },
                "CUDA 0 — unknown capacity".to_owned(),
                None,
                None,
                None,
            ),
        ];

        assert_eq!(
            runtime_memory_budget(&ApplicationPreferences::default(), &devices).device_bytes,
            0
        );
    }

    #[test]
    fn unavailable_cuda_catalogue_row_fails_closed_to_zero_budget() {
        let devices = [
            ApplicationDeviceSummary::cpu(),
            cuda_summary(0, 12 * GIBIBYTE),
            ApplicationDeviceSummary::unavailable(
                ApplicationDevice::Cuda { ordinal: 3 },
                crate::ApplicationDeviceUnavailableReason::DiscoveryFailed,
            ),
        ];

        assert_eq!(
            runtime_memory_budget(&ApplicationPreferences::default(), &devices).device_bytes,
            0
        );
    }

    #[test]
    fn explicit_accelerator_limit_is_capped_by_physical_capacity() {
        let preferences = ApplicationPreferences {
            accelerator_memory_policy: AcceleratorMemoryPolicy::Limit {
                bytes: NonZeroU64::new(8 * GIBIBYTE).unwrap_or(NonZeroU64::MIN),
            },
            ..ApplicationPreferences::default()
        };
        let devices = [
            ApplicationDeviceSummary::cpu(),
            cuda_summary(0, 12 * GIBIBYTE),
            cuda_summary(1, 6 * GIBIBYTE),
        ];

        assert_eq!(
            runtime_memory_budget(&preferences, &devices).device_bytes,
            6 * GIBIBYTE
        );

        let lower_preferences = ApplicationPreferences {
            accelerator_memory_policy: AcceleratorMemoryPolicy::Limit {
                bytes: NonZeroU64::new(4 * GIBIBYTE).unwrap_or(NonZeroU64::MIN),
            },
            ..preferences
        };
        assert_eq!(
            runtime_memory_budget(&lower_preferences, &devices).device_bytes,
            4 * GIBIBYTE
        );
    }

    #[test]
    fn cpu_only_budget_preserves_host_limit_and_uses_zero_device_bytes() {
        let preferences = ApplicationPreferences::default();
        let budget = runtime_memory_budget(&preferences, &[ApplicationDeviceSummary::cpu()]);

        assert_eq!(budget.host_bytes, preferences.maximum_host_memory_bytes);
        assert_eq!(budget.device_bytes, 0);
    }

    #[test]
    fn preference_storage_conversion_round_trips_device_and_policy() {
        let limit = NonZeroU64::new(3 * GIBIBYTE).unwrap_or(NonZeroU64::MIN);
        let preferences = ApplicationPreferences {
            selected_device: ApplicationDevice::Cuda { ordinal: 3 },
            accelerator_memory_policy: AcceleratorMemoryPolicy::Limit { bytes: limit },
            ..ApplicationPreferences::default()
        };

        assert_eq!(
            application_preferences(stored_settings(&preferences)),
            preferences
        );
    }
}
