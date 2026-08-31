use std::{
    io::Read as _,
    sync::{Arc, Mutex},
};

use milkdrift_peer_protocol::{
    ArtifactChunk, ArtifactMetadataOffer, ArtifactTransferDecision, CatalogSnapshot,
    HandshakeRequest, HandshakeResponse, InvocationAcceptance, InvocationLookup,
    ObservationHistory, ObservationPage, PeerCancellationAcknowledgement, PeerCancellationRequest,
    PeerExecutionId, PeerInvocationRequest, PeerRequestId, ProtocolEnvelope, TransferId,
    decode_envelope, encode_envelope,
};
use reqwest::{
    blocking::{Client, Response},
    header::{AUTHORIZATION, CONTENT_TYPE, HeaderValue},
};
use serde::{Serialize, de::DeserializeOwned};
use url::Url;

use crate::{PeerClientConfig, PeerCredentialSource, PeerHttpError, StaticPeerCredential};

/// Bounded blocking client used behind the synchronous capability-adapter boundary.
pub struct PeerHttpClient {
    config: PeerClientConfig,
    client: Client,
    credential: Arc<dyn PeerCredentialSource>,
    negotiated: Mutex<Option<HandshakeResponse>>,
}

impl std::fmt::Debug for PeerHttpClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PeerHttpClient")
            .field("endpoint", &self.config.endpoint)
            .field("local_peer", &self.config.local_peer)
            .field("expected_remote_peer", &self.config.expected_remote_peer)
            .field("authorization", &"[redacted]")
            .finish_non_exhaustive()
    }
}

impl PeerHttpClient {
    /// Builds a Rustls-backed HTTPS client for one operator-configured endpoint.
    pub fn new(config: PeerClientConfig) -> Result<Arc<Self>, PeerHttpError> {
        let credential = Arc::new(StaticPeerCredential::new(config.bearer_credential.clone()));
        Self::new_with_credential_source(config, credential)
    }

    /// Builds a client that resolves its credential at every request for safe rotation.
    pub fn new_with_credential_source(
        config: PeerClientConfig,
        credential: Arc<dyn PeerCredentialSource>,
    ) -> Result<Arc<Self>, PeerHttpError> {
        config.validate()?;
        let _ = credential.resolve()?;
        let client = Client::builder()
            .timeout(config.request_timeout)
            .connect_timeout(
                config
                    .request_timeout
                    .min(std::time::Duration::from_secs(10)),
            )
            .https_only(config.endpoint.scheme() == "https")
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("milkdrift-peer-http/1")
            .build()
            .map_err(|error| PeerHttpError::Configuration(error.to_string()))?;
        Ok(Arc::new(Self {
            config,
            client,
            credential,
            negotiated: Mutex::new(None),
        }))
    }

    /// Local configured peer identity.
    #[must_use]
    pub const fn local_peer(&self) -> &milkdrift_authority::PeerId {
        &self.config.local_peer
    }

    /// Exact remote configured peer identity.
    #[must_use]
    pub const fn remote_peer(&self) -> &milkdrift_authority::PeerId {
        &self.config.expected_remote_peer
    }

    pub(crate) fn endpoint_destination(&self) -> String {
        let host = match self.config.endpoint.host() {
            Some(url::Host::Ipv6(address)) => format!("[{address}]"),
            Some(url::Host::Ipv4(address)) => address.to_string(),
            Some(url::Host::Domain(name)) => name.to_owned(),
            None => "invalid-peer-endpoint".to_owned(),
        };
        self.config
            .endpoint
            .port_or_known_default()
            .map_or(host.clone(), |port| format!("{host}:{port}"))
    }

    /// Performs identity cross-checking and version/hard-limit negotiation.
    pub fn handshake(&self) -> Result<HandshakeResponse, PeerHttpError> {
        let request = HandshakeRequest {
            claimed_peer: self.config.local_peer.clone(),
            session: self.config.session.clone(),
            versions: self.config.versions,
            features: milkdrift_peer_protocol::FeatureSet {
                resumable_observations: true,
                resumable_artifacts: true,
                incremental_catalog: false,
                archived_execution_replay: true,
            },
            limits: milkdrift_peer_protocol::HardLimits::default(),
        };
        let response: HandshakeResponse = self.post(&["peer", "v1", "handshake"], &request)?;
        if response.peer != self.config.expected_remote_peer
            || self
                .config
                .versions
                .negotiate(milkdrift_peer_protocol::ProtocolVersionRange {
                    minimum: response.selected_version,
                    maximum: response.selected_version,
                })
                .is_err()
        {
            return Err(PeerHttpError::Unauthorized(
                "handshake remote identity or selected version mismatch".to_owned(),
            ));
        }
        *self
            .negotiated
            .lock()
            .map_err(|_| PeerHttpError::Unavailable("handshake cache unavailable".to_owned()))? =
            Some(response.clone());
        Ok(response)
    }

    /// Reads one complete authenticated expiring catalog snapshot.
    pub fn catalog(&self) -> Result<CatalogSnapshot, PeerHttpError> {
        self.ensure_handshake()?;
        let snapshot: CatalogSnapshot = self.get(&["peer", "v1", "catalog"], &[])?;
        snapshot
            .validate()
            .map_err(|error| PeerHttpError::Protocol(error.to_string()))?;
        Ok(snapshot)
    }

    /// Submits under exact idempotency. Transport ambiguity retries the same request, then
    /// queries the key before reporting uncertainty to the adapter.
    pub fn submit(
        &self,
        request: &PeerInvocationRequest,
    ) -> Result<InvocationAcceptance, PeerHttpError> {
        self.ensure_handshake()?;
        let mut last_error = None;
        for _attempt in 0..3 {
            match self.post(&["peer", "v1", "invocations"], request) {
                Ok(response) => return Ok(response),
                Err(error @ PeerHttpError::Transport(_))
                | Err(error @ PeerHttpError::Unavailable(_)) => last_error = Some(error),
                Err(error) => return Err(error),
            }
        }
        match self.lookup(&request.request_id) {
            Ok(InvocationLookup::Known {
                execution,
                request_digest,
                accepted_at_unix_ms,
                history,
                ..
            }) if request_digest == request.request_digest => match history {
                ObservationHistory::Hot => Ok(InvocationAcceptance::Accepted {
                    request_id: request.request_id.clone(),
                    execution,
                    request_digest,
                    accepted_at_unix_ms,
                    lease_expires_at_unix_ms: 0,
                    replayed: true,
                }),
                ObservationHistory::Archived { summary } => Ok(InvocationAcceptance::Archived {
                    request_id: request.request_id.clone(),
                    execution,
                    request_digest,
                    accepted_at_unix_ms,
                    summary,
                }),
            },
            Ok(InvocationLookup::NotAccepted) => Err(last_error.unwrap_or_else(|| {
                PeerHttpError::Transport("submission was not accepted".to_owned())
            })),
            Ok(InvocationLookup::Known { .. }) => Err(PeerHttpError::Protocol(
                "idempotency lookup returned conflicting request digest".to_owned(),
            )),
            Ok(InvocationLookup::Unknown { reason }) => Err(PeerHttpError::Transport(reason)),
            Err(error) => Err(last_error.unwrap_or(error)),
        }
    }

    /// Queries durable acceptance by idempotency key.
    pub fn lookup(&self, request: &PeerRequestId) -> Result<InvocationLookup, PeerHttpError> {
        self.get(&["peer", "v1", "requests", request.as_str()], &[])
    }

    /// Reads one contiguous resumable observation page.
    pub fn observations(
        &self,
        execution: &PeerExecutionId,
        after: u64,
        limit: usize,
    ) -> Result<ObservationPage, PeerHttpError> {
        let query = [
            ("after", after.to_string()),
            ("limit", limit.min(256).to_string()),
        ];
        let page: ObservationPage = self.get(
            &[
                "peer",
                "v1",
                "executions",
                execution.as_str(),
                "observations",
            ],
            &query,
        )?;
        page.validate(limit.min(256))
            .map_err(|error| PeerHttpError::Protocol(error.to_string()))?;
        Ok(page)
    }

    /// Requests cancellation independently from connection closure.
    pub fn cancel(
        &self,
        request: &PeerCancellationRequest,
    ) -> Result<PeerCancellationAcknowledgement, PeerHttpError> {
        let response: PeerCancellationAcknowledgement = self.post(
            &[
                "peer",
                "v1",
                "executions",
                request.execution.as_str(),
                "cancel",
            ],
            request,
        )?;
        response
            .validate()
            .map_err(|error| PeerHttpError::Protocol(error.to_string()))?;
        Ok(response)
    }

    /// Negotiates metadata, authority, quota, deduplication, and resume offset before bytes.
    pub fn negotiate_artifact(
        &self,
        offer: &ArtifactMetadataOffer,
    ) -> Result<ArtifactTransferDecision, PeerHttpError> {
        self.post(&["peer", "v1", "artifacts", "negotiate"], offer)
    }

    /// Uploads one raw bounded sequential chunk outside JSON control envelopes.
    pub fn write_artifact_chunk(
        &self,
        chunk: &ArtifactChunk,
    ) -> Result<ArtifactTransferDecision, PeerHttpError> {
        let mut url = endpoint(
            &self.config.endpoint,
            &[
                "peer",
                "v1",
                "artifacts",
                chunk.transfer.as_str(),
                "content",
            ],
        )?;
        url.query_pairs_mut()
            .append_pair("offset", &chunk.offset.to_string())
            .append_pair(
                "final_chunk",
                if chunk.final_chunk { "true" } else { "false" },
            );
        let response = self
            .client
            .post(url)
            .header(AUTHORIZATION, self.authorization()?)
            .header(CONTENT_TYPE, "application/octet-stream")
            .body(chunk.bytes.clone())
            .send()
            .map_err(transport)?;
        decode_response(response)
    }

    /// Downloads one raw bounded verified range after metadata negotiation.
    pub fn read_artifact_chunk(
        &self,
        transfer: &TransferId,
        offset: u64,
        maximum_bytes: u32,
    ) -> Result<ArtifactChunk, PeerHttpError> {
        let mut url = endpoint(
            &self.config.endpoint,
            &["peer", "v1", "artifacts", transfer.as_str(), "content"],
        )?;
        url.query_pairs_mut()
            .append_pair("offset", &offset.to_string())
            .append_pair("maximum_bytes", &maximum_bytes.to_string());
        let mut response = self
            .client
            .get(url)
            .header(AUTHORIZATION, self.authorization()?)
            .send()
            .map_err(transport)?;
        if !response.status().is_success() {
            return Err(status_error(response.status()));
        }
        let returned_offset = response
            .headers()
            .get("x-milkdrift-artifact-offset")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            .ok_or_else(|| {
                PeerHttpError::Protocol("artifact offset header is absent".to_owned())
            })?;
        let final_chunk = response
            .headers()
            .get("x-milkdrift-artifact-final")
            .and_then(|value| value.to_str().ok())
            .map(|value| value == "true")
            .ok_or_else(|| PeerHttpError::Protocol("artifact final header is absent".to_owned()))?;
        let limit = maximum_bytes.min(milkdrift_peer_protocol::MAX_ARTIFACT_CHUNK_BYTES);
        let mut bytes = Vec::new();
        response
            .by_ref()
            .take(u64::from(limit).saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|error| PeerHttpError::Transport(error.to_string()))?;
        if bytes.len() > usize::try_from(limit).unwrap_or(usize::MAX) {
            return Err(PeerHttpError::Protocol(
                "artifact range exceeds requested bound".to_owned(),
            ));
        }
        Ok(ArtifactChunk {
            transfer: transfer.clone(),
            offset: returned_offset,
            bytes,
            final_chunk,
        })
    }

    /// Aborts one incomplete transfer and its temporary bytes.
    pub fn abort_artifact(&self, transfer: &TransferId) -> Result<(), PeerHttpError> {
        let url = endpoint(
            &self.config.endpoint,
            &["peer", "v1", "artifacts", transfer.as_str(), "abort"],
        )?;
        let response = self
            .client
            .post(url)
            .header(AUTHORIZATION, self.authorization()?)
            .send()
            .map_err(transport)?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(status_error(response.status()))
        }
    }

    /// Configured idle interval for a slow or empty observation page.
    #[must_use]
    pub const fn observation_poll_interval(&self) -> std::time::Duration {
        self.config.observation_poll_interval
    }

    fn ensure_handshake(&self) -> Result<(), PeerHttpError> {
        let present = self
            .negotiated
            .lock()
            .map_err(|_| PeerHttpError::Unavailable("handshake cache unavailable".to_owned()))?
            .is_some();
        if !present {
            self.handshake()?;
        }
        Ok(())
    }

    fn authorization(&self) -> Result<HeaderValue, PeerHttpError> {
        let secret = self.credential.resolve()?;
        let mut value = secret
            .expose(|bytes| {
                let mut header = b"Bearer ".to_vec();
                header.extend_from_slice(bytes);
                HeaderValue::from_bytes(&header)
            })
            .map_err(|_| {
                PeerHttpError::Configuration(
                    "peer bearer credential cannot be represented as an HTTP header".to_owned(),
                )
            })?;
        value.set_sensitive(true);
        Ok(value)
    }

    fn post<T: Serialize, R: DeserializeOwned>(
        &self,
        path: &[&str],
        message: &T,
    ) -> Result<R, PeerHttpError> {
        let url = endpoint(&self.config.endpoint, path)?;
        let body = encode_envelope(&ProtocolEnvelope::v1(message))
            .map_err(|error| PeerHttpError::Protocol(error.to_string()))?;
        let response = self
            .client
            .post(url)
            .header(AUTHORIZATION, self.authorization()?)
            .header(CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .map_err(transport)?;
        decode_response(response)
    }

    fn get<R: DeserializeOwned>(
        &self,
        path: &[&str],
        query: &[(&str, String)],
    ) -> Result<R, PeerHttpError> {
        let mut url = endpoint(&self.config.endpoint, path)?;
        if !query.is_empty() {
            let mut pairs = url.query_pairs_mut();
            for (key, value) in query {
                pairs.append_pair(key, value);
            }
        }
        let response = self
            .client
            .get(url)
            .header(AUTHORIZATION, self.authorization()?)
            .send()
            .map_err(transport)?;
        decode_response(response)
    }
}

fn endpoint(base: &Url, path: &[&str]) -> Result<Url, PeerHttpError> {
    let mut value = base.clone();
    value.set_query(None);
    value.set_fragment(None);
    {
        let mut segments = value.path_segments_mut().map_err(|_| {
            PeerHttpError::Configuration("peer endpoint cannot be a base URL".to_owned())
        })?;
        segments.clear();
        for segment in path {
            segments.push(segment);
        }
    }
    Ok(value)
}

fn decode_response<T: DeserializeOwned>(mut response: Response) -> Result<T, PeerHttpError> {
    let status = response.status();
    let mut bytes = Vec::new();
    response
        .by_ref()
        .take(
            u64::try_from(milkdrift_peer_protocol::MAX_PEER_DOCUMENT_BYTES)
                .unwrap_or(u64::MAX)
                .saturating_add(1),
        )
        .read_to_end(&mut bytes)
        .map_err(|error| PeerHttpError::Transport(error.to_string()))?;
    if bytes.len() > milkdrift_peer_protocol::MAX_PEER_DOCUMENT_BYTES {
        return Err(PeerHttpError::Protocol(
            "peer response exceeds the negotiated document bound".to_owned(),
        ));
    }
    if !status.is_success() {
        return Err(status_error(status));
    }
    let envelope: ProtocolEnvelope<T> =
        decode_envelope(&bytes, milkdrift_peer_protocol::DecodeLimits::default())
            .map_err(|error| PeerHttpError::Protocol(error.to_string()))?;
    Ok(envelope.message)
}

fn status_error(status: reqwest::StatusCode) -> PeerHttpError {
    match status.as_u16() {
        401 => PeerHttpError::Unauthenticated,
        403 => PeerHttpError::Unauthorized("remote peer denied the operation".to_owned()),
        404 => PeerHttpError::NotFound("remote peer record was not found".to_owned()),
        429 => PeerHttpError::Overloaded("remote peer quota reached".to_owned()),
        value if value >= 500 => {
            PeerHttpError::Unavailable(format!("remote peer returned HTTP {value}"))
        }
        value => PeerHttpError::Protocol(format!("remote peer returned HTTP {value}")),
    }
}

fn transport(error: reqwest::Error) -> PeerHttpError {
    PeerHttpError::Transport(if error.is_timeout() {
        "peer request deadline elapsed".to_owned()
    } else if error.is_connect() {
        "peer connection failed".to_owned()
    } else {
        "peer response failed".to_owned()
    })
}
