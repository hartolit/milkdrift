//! Internal configuration validation, type conversion, and worker construction.

use std::num::{NonZeroU32, NonZeroU64, NonZeroUsize};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use candle_backend::CandleScalarType;
use domain_contracts::MemoryBudget;
use hf_hub_adapter::{ArtifactScalarType, HubClientConfiguration};
use host_runtime::ThreadPanicked;
use inference_runtime::{HostedRuntimeConfiguration, RuntimeLimits};
use redb_storage::{ApplicationSettings, StoredScalarType};

use crate::local::LocalInference;
use crate::{
    ApplicationConfigurationField, ApplicationError, ApplicationFailure, ApplicationFailureKind,
    ApplicationHubConfiguration, ApplicationPreferences, ApplicationRuntimeConfiguration,
    ApplicationScalarType, ApplicationTiming,
};

pub fn hub_configuration(configuration: &ApplicationHubConfiguration) -> HubClientConfiguration {
    HubClientConfiguration {
        cache_directory: configuration.cache_directory.clone(),
        access_token: configuration.access_token.clone(),
        maximum_retries: configuration.maximum_retries,
    }
}

pub fn create_runtime(
    preferences: &ApplicationPreferences,
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
    let limits = RuntimeLimits::new(
        NonZeroU32::MIN,
        maximum_requests,
        MemoryBudget {
            host_bytes: preferences.maximum_host_memory_bytes,
            device_bytes: preferences.maximum_device_memory_bytes,
        },
    );
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
        maximum_device_memory_bytes: settings.maximum_device_memory_bytes,
        drain_timeout_milliseconds: settings.drain_timeout_milliseconds,
    }
}

pub fn stored_settings(preferences: &ApplicationPreferences) -> ApplicationSettings {
    ApplicationSettings {
        default_repository: preferences.default_repository.clone(),
        default_revision: preferences.default_revision.clone(),
        maximum_host_memory_bytes: preferences.maximum_host_memory_bytes,
        maximum_device_memory_bytes: preferences.maximum_device_memory_bytes,
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
