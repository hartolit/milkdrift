//! Operate a Milkdrift daemon through authenticated requests and incremental observations.
//!
//! Construct a [`ControlClient`], call [`ControlClient::negotiate`], then submit commands or
//! inspect authorized read models. Safe JSON queries have bounded retries; command submission
//! sends once so a caller can deliberately recover a lost reply with the exact saved request.
//! Page methods fetch one page. [`ControlClient::subscribe`] reconnects observations, while its
//! consumer owns the overall deadline, reconnection limit, and handling of resynchronization.
//!
//! Construction performs no network request:
//!
//! ```
//! use milkdrift_control_client::{BearerCredential, ClientConfig, ControlClient};
//!
//! let config = ClientConfig::new("http://127.0.0.1:9734/".parse()?);
//! let credential = BearerCredential::new("example-only-credential")?;
//! let client = ControlClient::new(config, credential)?;
//! # let _ = client;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! Obtain real credentials from a private source. The server supplies actor/grant identity;
//! the client contains no CLI confirmation or presentation policy.

use std::{fmt, pin::Pin, time::Duration};

use futures_util::{Stream, StreamExt};
use milkdrift_control_protocol::{
    ArtifactMetadataRead, AuthorityRead, CapabilityRead, CommandAccepted, CommandRequest, Cursor,
    ErrorEnvelope, HealthRead, LayoutDocument, ObservationEnvelope, Page, PageRequest, PeerRead,
    ProposalRead, ProtocolError, ProtocolVersion, ResponseEnvelope, RevisionDiffRead, RevisionRead,
    RevisionSummary, RunRead, TimelineEntry, VersionRequest, VersionResponse, decode_json,
    encode_json,
};
use reqwest::{Method, StatusCode, header};
use serde::de::DeserializeOwned;
use thiserror::Error;
use url::Url;

mod invocation;
mod saved_run;
pub use saved_run::SavedRunRequest;

/// Default maximum artifact range materialized by one client call.
pub const DEFAULT_MAX_ARTIFACT_RANGE_BYTES: usize = 8 * 1024 * 1024;
/// Default bounded reconnect delay for resumable streams.
pub const DEFAULT_RECONNECT_DELAY: Duration = Duration::from_millis(500);

/// Bearer credential whose formatting is always redacted.
#[derive(Clone)]
pub struct BearerCredential(String);

impl BearerCredential {
    /// Accepts a nonempty credential without exposing it through diagnostics.
    ///
    /// # Errors
    /// Refuses empty, oversized, or newline-bearing credentials without including their value.
    pub fn new(value: impl Into<String>) -> Result<Self, ClientError> {
        let value = value.into();
        if value.is_empty() || value.len() > 4_096 || value.contains(['\r', '\n']) {
            return Err(ClientError::Configuration(
                "bearer credential must contain 1..=4096 non-newline bytes".to_owned(),
            ));
        }
        Ok(Self(value))
    }

    fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for BearerCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BearerCredential([redacted])")
    }
}

/// Immutable client construction and retry policy.
#[derive(Clone, Debug)]
pub struct ClientConfig {
    /// Daemon base URL, normally loopback HTTP.
    pub endpoint: Url,
    /// Timeout per non-streaming request; safe-query retries can extend the complete call.
    pub request_timeout: Duration,
    /// Safe-query retry count after the initial attempt.
    pub safe_query_retries: u8,
    /// Delay between safe-query retries and stream reconnects.
    pub retry_delay: Duration,
    /// Maximum artifact bytes returned by one range call.
    pub max_artifact_range_bytes: usize,
}

impl ClientConfig {
    /// Builds a conservative local client policy.
    #[must_use]
    pub fn new(endpoint: Url) -> Self {
        Self {
            endpoint,
            request_timeout: Duration::from_secs(30),
            safe_query_retries: 2,
            retry_delay: DEFAULT_RECONNECT_DELAY,
            max_artifact_range_bytes: DEFAULT_MAX_ARTIFACT_RANGE_BYTES,
        }
    }

    fn validate(&self) -> Result<(), ClientError> {
        let loopback = match self.endpoint.host() {
            Some(url::Host::Ipv4(address)) => address.is_loopback(),
            Some(url::Host::Ipv6(address)) => address.is_loopback(),
            Some(url::Host::Domain(name)) => name.eq_ignore_ascii_case("localhost"),
            None => false,
        };
        if self.endpoint.scheme() == "http" && !loopback {
            return Err(ClientError::Configuration(
                "non-loopback control endpoints require HTTPS".to_owned(),
            ));
        }
        if self.endpoint.scheme() != "http" && self.endpoint.scheme() != "https" {
            return Err(ClientError::Configuration(
                "control endpoint must use http or https".to_owned(),
            ));
        }
        if self.endpoint.cannot_be_a_base()
            || !self.endpoint.username().is_empty()
            || self.endpoint.password().is_some()
            || self.endpoint.query().is_some()
            || self.endpoint.fragment().is_some()
            || self.request_timeout.is_zero()
            || self.max_artifact_range_bytes == 0
            || self.max_artifact_range_bytes > 256 * 1024 * 1024
        {
            return Err(ClientError::Configuration(
                "invalid endpoint, timeout, or artifact range bound".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Client-side transport and public API failures.
#[derive(Debug, Error)]
pub enum ClientError {
    /// Client configuration is invalid.
    #[error("invalid client configuration: {0}")]
    Configuration(String),
    /// The daemon returned a stable public error.
    #[error("daemon returned a public API error")]
    Api(ErrorEnvelope),
    /// HTTP failed; credentials and headers are not included.
    #[error("control transport failed: {0}")]
    Transport(String),
    /// A protocol JSON document was invalid.
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
    /// The request timed out.
    #[error("control request timed out")]
    Timeout,
    /// A stream ended with a non-resumable protocol failure.
    #[error("control stream failed: {0}")]
    Stream(String),
}

impl ClientError {
    /// Whether this failure is classified as retryable.
    ///
    /// A timeout can follow durable command acceptance. Retrying a mutation still requires
    /// its exact original request and authority; this flag does not prove non-execution.
    #[must_use]
    pub fn retryable(&self) -> bool {
        match self {
            Self::Api(error) => error.retryable,
            Self::Transport(_) | Self::Timeout => true,
            Self::Configuration(_) | Self::Protocol(_) | Self::Stream(_) => false,
        }
    }
}

/// One bounded artifact range response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactRange {
    /// Inclusive returned byte start.
    pub start: u64,
    /// Inclusive returned byte end.
    pub end: u64,
    /// Complete artifact size.
    pub complete_size: u64,
    /// Declared media type.
    pub content_type: String,
    /// Bounded response bytes; the caller verifies the assembled artifact's size and digest.
    pub bytes: Vec<u8>,
}

/// Authenticated transport used by the CLI and other Rust consumers.
///
/// Clones share the HTTP connection pool and immutable credential. Build a new client after
/// credential rotation and obtain fresh cursors when the daemon refuses old continuations.
#[derive(Clone)]
pub struct ControlClient {
    http: reqwest::Client,
    config: ClientConfig,
    credential: BearerCredential,
}

impl fmt::Debug for ControlClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ControlClient")
            .field("endpoint", &self.config.endpoint)
            .field("request_timeout", &self.config.request_timeout)
            .field("credential", &"[redacted]")
            .finish_non_exhaustive()
    }
}

impl ControlClient {
    /// Constructs one client with redirects disabled and bounded timeouts.
    ///
    /// # Errors
    /// Refuses unsafe endpoint URLs, zero timeouts, or invalid artifact bounds, and reports
    /// failure to construct the HTTP transport. Construction performs no daemon request.
    pub fn new(config: ClientConfig, credential: BearerCredential) -> Result<Self, ClientError> {
        config.validate()?;
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(config.request_timeout.min(Duration::from_secs(10)))
            .timeout(config.request_timeout)
            .build()
            .map_err(redacted_transport)?;
        Ok(Self {
            http,
            config,
            credential,
        })
    }

    /// Confirms that the daemon uses the same current protocol version.
    ///
    /// # Errors
    /// Reports transport/API failure, an invalid response, or a daemon protocol version
    /// incompatible with this client.
    pub async fn negotiate(&self) -> Result<VersionResponse, ClientError> {
        let response: VersionResponse = self
            .json_request(
                Method::POST,
                "v1/version",
                Some(&VersionRequest {
                    protocol: ProtocolVersion::CURRENT,
                }),
                false,
            )
            .await?;
        response.protocol.negotiate()?;
        Ok(response)
    }

    /// Reads liveness state.
    ///
    /// # Errors
    /// Reports failure to obtain a bounded, decodable liveness response after the configured
    /// safe-query retries.
    pub async fn health(&self) -> Result<HealthRead, ClientError> {
        self.safe_get("v1/health").await
    }

    /// Reads readiness state; a non-ready daemon returns an availability error.
    ///
    /// # Errors
    /// Returns the daemon's availability refusal when it is not ready, or a transport or
    /// protocol failure after the configured safe-query retries.
    pub async fn readiness(&self) -> Result<HealthRead, ClientError> {
        self.safe_get("v1/readiness").await
    }

    /// Submits one exact idempotent command. The client does not retry it implicitly.
    ///
    /// # Errors
    /// Refuses invalid or oversized commands and reports daemon, transport, or response errors.
    /// A lost reply can follow durable acceptance; recover with the exact saved request.
    pub async fn submit(&self, request: &CommandRequest) -> Result<CommandAccepted, ClientError> {
        request.validate()?;
        let body = encode_json(request)?;
        let url = self.url("v1/commands")?;
        let response = self
            .authorized(
                self.http
                    .request(Method::POST, url)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(body),
            )
            .send()
            .await
            .map_err(|error| {
                if error.is_timeout() {
                    ClientError::Timeout
                } else {
                    redacted_transport(error)
                }
            })?;
        decode_response(response).await
    }

    /// Reads one immutable revision.
    ///
    /// # Errors
    /// Refuses an unsafe revision identity or reports failure to read its authorized projection.
    pub async fn revision(&self, revision: &str) -> Result<RevisionRead, ClientError> {
        self.safe_get(&format!("v1/revisions/{}", path_segment(revision)?))
            .await
    }

    /// Lists one bounded revision page.
    ///
    /// # Errors
    /// Refuses invalid page bounds or workflow filters, and reports daemon refusal or an
    /// unreadable response. An expired cursor requires a fresh authorized listing.
    pub async fn revisions(
        &self,
        workflow: Option<&str>,
        page: &PageRequest,
    ) -> Result<Page<RevisionSummary>, ClientError> {
        page.validate()?;
        let mut path = format!("v1/revisions?limit={}", page.limit);
        push_query(&mut path, "workflow", workflow)?;
        push_cursor(&mut path, page.cursor.as_ref());
        self.safe_get(&path).await
    }

    /// Compares two semantic revisions through a bounded structured diff.
    ///
    /// # Errors
    /// Refuses unsafe revision identities or reports failure to obtain the authorized diff.
    pub async fn revision_diff(
        &self,
        from: &str,
        to: &str,
    ) -> Result<RevisionDiffRead, ClientError> {
        self.safe_get(&format!(
            "v1/revisions/{}/diff/{}",
            path_segment(from)?,
            path_segment(to)?
        ))
        .await
    }

    /// Reads one compact current run.
    ///
    /// # Errors
    /// Refuses an unsafe run identity or reports failure to obtain its authorized current state.
    pub async fn run(&self, run: &str) -> Result<RunRead, ClientError> {
        self.safe_get(&format!("v1/runs/{}", path_segment(run)?))
            .await
    }

    /// Lists one bounded run page with optional stable filters.
    ///
    /// # Errors
    /// Refuses invalid page bounds or filters, and reports daemon refusal or an unreadable
    /// page. A refused continuation must be replaced with a fresh authorized listing.
    pub async fn runs(
        &self,
        state: Option<&str>,
        workflow: Option<&str>,
        page: &PageRequest,
    ) -> Result<Page<RunRead>, ClientError> {
        page.validate()?;
        let mut path = format!("v1/runs?limit={}", page.limit);
        push_query(&mut path, "state", state)?;
        push_query(&mut path, "workflow", workflow)?;
        push_cursor(&mut path, page.cursor.as_ref());
        self.safe_get(&path).await
    }

    /// Reads one exact node execution from a current run projection.
    ///
    /// # Errors
    /// Refuses unsafe run/execution identities or reports failure to read the authorized node.
    pub async fn node(
        &self,
        run: &str,
        execution: &str,
    ) -> Result<milkdrift_control_protocol::NodeRead, ClientError> {
        self.safe_get(&format!(
            "v1/runs/{}/nodes/{}",
            path_segment(run)?,
            path_segment(execution)?
        ))
        .await
    }

    /// Reads one exact current or historical attempt with authorized context provenance.
    ///
    /// # Errors
    /// Refuses unsafe run/attempt identities or reports failure to read authorized attempt evidence.
    pub async fn attempt(
        &self,
        run: &str,
        attempt: &str,
    ) -> Result<milkdrift_control_protocol::AttemptRead, ClientError> {
        self.safe_get(&format!(
            "v1/runs/{}/attempts/{}",
            path_segment(run)?,
            path_segment(attempt)?
        ))
        .await
    }

    /// Reads current progress, terminal outputs and currently permitted control actions.
    /// Output previews and attempt evidence are separately authorized by the daemon.
    ///
    /// # Errors
    /// Refuses an unsafe run identity or reports failure to read the authorized result projection.
    pub async fn run_result(
        &self,
        run: &str,
    ) -> Result<milkdrift_control_protocol::RunResultRead, ClientError> {
        self.safe_get(&format!("v1/runs/{}/result", path_segment(run)?))
            .await
    }

    /// Reads one bounded projected timeline page.
    ///
    /// # Errors
    /// Refuses invalid page bounds or an unsafe run identity, and reports daemon cursor/access
    /// refusal or failure to decode the bounded timeline response.
    pub async fn timeline(
        &self,
        run: &str,
        page: &PageRequest,
    ) -> Result<Page<TimelineEntry>, ClientError> {
        page.validate()?;
        let mut path = format!(
            "v1/runs/{}/timeline?limit={}",
            path_segment(run)?,
            page.limit
        );
        push_cursor(&mut path, page.cursor.as_ref());
        self.safe_get(&path).await
    }

    /// Lists capability generation observations visible to the actor.
    ///
    /// # Errors
    /// Reports daemon refusal or failure to read the bounded capability catalog after safe-query retries.
    pub async fn capabilities(&self) -> Result<Vec<CapabilityRead>, ClientError> {
        self.safe_get("v1/capabilities").await
    }

    /// Lists configured peer health and live catalog status without secret values.
    ///
    /// # Errors
    /// Reports daemon refusal or failure to decode the bounded peer-status response.
    pub async fn peers(&self) -> Result<Vec<PeerRead>, ClientError> {
        self.safe_get("v1/peers").await
    }

    /// Reads one configured peer relationship status.
    ///
    /// # Errors
    /// Refuses an unsafe peer identity or reports failure to obtain its authorized status.
    pub async fn peer(&self, peer: &str) -> Result<PeerRead, ClientError> {
        self.safe_get(&format!("v1/peers/{}", path_segment(peer)?))
            .await
    }

    /// Requests one explicit mutable peer lifecycle action.
    ///
    /// # Errors
    /// Refuses an unsupported action or unsafe peer identity and reports daemon/transport
    /// failures. The action is sent once; a lost reply does not establish non-execution.
    pub async fn peer_action(&self, peer: &str, action: &str) -> Result<PeerRead, ClientError> {
        if !matches!(
            action,
            "connect" | "reload" | "disconnect" | "drain" | "revoke"
        ) {
            return Err(ClientError::Configuration(
                "unsupported peer lifecycle action".to_owned(),
            ));
        }
        self.json_request::<(), PeerRead>(
            Method::POST,
            &format!("v1/peers/{}/{}", path_segment(peer)?, action),
            None,
            false,
        )
        .await
    }

    /// Lists one bounded proposal page for an exact run.
    ///
    /// # Errors
    /// Refuses invalid page bounds or an unsafe run identity, and reports access/cursor
    /// refusal or failure to decode the bounded proposal page.
    pub async fn proposals(
        &self,
        run: &str,
        page: &PageRequest,
    ) -> Result<Page<ProposalRead>, ClientError> {
        page.validate()?;
        let mut path = format!(
            "v1/runs/{}/proposals?limit={}",
            path_segment(run)?,
            page.limit
        );
        push_cursor(&mut path, page.cursor.as_ref());
        self.safe_get(&path).await
    }

    /// Reads one exact proposal/reconciliation status.
    ///
    /// # Errors
    /// Refuses unsafe run, proposal, or revision identities, and reports failure to read
    /// the exact authorized proposal status.
    pub async fn proposal(
        &self,
        run: &str,
        proposal: &str,
        proposed_revision: &str,
    ) -> Result<ProposalRead, ClientError> {
        self.safe_get(&format!(
            "v1/runs/{}/proposals/{}?revision={}",
            path_segment(run)?,
            path_segment(proposal)?,
            path_segment(proposed_revision)?
        ))
        .await
    }

    /// Reads current server-owned actor/grant context.
    ///
    /// # Errors
    /// Reports failure to obtain the daemon's bounded actor/grant context; no authority
    /// is inferred locally when the read fails.
    pub async fn authority(&self) -> Result<AuthorityRead, ClientError> {
        self.safe_get("v1/authority").await
    }

    /// Reads safe artifact metadata.
    ///
    /// # Errors
    /// Refuses an unsafe artifact identity or reports failure to read authorized metadata.
    pub async fn artifact_metadata(
        &self,
        artifact: &str,
    ) -> Result<ArtifactMetadataRead, ClientError> {
        self.safe_get(&format!("v1/artifacts/{}", path_segment(artifact)?))
            .await
    }

    /// Fetches one explicit bounded byte range without redirects or automatic retries.
    ///
    /// This bounds the response body; it does not hash a complete artifact. Read metadata
    /// first, assemble ranges, and verify total size and content digest before accepting a file.
    ///
    /// # Errors
    /// Refuses reversed, overflowing, or oversized ranges and unsafe artifact identities.
    /// Reports transport/API failure, mismatched range headers, and truncated or oversized
    /// bodies. Successful chunks still require the caller's complete-artifact verification.
    pub async fn artifact_range(
        &self,
        artifact: &str,
        start: u64,
        end: u64,
    ) -> Result<ArtifactRange, ClientError> {
        let requested = end
            .checked_sub(start)
            .and_then(|value| value.checked_add(1))
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| ClientError::Configuration("invalid artifact range".to_owned()))?;
        if requested > self.config.max_artifact_range_bytes {
            return Err(ClientError::Configuration(format!(
                "artifact range exceeds {} bytes",
                self.config.max_artifact_range_bytes
            )));
        }
        let url = self.url(&format!("v1/artifacts/{}/content", path_segment(artifact)?))?;
        let response = self
            .authorized(
                self.http
                    .get(url)
                    .header(header::RANGE, format!("bytes={start}-{end}")),
            )
            .send()
            .await
            .map_err(redacted_transport)?;
        if !response.status().is_success() {
            return Err(read_error(response).await);
        }
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_owned();
        let (returned_start, returned_end, complete_size) = response
            .headers()
            .get(header::CONTENT_RANGE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("bytes "))
            .and_then(|value| value.split_once('/'))
            .and_then(|(range, size)| {
                let (start, end) = range.split_once('-')?;
                Some((
                    start.parse::<u64>().ok()?,
                    end.parse::<u64>().ok()?,
                    size.parse::<u64>().ok()?,
                ))
            })
            .ok_or_else(|| {
                ClientError::Stream("artifact response has no valid Content-Range".to_owned())
            })?;
        if response.status() != StatusCode::PARTIAL_CONTENT
            || returned_start != start
            || returned_end < returned_start
            || returned_end > end
            || returned_end >= complete_size
        {
            return Err(ClientError::Stream(
                "artifact response range differs from the requested content".to_owned(),
            ));
        }
        let bytes = bounded_body(
            response,
            requested.min(self.config.max_artifact_range_bytes),
        )
        .await?;
        if u64::try_from(bytes.len()).ok() != Some(returned_end - returned_start + 1) {
            return Err(ClientError::Stream(
                "artifact response is incomplete".to_owned(),
            ));
        }
        Ok(ArtifactRange {
            start,
            end: returned_end,
            complete_size,
            content_type,
            bytes,
        })
    }

    /// Reads layout independently from semantic revision identity.
    ///
    /// # Errors
    /// Refuses unsafe workflow/revision identities or reports failure to read the authorized layout.
    pub async fn layout(
        &self,
        workflow: &str,
        revision: &str,
    ) -> Result<LayoutDocument, ClientError> {
        self.safe_get(&format!(
            "v1/layouts/{}/{}",
            path_segment(workflow)?,
            path_segment(revision)?
        ))
        .await
    }

    /// Opens an SSE feed and reconnects with the latest observed cursor.
    ///
    /// Retryable failures are yielded before reconnecting; there is no overall retry limit.
    /// The consumer owns its deadline and handles `ResyncRequired`/`StreamClosing` observations,
    /// which are yielded once before ending. A resync requires an authorized fresh read and a
    /// new subscription without a cursor. Duplicate or older feed positions are suppressed.
    /// The resume cursor advances on decoding, not on durable consumer acknowledgement.
    /// A nonretryable API error or malformed frame ends the stream. Dropping it stops local
    /// observation without cancelling work on the daemon. Heartbeat comments are ignored.
    /// Run feeds use `v1/runs/{raw_run_identity}/stream`: the identity remains raw here,
    /// including slashes, and is encoded once for transport. Cursors bind that raw identity.
    pub fn subscribe(
        &self,
        feed_path: impl Into<String>,
        cursor: Option<Cursor>,
    ) -> Pin<Box<dyn Stream<Item = Result<ObservationEnvelope, ClientError>> + Send>> {
        let client = self.clone();
        let feed_path = feed_path.into();
        Box::pin(async_stream::stream! {
            let (feed_path, feed) = match observation_route(&feed_path) {
                Ok(route) => route,
                Err(error) => { yield Err(error); return; }
            };
            let mut resume = cursor;
            let mut position = match resume.as_ref().map(|cursor| cursor.position_for(&feed)).transpose() {
                Ok(position) => position,
                Err(error) => { yield Err(error.into()); return; }
            };
            loop {
                let mut path = feed_path.clone();
                push_cursor(&mut path, resume.as_ref());
                let url = match client.url(&path) {
                    Ok(url) => url,
                    Err(error) => {
                        yield Err(error);
                        break;
                    }
                };
                let response = match client.authorized(client.http.get(url).timeout(Duration::from_secs(86_400))).send().await {
                    Ok(response) if response.status().is_success() => response,
                    Ok(response) => {
                        let error = read_error(response).await;
                        let retryable = error.retryable();
                        yield Err(error);
                        if !retryable { break; }
                        tokio::time::sleep(client.config.retry_delay).await;
                        continue;
                    }
                    Err(error) => {
                        yield Err(redacted_transport(error));
                        tokio::time::sleep(client.config.retry_delay).await;
                        continue;
                    }
                };
                let mut bytes = response.bytes_stream();
                let mut buffer = Vec::new();
                let mut reconnect = false;
                while let Some(chunk) = bytes.next().await {
                    let chunk = match chunk {
                        Ok(chunk) => chunk,
                        Err(error) => {
                            yield Err(redacted_transport(error));
                            reconnect = true;
                            break;
                        }
                    };
                    if buffer.len().saturating_add(chunk.len()) > milkdrift_control_protocol::MAX_DOCUMENT_BYTES * 2 {
                        yield Err(ClientError::Stream("SSE frame exceeds client bound".to_owned()));
                        return;
                    }
                    buffer.extend_from_slice(&chunk);
                    while let Some(boundary) = find_sse_boundary(&buffer) {
                        let frame = buffer.drain(..boundary).collect::<Vec<_>>();
                        drain_sse_boundary(&mut buffer);
                        match parse_sse_data(&frame) {
                            Ok(None) => {}
                            Ok(Some(observation)) => {
                                if observation.feed != feed {
                                    yield Err(ClientError::Stream("observation belongs to another feed".to_owned()));
                                    return;
                                }
                                let next = match observation.cursor.position_for(&feed) {
                                    Ok(next) => next,
                                    Err(error) => { yield Err(error.into()); return; }
                                };
                                if matches!(observation.observation, milkdrift_control_protocol::Observation::ResyncRequired { .. } | milkdrift_control_protocol::Observation::StreamClosing { .. }) {
                                    yield Ok(observation);
                                    return;
                                }
                                if position.is_some_and(|position| next <= position) { continue; }
                                position = Some(next);
                                resume = Some(observation.cursor.clone());
                                yield Ok(observation);
                            }
                            Err(error) => {
                                yield Err(error);
                                return;
                            }
                        }
                    }
                }
                if !reconnect {
                    if !buffer.is_empty() {
                        yield Err(ClientError::Stream("server truncated an observation frame".to_owned()));
                        return;
                    }
                    yield Err(ClientError::Transport("server closed the observation stream".to_owned()));
                }
                tokio::time::sleep(client.config.retry_delay).await;
            }
        })
    }

    async fn safe_get<T: DeserializeOwned>(&self, path: &str) -> Result<T, ClientError> {
        let mut attempts = 0_u8;
        loop {
            match self
                .json_request::<(), T>(Method::GET, path, None, true)
                .await
            {
                Ok(value) => return Ok(value),
                Err(error) if error.retryable() && attempts < self.config.safe_query_retries => {
                    attempts = attempts.saturating_add(1);
                    tokio::time::sleep(self.config.retry_delay).await;
                }
                Err(error) => return Err(error),
            }
        }
    }

    async fn json_request<B: serde::Serialize + ?Sized, T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<&B>,
        _safe: bool,
    ) -> Result<T, ClientError> {
        let url = self.url(path)?;
        let mut request = self.authorized(self.http.request(method, url));
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send().await.map_err(|error| {
            if error.is_timeout() {
                ClientError::Timeout
            } else {
                redacted_transport(error)
            }
        })?;
        if !response.status().is_success() {
            return Err(read_error(response).await);
        }
        decode_response(response).await
    }

    fn authorized(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        builder
            .bearer_auth(self.credential.expose())
            .header(header::ACCEPT, "application/json")
            .header(
                "x-milkdrift-protocol",
                format!(
                    "{}.{}",
                    ProtocolVersion::CURRENT.major,
                    ProtocolVersion::CURRENT.minor
                ),
            )
    }

    fn url(&self, path: &str) -> Result<Url, ClientError> {
        self.config
            .endpoint
            .join(path)
            .map_err(|_| ClientError::Configuration("invalid control request path".to_owned()))
    }
}

async fn decode_response<T: DeserializeOwned>(
    response: reqwest::Response,
) -> Result<T, ClientError> {
    if !response.status().is_success() {
        return Err(read_error(response).await);
    }
    let bytes = bounded_body(response, milkdrift_control_protocol::MAX_DOCUMENT_BYTES).await?;
    let envelope: ResponseEnvelope<T> = decode_json(&bytes)?;
    envelope.protocol.negotiate()?;
    Ok(envelope.value)
}

async fn read_error(response: reqwest::Response) -> ClientError {
    let status = response.status();
    match bounded_body(response, milkdrift_control_protocol::MAX_DOCUMENT_BYTES).await {
        Ok(bytes) => match decode_json::<ErrorEnvelope>(&bytes) {
            Ok(error) => match error.protocol.negotiate() {
                Ok(_) => ClientError::Api(error),
                Err(error) => ClientError::Protocol(error),
            },
            Err(_) => ClientError::Transport(format!(
                "daemon returned HTTP {} with an invalid redacted error envelope",
                status.as_u16()
            )),
        },
        Err(_) => ClientError::Transport(format!(
            "daemon returned HTTP {} and its error body could not be read",
            status.as_u16()
        )),
    }
}

async fn bounded_body(response: reqwest::Response, maximum: usize) -> Result<Vec<u8>, ClientError> {
    let oversized = || {
        ClientError::Protocol(ProtocolError::Bounds(
            "response exceeds client byte bound".to_owned(),
        ))
    };
    if response
        .content_length()
        .is_some_and(|size| size > maximum as u64)
    {
        return Err(oversized());
    }
    let mut chunks = response.bytes_stream();
    let mut body = Vec::new();
    while let Some(chunk) = chunks.next().await {
        let chunk = chunk.map_err(redacted_transport)?;
        if body.len().saturating_add(chunk.len()) > maximum {
            return Err(oversized());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn redacted_transport(error: reqwest::Error) -> ClientError {
    if error.is_timeout() {
        return ClientError::Timeout;
    }
    let classification = if error.is_connect() {
        "connection failed"
    } else if error.is_decode() {
        "response decoding failed"
    } else if error.is_request() {
        "request construction failed"
    } else {
        "HTTP exchange failed"
    };
    ClientError::Transport(classification.to_owned())
}

fn path_segment(value: &str) -> Result<String, ClientError> {
    if value.is_empty()
        || matches!(value, "." | "..")
        || value.len() > 256
        || !value.is_ascii()
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/' | b'%')
        })
    {
        return Err(ClientError::Configuration(
            "resource identity is not a safe path segment".to_owned(),
        ));
    }
    // Push the raw identity as one segment. In particular a slash is data and
    // a literal percent sequence must survive the server's single decoding pass.
    // Standalone dot segments cannot be carried by WHATWG URLs; domain identities
    // already require an initial alphanumeric character and never admit those.
    let mut encoded = Url::parse("http://localhost/")
        .map_err(|_| ClientError::Configuration("invalid identity encoder base".into()))?;
    encoded
        .path_segments_mut()
        .map_err(|()| ClientError::Configuration("invalid identity encoder path".into()))?
        .push(value);
    Ok(encoded.path().trim_start_matches('/').to_owned())
}

fn observation_route(path: &str) -> Result<(String, String), ClientError> {
    if path == "v1/stream/health" {
        return Ok((path.into(), "daemon-health".into()));
    }
    if path == "v1/stream/capabilities" {
        return Ok((path.into(), "capability-health".into()));
    }
    if let Some(run) = path
        .strip_prefix("v1/runs/")
        .and_then(|path| path.strip_suffix("/stream"))
    {
        return Ok((
            format!("v1/runs/{}/stream", path_segment(run)?),
            format!("run:{run}"),
        ));
    }
    Err(ClientError::Configuration(
        "unsupported observation feed".to_owned(),
    ))
}

fn push_query(path: &mut String, name: &str, value: Option<&str>) -> Result<(), ClientError> {
    if let Some(value) = value {
        let value = path_segment(value)?;
        path.push('&');
        path.push_str(name);
        path.push('=');
        path.push_str(&value);
    }
    Ok(())
}

fn push_cursor(path: &mut String, cursor: Option<&Cursor>) {
    if let Some(cursor) = cursor {
        path.push(if path.contains('?') { '&' } else { '?' });
        path.push_str("cursor=");
        path.push_str(cursor.as_str());
    }
}

fn find_sse_boundary(bytes: &[u8]) -> Option<usize> {
    bytes
        .windows(2)
        .position(|window| window == b"\n\n")
        .or_else(|| bytes.windows(4).position(|window| window == b"\r\n\r\n"))
}

fn drain_sse_boundary(bytes: &mut Vec<u8>) {
    if bytes.starts_with(b"\r\n\r\n") {
        bytes.drain(..4);
    } else if bytes.starts_with(b"\n\n") {
        bytes.drain(..2);
    }
}

fn parse_sse_data(frame: &[u8]) -> Result<Option<ObservationEnvelope>, ClientError> {
    let text = std::str::from_utf8(frame)
        .map_err(|_| ClientError::Stream("SSE frame is not UTF-8".to_owned()))?;
    let mut data = String::new();
    for line in text.lines() {
        if line.starts_with(':') {
            continue;
        }
        if let Some(value) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(value.strip_prefix(' ').unwrap_or(value));
        }
    }
    if data.is_empty() {
        return Ok(None);
    }
    let envelope: ObservationEnvelope = decode_json(data.as_bytes())?;
    envelope.protocol.negotiate()?;
    Ok(Some(envelope))
}

/// Maps a client result into coarse categories useful to CLI exit-code policy.
#[must_use]
pub fn status_class(error: &ClientError) -> Option<StatusCode> {
    match error {
        ClientError::Api(error) => Some(match error.code {
            milkdrift_control_protocol::ErrorCode::Unauthenticated => StatusCode::UNAUTHORIZED,
            milkdrift_control_protocol::ErrorCode::Unauthorized => StatusCode::FORBIDDEN,
            milkdrift_control_protocol::ErrorCode::InvalidInput => StatusCode::BAD_REQUEST,
            milkdrift_control_protocol::ErrorCode::Conflict => StatusCode::CONFLICT,
            milkdrift_control_protocol::ErrorCode::NotFound => StatusCode::NOT_FOUND,
            milkdrift_control_protocol::ErrorCode::Overload => StatusCode::TOO_MANY_REQUESTS,
            milkdrift_control_protocol::ErrorCode::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
            milkdrift_control_protocol::ErrorCode::UnsupportedVersion => {
                StatusCode::UPGRADE_REQUIRED
            }
            milkdrift_control_protocol::ErrorCode::Timeout => StatusCode::GATEWAY_TIMEOUT,
            milkdrift_control_protocol::ErrorCode::Corruption
            | milkdrift_control_protocol::ErrorCode::Uncertain
            | milkdrift_control_protocol::ErrorCode::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use milkdrift_control_protocol::{Observation, ProtocolVersion, TimelineCategory};
    use serde_json::Value;

    #[test]
    fn resource_segments_preserve_data_and_stream_cursors_use_raw_names()
    -> Result<(), Box<dyn std::error::Error>> {
        let endpoint = Url::parse("http://127.0.0.1:9734/prefix/")?;
        for (raw, encoded) in [
            ("team/build-1", "team%2Fbuild-1"),
            ("team/../build/./", "team%2F..%2Fbuild%2F.%2F"),
            ("team//build", "team%2F%2Fbuild"),
            ("team%2Fbuild", "team%252Fbuild"),
            ("team:._-9", "team:._-9"),
        ] {
            assert_eq!(path_segment(raw)?, encoded);
            let path = format!("v1/runs/{raw}/stream");
            let (route, feed) = observation_route(&path)?;
            assert_eq!(feed, format!("run:{raw}"));
            assert_eq!(
                endpoint.join(&route)?.path(),
                format!("/prefix/v1/runs/{encoded}/stream")
            );
            let cursor = Cursor::new(&feed, 7)?;
            assert_eq!(cursor.position_for(&feed)?, 7);
            if raw != encoded {
                assert!(cursor.position_for(&format!("run:{encoded}")).is_err());
            }
        }
        for invalid in ["", ".", "..", "a?x=1", "a#fragment", "a&b", "a\n"] {
            assert!(path_segment(invalid).is_err());
        }
        Ok(())
    }

    #[test]
    fn credential_and_client_debug_are_redacted() -> Result<(), Box<dyn std::error::Error>> {
        let credential = BearerCredential::new("top-secret-value")?;
        assert!(!format!("{credential:?}").contains("top-secret-value"));
        let client = ControlClient::new(
            ClientConfig::new(Url::parse("http://127.0.0.1:9734/")?),
            credential,
        )?;
        assert!(!format!("{client:?}").contains("top-secret-value"));
        Ok(())
    }

    #[test]
    fn sse_parser_ignores_heartbeats_and_reads_external_envelopes()
    -> Result<(), Box<dyn std::error::Error>> {
        assert!(
            parse_sse_data(b": heartbeat")
                .map_err(|error| error.to_string())?
                .is_none()
        );
        let mut observation = ObservationEnvelope {
            protocol: ProtocolVersion::CURRENT,
            cursor: Cursor::new("run:test", 1)?,
            observed_at_ms: 1,
            feed: "run:test".to_owned(),
            observation: Observation::Timeline(TimelineEntry {
                sequence: 1,
                timestamp_ms: 1,
                category: TimelineCategory::Lifecycle,
                actor: "human:test".to_owned(),
                run_id: "test".to_owned(),
                node_id: None,
                attempt_id: None,
                revision_id: None,
                summary: "created".to_owned(),
                detail: Value::Null,
            }),
        };
        let frame = format!("data: {}", serde_json::to_string(&observation)?);
        assert_eq!(
            parse_sse_data(frame.as_bytes()).map_err(|error| error.to_string())?,
            Some(observation.clone())
        );
        for minor in [
            0,
            ProtocolVersion::CURRENT.minor - 1,
            ProtocolVersion::CURRENT.minor + 1,
        ] {
            observation.protocol.minor = minor;
            let frame = format!("data: {}", serde_json::to_string(&observation)?);
            assert!(matches!(
                parse_sse_data(frame.as_bytes()),
                Err(ClientError::Protocol(
                    ProtocolError::UnsupportedMinor { .. }
                ))
            ));
        }
        Ok(())
    }
}
