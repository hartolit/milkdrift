use std::collections::{BTreeMap, BTreeSet};

use milkdrift_authority::SecretRef;
use milkdrift_capability::{BoundedJson, ExtensionKey, ProviderProfileRef};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;

/// Current non-secret model endpoint profile schema.
pub const MODEL_ENDPOINT_PROFILE_SCHEMA_VERSION_V1: u32 = 1;
const MAX_PROFILE_BYTES: usize = 262_144;
const PROFILE_JSON_LIMITS: milkdrift_contracts::JsonLimits = milkdrift_contracts::JsonLimits {
    maximum_depth: 24,
    maximum_string_bytes: 16_384,
    maximum_key_bytes: 192,
    maximum_container_items: 2_048,
};

/// Independently mapped provider wire family.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum ProviderProtocol {
    /// Configurable `/v1/chat/completions`-style protocol.
    OpenAiCompatible {
        /// Relative endpoint path, normally `v1/chat/completions`.
        path: String,
    },
    /// Native Anthropic Messages API.
    Anthropic {
        /// Required `anthropic-version` header value.
        version: String,
        /// Relative endpoint path, normally `v1/messages`.
        path: String,
    },
}

/// Non-secret authentication configuration.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum AuthMode {
    /// No authorization header, appropriate for explicit local endpoints.
    NoAuth,
    /// Bearer token resolved only at request entry.
    Bearer {
        /// Opaque secret reference.
        secret: SecretRef,
    },
    /// Native Anthropic `x-api-key` value resolved only at request entry.
    AnthropicApiKey {
        /// Opaque secret reference.
        secret: SecretRef,
    },
}

/// Redirect policy. Cross-origin redirects are never supported by this adapter.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RedirectPolicy {
    /// Reject every redirect.
    #[default]
    Deny,
    /// Permit same-origin redirects up to the bounded HTTP-client limit.
    SameOrigin,
}

/// TLS root policy. Insecure certificate acceptance is deliberately absent.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TlsPolicy {
    /// Maintained WebPKI roots and normal certificate verification.
    #[default]
    WebPkiRoots,
}

/// Proxy behavior is explicit.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyPolicy {
    /// Ignore ambient proxy variables.
    #[default]
    Disabled,
    /// Deliberately use the HTTP client's system proxy discovery.
    System,
}

/// Model features explicitly advertised by one profile.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelFeature {
    /// Streaming text fragments.
    Streaming,
    /// System role/instruction.
    SystemRole,
    /// Developer role distinct from system.
    DeveloperRole,
    /// Tool definitions and returned calls/results.
    Tools,
    /// Enforced JSON structured-output schema.
    StructuredOutput,
    /// Image content parts.
    Images,
    /// Generic file content parts.
    Files,
    /// Typed reasoning controls.
    Reasoning,
    /// Explicit provider-managed opaque sessions.
    ProviderSessions,
}

/// Defensive HTTP and stream limits.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EndpointLimits {
    /// Connect timeout.
    pub connect_timeout_ms: u64,
    /// Whole-request timeout.
    pub request_timeout_ms: u64,
    /// Idle/read timeout.
    pub idle_timeout_ms: u64,
    /// Maximum response-header count.
    pub max_headers: u16,
    /// Maximum aggregate response-header bytes.
    pub max_header_bytes: u32,
    /// Maximum encoded outbound request-body bytes.
    pub max_request_bytes: u64,
    /// Maximum complete response bytes.
    pub max_response_bytes: u64,
    /// Maximum SSE line bytes.
    pub max_stream_line_bytes: u32,
    /// Maximum SSE event bytes.
    pub max_stream_event_bytes: u32,
    /// Maximum streamed text fragment bytes reported durably.
    pub max_fragment_bytes: u32,
}

impl EndpointLimits {
    /// Validates nonzero, bounded, internally ordered limits.
    pub fn validate(self) -> Result<Self, ProfileError> {
        if self.connect_timeout_ms == 0
            || self.request_timeout_ms == 0
            || self.idle_timeout_ms == 0
            || self.connect_timeout_ms > self.request_timeout_ms
            || self.idle_timeout_ms > self.request_timeout_ms
            || self.max_headers == 0
            || self.max_headers > 1024
            || self.max_header_bytes == 0
            || self.max_header_bytes > 1_048_576
            || self.max_request_bytes == 0
            || self.max_request_bytes > 268_435_456
            || self.max_response_bytes == 0
            || self.max_response_bytes > 268_435_456
            || self.max_stream_line_bytes == 0
            || self.max_stream_event_bytes < self.max_stream_line_bytes
            || self.max_stream_event_bytes > 4_194_304
            || self.max_fragment_bytes == 0
            || self.max_fragment_bytes > 4_096
        {
            return Err(ProfileError::Invalid("invalid endpoint limits".to_owned()));
        }
        Ok(self)
    }
}

/// Versioned endpoint profile containing no secret values.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EndpointProfile {
    schema_version: u32,
    identity: ProviderProfileRef,
    revision: u64,
    protocol: ProviderProtocol,
    base_url: String,
    model: String,
    auth: AuthMode,
    limits: EndpointLimits,
    redirect: RedirectPolicy,
    tls: TlsPolicy,
    proxy: ProxyPolicy,
    features: BTreeSet<ModelFeature>,
    max_concurrent: u32,
    local_development: bool,
    allowed_hosts: BTreeSet<String>,
    trust_zones: BTreeSet<String>,
    provider_options: BTreeMap<ExtensionKey, BoundedJson>,
}

impl EndpointProfile {
    /// Constructs a completely validated non-secret profile.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        identity: ProviderProfileRef,
        revision: u64,
        protocol: ProviderProtocol,
        base_url: impl Into<String>,
        model: impl Into<String>,
        auth: AuthMode,
        limits: EndpointLimits,
        redirect: RedirectPolicy,
        tls: TlsPolicy,
        proxy: ProxyPolicy,
        features: BTreeSet<ModelFeature>,
        max_concurrent: u32,
        local_development: bool,
        allowed_hosts: BTreeSet<String>,
        trust_zones: BTreeSet<String>,
        provider_options: BTreeMap<ExtensionKey, BoundedJson>,
    ) -> Result<Self, ProfileError> {
        let value = Self {
            schema_version: MODEL_ENDPOINT_PROFILE_SCHEMA_VERSION_V1,
            identity,
            revision,
            protocol,
            base_url: base_url.into(),
            model: model.into(),
            auth,
            limits: limits.validate()?,
            redirect,
            tls,
            proxy,
            features,
            max_concurrent,
            local_development,
            allowed_hosts,
            trust_zones,
            provider_options,
        };
        value.validate()?;
        Ok(value)
    }

    /// Encodes the exact profile as bounded canonical JSON.
    pub fn to_canonical_json(&self) -> Result<Vec<u8>, ProfileError> {
        let bytes = milkdrift_contracts::canonical_json_bytes(self, PROFILE_JSON_LIMITS)
            .map_err(|error| ProfileError::Invalid(format!("profile JSON: {error:?}")))?;
        if bytes.len() > MAX_PROFILE_BYTES {
            return Err(ProfileError::Invalid(
                "endpoint profile exceeds its document byte bound".to_owned(),
            ));
        }
        Ok(bytes)
    }

    /// Bounds-checks, duplicate-checks, parses, and validates one exact v1 profile.
    pub fn from_json(bytes: &[u8]) -> Result<Self, ProfileError> {
        if bytes.len() > MAX_PROFILE_BYTES {
            return Err(ProfileError::Invalid(
                "endpoint profile exceeds its document byte bound".to_owned(),
            ));
        }
        milkdrift_contracts::preflight_json_structure(bytes, PROFILE_JSON_LIMITS)
            .map_err(|error| ProfileError::Invalid(format!("{error:?}")))?;
        let value = milkdrift_contracts::parse_json_without_duplicates(bytes)
            .map_err(|error| ProfileError::Invalid(error.to_string()))?;
        milkdrift_contracts::validate_json_value(&value, PROFILE_JSON_LIMITS)
            .map_err(|error| ProfileError::Invalid(format!("{error:?}")))?;
        serde_json::from_value(value).map_err(|error| ProfileError::Invalid(error.to_string()))
    }
    fn validate(&self) -> Result<(), ProfileError> {
        if self.revision == 0
            || self.model.is_empty()
            || self.model.len() > 512
            || self.max_concurrent == 0
            || self.max_concurrent > 4096
            || self.features.len() > 64
            || self.allowed_hosts.is_empty()
            || self.allowed_hosts.len() > 256
            || self.trust_zones.len() > 64
            || self.provider_options.len() > 64
        {
            return Err(ProfileError::Invalid(
                "endpoint profile exceeds count/identity bounds".to_owned(),
            ));
        }
        let url = Url::parse(&self.base_url)
            .map_err(|_| ProfileError::Invalid("invalid endpoint base URL".to_owned()))?;
        if url.username() != ""
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(ProfileError::Invalid(
                "endpoint URL cannot contain credentials, query, or fragment".to_owned(),
            ));
        }
        let host = url
            .host_str()
            .ok_or_else(|| ProfileError::Invalid("endpoint URL requires a host".to_owned()))?;
        if !self.allowed_hosts.contains(host) {
            return Err(ProfileError::Invalid(
                "endpoint host is not allowlisted".to_owned(),
            ));
        }
        let loopback = host.eq_ignore_ascii_case("localhost")
            || url.host().is_some_and(|host| match host {
                url::Host::Ipv4(ip) => ip.is_loopback(),
                url::Host::Ipv6(ip) => ip.is_loopback(),
                url::Host::Domain(domain) => domain.eq_ignore_ascii_case("localhost"),
            });
        match url.scheme(){"https"=>{},"http" if self.local_development && loopback=>{},_=>return Err(ProfileError::Invalid(
            "remote endpoints require HTTPS; HTTP is limited to explicit loopback development profiles".to_owned()))}
        if matches!(self.auth, AuthMode::NoAuth)
            && !self.local_development
            && url.scheme() != "https"
        {
            return Err(ProfileError::Invalid(
                "remote no-auth profiles still require HTTPS".to_owned(),
            ));
        }
        match &self.protocol {
            ProviderProtocol::OpenAiCompatible { path } if !safe_path(path) => {
                return Err(ProfileError::Invalid(
                    "invalid OpenAI-compatible path".to_owned(),
                ));
            }
            ProviderProtocol::Anthropic { version, path }
                if version.is_empty() || version.len() > 64 || !safe_path(path) =>
            {
                return Err(ProfileError::Invalid(
                    "invalid Anthropic version/path".to_owned(),
                ));
            }
            ProviderProtocol::Anthropic { .. }
                if self.features.contains(&ModelFeature::DeveloperRole)
                    || self.features.contains(&ModelFeature::StructuredOutput)
                    || self.features.contains(&ModelFeature::Files)
                    || self.features.contains(&ModelFeature::Reasoning)
                    || self.features.contains(&ModelFeature::ProviderSessions) =>
            {
                return Err(ProfileError::Invalid(
                    "Anthropic profile advertises an unsupported native mapping".to_owned(),
                ));
            }
            _ => {}
        }
        if self.features.contains(&ModelFeature::Files)
            || self.features.contains(&ModelFeature::ProviderSessions)
        {
            return Err(ProfileError::Invalid(
                "this adapter has no protocol mapping for generic files or provider-managed sessions"
                    .to_owned(),
            ));
        }
        Ok(())
    }
    /// Profile identity.
    #[must_use]
    pub const fn identity(&self) -> &ProviderProfileRef {
        &self.identity
    }
    /// Profile revision.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }
    /// Protocol family.
    #[must_use]
    pub const fn protocol(&self) -> &ProviderProtocol {
        &self.protocol
    }
    /// Secret-free base URL.
    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }
    /// Exact configured model identifier.
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }
    /// Authentication mode containing only a reference.
    #[must_use]
    pub const fn auth(&self) -> &AuthMode {
        &self.auth
    }
    /// HTTP/stream limits.
    #[must_use]
    pub const fn limits(&self) -> EndpointLimits {
        self.limits
    }
    /// Redirect policy.
    #[must_use]
    pub const fn redirect(&self) -> RedirectPolicy {
        self.redirect
    }
    /// TLS policy.
    #[must_use]
    pub const fn tls(&self) -> TlsPolicy {
        self.tls
    }
    /// Proxy policy.
    #[must_use]
    pub const fn proxy(&self) -> ProxyPolicy {
        self.proxy
    }
    /// Explicit advertised features.
    #[must_use]
    pub const fn features(&self) -> &BTreeSet<ModelFeature> {
        &self.features
    }
    /// Admission cap.
    #[must_use]
    pub const fn max_concurrent(&self) -> u32 {
        self.max_concurrent
    }
    /// True only for explicit local development.
    #[must_use]
    pub const fn local_development(&self) -> bool {
        self.local_development
    }
    /// Authority-controlled host allowlist.
    #[must_use]
    pub const fn allowed_hosts(&self) -> &BTreeSet<String> {
        &self.allowed_hosts
    }
    /// Profile trust zones.
    #[must_use]
    pub const fn trust_zones(&self) -> &BTreeSet<String> {
        &self.trust_zones
    }
    /// Bounded provider options.
    #[must_use]
    pub const fn provider_options(&self) -> &BTreeMap<ExtensionKey, BoundedJson> {
        &self.provider_options
    }
    pub(crate) fn endpoint_url(&self) -> Result<Url, ProfileError> {
        let mut url = Url::parse(&self.base_url)
            .map_err(|_| ProfileError::Invalid("invalid endpoint URL".to_owned()))?;
        let path = match &self.protocol {
            ProviderProtocol::OpenAiCompatible { path }
            | ProviderProtocol::Anthropic { path, .. } => path,
        };
        url.set_path(&format!(
            "{}/{}",
            url.path().trim_end_matches('/'),
            path.trim_start_matches('/')
        ));
        Ok(url)
    }
}

impl<'de> Deserialize<'de> for EndpointProfile {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            schema_version: u32,
            identity: ProviderProfileRef,
            revision: u64,
            protocol: ProviderProtocol,
            base_url: String,
            model: String,
            auth: AuthMode,
            limits: EndpointLimits,
            redirect: RedirectPolicy,
            tls: TlsPolicy,
            proxy: ProxyPolicy,
            features: BTreeSet<ModelFeature>,
            max_concurrent: u32,
            local_development: bool,
            allowed_hosts: BTreeSet<String>,
            trust_zones: BTreeSet<String>,
            provider_options: BTreeMap<ExtensionKey, BoundedJson>,
        }
        let w = Wire::deserialize(deserializer)?;
        if w.schema_version != MODEL_ENDPOINT_PROFILE_SCHEMA_VERSION_V1 {
            return Err(serde::de::Error::custom(
                "unsupported endpoint profile version",
            ));
        }
        Self::new(
            w.identity,
            w.revision,
            w.protocol,
            w.base_url,
            w.model,
            w.auth,
            w.limits,
            w.redirect,
            w.tls,
            w.proxy,
            w.features,
            w.max_concurrent,
            w.local_development,
            w.allowed_hosts,
            w.trust_zones,
            w.provider_options,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// Profile validation failure containing no secret values.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ProfileError {
    /// Invalid policy/configuration fact.
    #[error("invalid model endpoint profile: {0}")]
    Invalid(String),
}

fn safe_path(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && !value.contains("..")
        && !value.contains('?')
        && !value.contains('#')
}
