//! Policy-enforcing model endpoint adapter.
//!
//! OpenAI-compatible and Anthropic mappings share bounded HTTP mechanics while
//! retaining independent request, streaming-event, tool, usage, and error mappings.

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
