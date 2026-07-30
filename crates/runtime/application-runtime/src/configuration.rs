//! Validated defaults and host-worker configuration for application orchestration.

use std::fmt::{self, Formatter};
use std::path::PathBuf;
use std::time::Duration;

const DEFAULT_REVISION: &str = "main";
const DEFAULT_HOST_MEMORY_BYTES: u64 = 16 * 1024 * 1024 * 1024;
const DEFAULT_DEVICE_MEMORY_BYTES: u64 = 0;
const DEFAULT_DRAIN_TIMEOUT_MILLISECONDS: u64 = 2_000;
const DEFAULT_MAXIMUM_REQUESTS: u32 = 1;
const DEFAULT_COMMAND_CAPACITY: usize = 32;
const DEFAULT_EVENT_CAPACITY: usize = 32;
const DEFAULT_HUB_CHANNEL_CAPACITY: usize = 4;
const DEFAULT_TOKEN_OUTPUT_CAPACITY: usize = 256;
const DEFAULT_TOKEN_OUTPUT_RECORD_CAPACITY: usize = 512;
const DEFAULT_TEXT_OUTPUT_BYTE_CAPACITY: usize = 64 * 1024;
const DEFAULT_TEXT_OUTPUT_RECORD_CAPACITY: usize = 512;
const DEFAULT_RUNTIME_POLL_MILLISECONDS: u64 = 10;
const DEFAULT_HUB_WORKER_POLL_MILLISECONDS: u64 = 100;
const DEFAULT_HUB_EVENT_SEND_TIMEOUT_MILLISECONDS: u64 = 100;
const DEFAULT_HUB_COMMAND_SHUTDOWN_TIMEOUT_MILLISECONDS: u64 = 250;
const DEFAULT_RUNTIME_SHUTDOWN_TIMEOUT_MILLISECONDS: u64 = 2_000;
const DEFAULT_RUNTIME_SHUTDOWN_EVENT_POLL_MILLISECONDS: u64 = 25;
const DEFAULT_RUNTIME_JOIN_TIMEOUT_MILLISECONDS: u64 = 2_000;
const DEFAULT_RUNTIME_JOIN_POLL_MILLISECONDS: u64 = 10;
const DEFAULT_HUB_SHUTDOWN_TIMEOUT_MILLISECONDS: u64 = 2_000;
const DEFAULT_HUB_SHUTDOWN_POLL_MILLISECONDS: u64 = 10;
const DEFAULT_GGUF_MAXIMUM_CONTEXT_TOKENS: u32 = 4_096;
const DEFAULT_GGUF_MAXIMUM_PREFILL_TOKENS: u32 = 512;
const DEFAULT_GGUF_MICRO_BATCH_TOKENS: u32 = 128;
const DEFAULT_GGUF_THREADS: u32 = 1;

/// Frontend-neutral Hugging Face cache and authentication overrides.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct ApplicationHubConfiguration {
    /// Optional cache root overriding environment-derived Hugging Face paths.
    pub cache_directory: Option<PathBuf>,
    /// Optional access token overriding environment-derived authentication.
    pub access_token: Option<String>,
    /// Number of download retries after the initial attempt.
    pub maximum_retries: usize,
}

impl fmt::Debug for ApplicationHubConfiguration {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ApplicationHubConfiguration")
            .field("cache_directory", &self.cache_directory)
            .field(
                "access_token",
                &self.access_token.as_ref().map(|_| "<redacted>"),
            )
            .field("maximum_retries", &self.maximum_retries)
            .finish()
    }
}

/// Application-owned low-level defaults for local llama.cpp/GGUF execution.
///
/// Frontends select a GGUF file but do not need to supply these backend tuning
/// values. E1 validates them and caps the effective context and batch sizes by
/// immutable model metadata during resolution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ApplicationGgufConfiguration {
    /// Maximum context tokens retained per sequence before the model metadata cap.
    pub maximum_context_tokens: u32,
    /// Maximum prompt tokens admitted in one prefill before the context cap.
    pub maximum_prefill_tokens: u32,
    /// Maximum physical llama.cpp micro-batch token count.
    pub micro_batch_tokens: u32,
    /// CPU thread count used for prompt and incremental decoding.
    pub threads: u32,
    /// Whether llama.cpp should memory-map GGUF weights.
    pub use_mmap: bool,
    /// Whether llama.cpp should lock model pages in host memory.
    pub use_mlock: bool,
}

impl Default for ApplicationGgufConfiguration {
    fn default() -> Self {
        Self {
            maximum_context_tokens: DEFAULT_GGUF_MAXIMUM_CONTEXT_TOKENS,
            maximum_prefill_tokens: DEFAULT_GGUF_MAXIMUM_PREFILL_TOKENS,
            micro_batch_tokens: DEFAULT_GGUF_MICRO_BATCH_TOKENS,
            threads: DEFAULT_GGUF_THREADS,
            use_mmap: false,
            use_mlock: false,
        }
    }
}

/// User-facing defaults used only when no persisted settings exist.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApplicationPreferences {
    /// Initial repository shown by a frontend.
    pub default_repository: String,
    /// Initial branch, tag, reference, or commit shown by a frontend.
    pub default_revision: String,
    /// Aggregate host-memory admission limit.
    pub maximum_host_memory_bytes: u64,
    /// Aggregate device-memory admission limit.
    pub maximum_device_memory_bytes: u64,
    /// Mandatory drain window before force-cancellation.
    pub drain_timeout_milliseconds: u64,
}

impl Default for ApplicationPreferences {
    fn default() -> Self {
        Self {
            default_repository: String::new(),
            default_revision: DEFAULT_REVISION.to_owned(),
            maximum_host_memory_bytes: DEFAULT_HOST_MEMORY_BYTES,
            maximum_device_memory_bytes: DEFAULT_DEVICE_MEMORY_BYTES,
            drain_timeout_milliseconds: DEFAULT_DRAIN_TIMEOUT_MILLISECONDS,
        }
    }
}

/// Bounded shutdown and worker polling intervals.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ApplicationTiming {
    /// Lifecycle polling interval for the inference worker.
    pub runtime_poll: Duration,
    /// Hub command polling interval.
    pub hub_worker_poll: Duration,
    /// Maximum wait for Hub event-channel capacity.
    pub hub_event_send_timeout: Duration,
    /// Maximum wait when submitting cooperative Hub shutdown.
    pub hub_command_shutdown_timeout: Duration,
    /// Maximum wait for the ticketed inference shutdown event.
    pub runtime_shutdown_timeout: Duration,
    /// Poll interval while waiting for the inference shutdown event.
    pub runtime_shutdown_event_poll: Duration,
    /// Maximum wait for inference-thread completion.
    pub runtime_join_timeout: Duration,
    /// Poll interval while waiting for inference-thread completion.
    pub runtime_join_poll: Duration,
    /// Maximum wait for Hub-thread completion.
    pub hub_shutdown_timeout: Duration,
    /// Poll interval while waiting for Hub-thread completion.
    pub hub_shutdown_poll: Duration,
}

impl Default for ApplicationTiming {
    fn default() -> Self {
        Self {
            runtime_poll: Duration::from_millis(DEFAULT_RUNTIME_POLL_MILLISECONDS),
            hub_worker_poll: Duration::from_millis(DEFAULT_HUB_WORKER_POLL_MILLISECONDS),
            hub_event_send_timeout: Duration::from_millis(
                DEFAULT_HUB_EVENT_SEND_TIMEOUT_MILLISECONDS,
            ),
            hub_command_shutdown_timeout: Duration::from_millis(
                DEFAULT_HUB_COMMAND_SHUTDOWN_TIMEOUT_MILLISECONDS,
            ),
            runtime_shutdown_timeout: Duration::from_millis(
                DEFAULT_RUNTIME_SHUTDOWN_TIMEOUT_MILLISECONDS,
            ),
            runtime_shutdown_event_poll: Duration::from_millis(
                DEFAULT_RUNTIME_SHUTDOWN_EVENT_POLL_MILLISECONDS,
            ),
            runtime_join_timeout: Duration::from_millis(DEFAULT_RUNTIME_JOIN_TIMEOUT_MILLISECONDS),
            runtime_join_poll: Duration::from_millis(DEFAULT_RUNTIME_JOIN_POLL_MILLISECONDS),
            hub_shutdown_timeout: Duration::from_millis(DEFAULT_HUB_SHUTDOWN_TIMEOUT_MILLISECONDS),
            hub_shutdown_poll: Duration::from_millis(DEFAULT_HUB_SHUTDOWN_POLL_MILLISECONDS),
        }
    }
}

/// Complete frontend-neutral application-runtime configuration.
#[derive(Clone, Debug)]
pub struct ApplicationRuntimeConfiguration {
    /// redb database path selected by the execution environment.
    pub database_path: PathBuf,
    /// Hugging Face cache, authentication, and retry overrides.
    pub hub: ApplicationHubConfiguration,
    /// Settings used only when no persisted record exists.
    pub defaults: ApplicationPreferences,
    /// Application-owned local llama.cpp/GGUF execution defaults.
    pub gguf: ApplicationGgufConfiguration,
    /// Maximum concurrently active inference requests.
    pub maximum_requests: u32,
    /// Maximum queued inference commands.
    pub command_capacity: usize,
    /// Maximum queued inference events.
    pub event_capacity: usize,
    /// Maximum queued Hub commands and results.
    pub hub_channel_capacity: usize,
    /// Maximum unpublished E0 token identifiers retained between application pulls.
    pub token_output_capacity: usize,
    /// Maximum unpublished E0 token/state records retained between application pulls.
    pub token_output_record_capacity: usize,
    /// Maximum unpublished decoded UTF-8 bytes retained for frontend pulls.
    pub text_output_byte_capacity: usize,
    /// Maximum unpublished decoded text/state records retained for frontend pulls.
    pub text_output_record_capacity: usize,
    /// Worker polling and shutdown intervals.
    pub timing: ApplicationTiming,
}

impl ApplicationRuntimeConfiguration {
    /// Creates a desktop-oriented single-model configuration with bounded defaults.
    #[must_use]
    pub fn desktop(database_path: impl Into<PathBuf>) -> Self {
        Self {
            database_path: database_path.into(),
            hub: ApplicationHubConfiguration::default(),
            defaults: ApplicationPreferences::default(),
            gguf: ApplicationGgufConfiguration::default(),
            maximum_requests: DEFAULT_MAXIMUM_REQUESTS,
            command_capacity: DEFAULT_COMMAND_CAPACITY,
            event_capacity: DEFAULT_EVENT_CAPACITY,
            hub_channel_capacity: DEFAULT_HUB_CHANNEL_CAPACITY,
            token_output_capacity: DEFAULT_TOKEN_OUTPUT_CAPACITY,
            token_output_record_capacity: DEFAULT_TOKEN_OUTPUT_RECORD_CAPACITY,
            text_output_byte_capacity: DEFAULT_TEXT_OUTPUT_BYTE_CAPACITY,
            text_output_record_capacity: DEFAULT_TEXT_OUTPUT_RECORD_CAPACITY,
            timing: ApplicationTiming::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ApplicationRuntimeConfiguration;
    use crate::support::validate_configuration;
    use crate::{ApplicationConfigurationField, ApplicationError};

    #[test]
    fn application_configuration_debug_redacts_hub_access_token() {
        let mut configuration = ApplicationRuntimeConfiguration::desktop("application.redb");
        configuration.hub.access_token = Some("secret-token".to_owned());

        let debug = format!("{configuration:?}");
        assert!(!debug.contains("secret-token"));
        assert!(debug.contains("<redacted>"));
    }

    #[test]
    fn gguf_execution_defaults_are_bounded_and_validated_by_e1() {
        let configuration = ApplicationRuntimeConfiguration::desktop("application.redb");
        assert_eq!(validate_configuration(&configuration), Ok(()));
        assert!(configuration.gguf.maximum_context_tokens > 0);
        assert!(configuration.gguf.maximum_prefill_tokens > 0);
        assert!(configuration.gguf.micro_batch_tokens > 0);
        assert!(configuration.gguf.threads > 0);
        assert!(!configuration.gguf.use_mlock);
    }

    #[test]
    fn invalid_gguf_execution_defaults_are_rejected_before_worker_start() {
        let mut configuration = ApplicationRuntimeConfiguration::desktop("application.redb");
        configuration.gguf.micro_batch_tokens =
            configuration.gguf.maximum_prefill_tokens.saturating_add(1);
        assert_eq!(
            validate_configuration(&configuration),
            Err(ApplicationError::InvalidConfiguration(
                ApplicationConfigurationField::GgufMicroBatchTokens
            ))
        );

        configuration.gguf.micro_batch_tokens = 1;
        configuration.gguf.maximum_context_tokens = 0;
        assert_eq!(
            validate_configuration(&configuration),
            Err(ApplicationError::InvalidConfiguration(
                ApplicationConfigurationField::GgufMaximumContextTokens
            ))
        );
    }
}
