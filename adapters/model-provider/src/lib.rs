//! Send a model task to an explicitly configured endpoint and publish its response artifacts.
//!
//! Read an [`EndpointProfile`], then construct [`ModelEndpointAdapter`] and register the matching
//! [`descriptor_for_profile`] result with the capability host. Tasks use `milkdrift-model` request
//! documents and runtime's frozen context manifest. [`ModelFeature`] determines which task and
//! injected-context features this profile advertises; negotiation precedes HTTP entry.
//! Health reports local lifecycle and load without probing endpoint availability.
//!
//! OpenAI-compatible chat and native Anthropic mappings share transport bounds while retaining
//! their own request and completion semantics. Returned tool calls remain data. Cancellation
//! can interrupt local observation but cannot prove that remote computation stopped.

mod adapter;
mod anthropic;
mod http;
mod openai_compatible;
mod profile;
mod stream;

#[cfg(feature = "operational-evidence")]
mod operational_evidence;

pub use adapter::{ModelEndpointAdapter, descriptor_for_profile};
pub use profile::{
    AuthMode, EndpointLimits, EndpointProfile, ModelFeature, ProfileError, ProviderProtocol,
    ProxyPolicy, RedirectPolicy, TlsPolicy,
};

#[cfg(feature = "operational-evidence")]
pub use operational_evidence::{StreamFixtureEvidence, exercise_stream_fixtures};
