use std::{sync::Arc, time::Duration};

use milkdrift_authority::SensitiveSecret;
use milkdrift_capability::PeerId;
use milkdrift_peer_protocol::{ProtocolVersionRange, SessionId};
use url::{Host, Url};

use crate::PeerHttpError;

/// Explicitly named development-only plaintext policy.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum InsecureLoopbackMode {
    /// Plaintext is rejected.
    #[default]
    Disabled,
    /// Plaintext is allowed only for literal loopback or `localhost` endpoints.
    AllowInsecureLoopbackDevelopment,
}

/// Operator-configured outbound peer endpoint. Workflow/model input cannot replace it.
#[derive(Clone, Debug)]
pub struct PeerClientConfig {
    /// Operator-configured base URL.
    pub endpoint: Url,
    /// Authenticated local identity claimed only as a transport cross-check.
    pub local_peer: PeerId,
    /// Exact remote identity required in handshake responses.
    pub expected_remote_peer: PeerId,
    /// Current local boot/session identity.
    pub session: SessionId,
    /// Allowed versions for this relationship.
    pub versions: ProtocolVersionRange,
    /// Resolved bearer credential, redacted and never serialized.
    pub bearer_credential: Arc<SensitiveSecret>,
    /// Explicit development-only exception.
    pub insecure_loopback: InsecureLoopbackMode,
    /// Complete request deadline.
    pub request_timeout: Duration,
    /// Idle wait between resumable observation polls.
    pub observation_poll_interval: Duration,
}

impl PeerClientConfig {
    /// Refuses credentials in URLs, fragments, non-HTTP schemes, and plaintext non-loopback.
    pub fn validate(&self) -> Result<(), PeerHttpError> {
        validate_current_protocol_range(self.versions)?;
        if !self.endpoint.username().is_empty()
            || self.endpoint.password().is_some()
            || self.endpoint.fragment().is_some()
            || self.bearer_credential.is_empty()
            || self.request_timeout.is_zero()
            || self.observation_poll_interval.is_zero()
        {
            return Err(PeerHttpError::Configuration(
                "peer endpoint, credential, or deadline is invalid".to_owned(),
            ));
        }
        match self.endpoint.scheme() {
            "https" => Ok(()),
            "http"
                if self.insecure_loopback
                    == InsecureLoopbackMode::AllowInsecureLoopbackDevelopment
                    && endpoint_is_loopback(&self.endpoint) =>
            {
                Ok(())
            }
            "http" => Err(PeerHttpError::Configuration(
                "plaintext peer HTTP requires explicitly enabled loopback development mode"
                    .to_owned(),
            )),
            _ => Err(PeerHttpError::Configuration(
                "peer endpoint must use HTTPS".to_owned(),
            )),
        }
    }
}

fn validate_current_protocol_range(versions: ProtocolVersionRange) -> Result<(), PeerHttpError> {
    if versions != ProtocolVersionRange::default() {
        return Err(PeerHttpError::Configuration(
            "peer protocol configuration must select exactly v1.4".to_owned(),
        ));
    }
    Ok(())
}

fn endpoint_is_loopback(endpoint: &Url) -> bool {
    match endpoint.host() {
        Some(Host::Ipv4(address)) => address.is_loopback(),
        Some(Host::Ipv6(address)) => address.is_loopback(),
        Some(Host::Domain(name)) => name.eq_ignore_ascii_case("localhost"),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use milkdrift_peer_protocol::{ProtocolVersion, ProtocolVersionRange};

    use super::validate_current_protocol_range;

    #[test]
    fn configured_protocol_ranges_must_name_only_v1_4() -> Result<(), Box<dyn std::error::Error>> {
        assert!(validate_current_protocol_range(ProtocolVersionRange::default()).is_ok());
        for (minimum, maximum) in [
            (1_u16, 1_u16),
            (1, 2),
            (2, 2),
            (2, 3),
            (3, 3),
            (3, 4),
            (4, 5),
            (5, 5),
        ] {
            let range = ProtocolVersionRange::new(
                ProtocolVersion {
                    major: 1,
                    minor: minimum,
                },
                ProtocolVersion {
                    major: 1,
                    minor: maximum,
                },
            )?;
            assert!(validate_current_protocol_range(range).is_err());
        }
        Ok(())
    }
}
