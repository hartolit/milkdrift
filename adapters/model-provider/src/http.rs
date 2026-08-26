use std::{
    io::Read,
    sync::atomic::{AtomicBool, Ordering},
};

use reqwest::blocking::{Client, Response};
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use serde_json::Value;
use thiserror::Error;

use milkdrift_capability::ErrorClass;

use crate::{
    EndpointProfile, ProxyPolicy, RedirectPolicy,
    profile::AuthMode,
    stream::{SseParser, StreamError},
};

pub(crate) fn client(profile: &EndpointProfile) -> Result<Client, HttpError> {
    let limits = profile.limits();
    let base = profile
        .endpoint_url()
        .map_err(|_| HttpError::Policy("invalid endpoint URL"))?;
    let base_origin = (
        base.scheme().to_owned(),
        base.host_str().unwrap_or_default().to_owned(),
        base.port_or_known_default(),
    );
    let redirect = match profile.redirect() {
        RedirectPolicy::Deny => reqwest::redirect::Policy::none(),
        RedirectPolicy::SameOrigin => reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() >= 5 {
                return attempt.stop();
            }
            let next = attempt.url();
            let origin = (
                next.scheme().to_owned(),
                next.host_str().unwrap_or_default().to_owned(),
                next.port_or_known_default(),
            );
            if origin == base_origin {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }),
    };
    let mut builder = Client::builder()
        .connect_timeout(std::time::Duration::from_millis(limits.connect_timeout_ms))
        // The blocking client does not expose a distinct per-read timer. Use the
        // smaller configured value as a conservative whole-request cap so the
        // advertised idle bound can never be exceeded.
        .timeout(std::time::Duration::from_millis(
            limits.request_timeout_ms.min(limits.idle_timeout_ms),
        ))
        .redirect(redirect)
        .user_agent("milkdrift-model-provider/0.1");
    if profile.proxy() == ProxyPolicy::Disabled {
        builder = builder.no_proxy();
    }
    builder.build().map_err(|_| HttpError::Transport)
}

pub(crate) fn headers(
    profile: &EndpointProfile,
    secret: Option<&[u8]>,
) -> Result<HeaderMap, HttpError> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("application/json, text/event-stream"),
    );
    match profile.auth() {
        AuthMode::NoAuth => {
            if secret.is_some() {
                return Err(HttpError::Policy("unexpected secret for no-auth profile"));
            }
        }
        AuthMode::Bearer { .. } => {
            let bytes = secret.ok_or(HttpError::Policy("authorization secret unavailable"))?;
            let text = std::str::from_utf8(bytes)
                .map_err(|_| HttpError::Policy("authorization secret is not UTF-8"))?;
            let value = HeaderValue::from_str(&format!("Bearer {text}")).map_err(|_| {
                HttpError::Policy("authorization secret is not a valid header value")
            })?;
            headers.insert(AUTHORIZATION, value);
        }
        AuthMode::AnthropicApiKey { .. } => {
            let bytes = secret.ok_or(HttpError::Policy("authorization secret unavailable"))?;
            let value = HeaderValue::from_bytes(bytes).map_err(|_| {
                HttpError::Policy("authorization secret is not a valid header value")
            })?;
            headers.insert(HeaderName::from_static("x-api-key"), value);
        }
    }
    Ok(headers)
}

pub(crate) fn validate_headers(
    profile: &EndpointProfile,
    response: &Response,
) -> Result<(), HttpError> {
    let headers = response.headers();
    if headers.len() > usize::from(profile.limits().max_headers) {
        return Err(HttpError::ResponseTooLarge);
    }
    let bytes = headers.iter().try_fold(0usize, |total, (name, value)| {
        total
            .checked_add(name.as_str().len())
            .and_then(|n| n.checked_add(value.as_bytes().len()))
            .ok_or(HttpError::ResponseTooLarge)
    })?;
    if bytes > profile.limits().max_header_bytes as usize {
        return Err(HttpError::ResponseTooLarge);
    }
    if headers
        .get("content-encoding")
        .is_some_and(|value| value.as_bytes() != b"identity")
    {
        return Err(HttpError::Policy("compressed model responses are disabled"));
    }
    Ok(())
}

pub(crate) fn read_json(profile: &EndpointProfile, response: Response) -> Result<Value, HttpError> {
    validate_headers(profile, &response)?;
    let mut bytes = Vec::new();
    response
        .take(profile.limits().max_response_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|_| HttpError::Transport)?;
    if bytes.len() as u64 > profile.limits().max_response_bytes {
        return Err(HttpError::ResponseTooLarge);
    }
    serde_json::from_slice(&bytes).map_err(|_| HttpError::MalformedResponse)
}

pub(crate) fn read_sse(
    profile: &EndpointProfile,
    mut response: Response,
    cancelled: &AtomicBool,
    mut event: impl FnMut(&str) -> Result<(), HttpError>,
) -> Result<(), HttpError> {
    validate_headers(profile, &response)?;
    let mut parser = SseParser::new(
        profile.limits().max_stream_line_bytes,
        profile.limits().max_stream_event_bytes,
    );
    let mut buffer = [0u8; 8192];
    let mut total = 0u64;
    loop {
        if cancelled.load(Ordering::SeqCst) {
            return Err(HttpError::Cancelled);
        }
        let read = response
            .read(&mut buffer)
            .map_err(|_| HttpError::Transport)?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(read as u64)
            .ok_or(HttpError::ResponseTooLarge)?;
        if total > profile.limits().max_response_bytes {
            return Err(HttpError::ResponseTooLarge);
        }
        let mut callback_error = None;
        let parsed = parser.push(&buffer[..read], |data| match event(data) {
            Ok(()) => Ok(()),
            Err(error) => {
                callback_error = Some(error);
                Err(StreamError::MalformedField)
            }
        });
        if let Some(error) = callback_error {
            return Err(error);
        }
        parsed?;
    }
    parser.finish()?;
    Ok(())
}

/// Bounded provider/transport failure with no response body or credential text.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub(crate) enum HttpError {
    #[error("endpoint policy rejected the request: {0}")]
    Policy(&'static str),
    #[error("model endpoint transport failed")]
    Transport,
    #[error("model response exceeded configured bounds")]
    ResponseTooLarge,
    #[error("model response was malformed")]
    MalformedResponse,
    #[error("model invocation was cancelled")]
    Cancelled,
    #[error("model stream was malformed: {0}")]
    Stream(#[from] StreamError),
}

impl HttpError {
    pub(crate) const fn class(self) -> ErrorClass {
        match self {
            Self::Policy(_) => ErrorClass::Unsupported,
            Self::Transport => ErrorClass::Transport,
            Self::ResponseTooLarge | Self::MalformedResponse | Self::Stream(_) => {
                ErrorClass::Provider
            }
            Self::Cancelled => ErrorClass::Unknown,
        }
    }
    pub(crate) const fn code(self) -> &'static str {
        match self {
            Self::Policy(_) => "endpoint_policy",
            Self::Transport => "transport",
            Self::ResponseTooLarge => "response_too_large",
            Self::MalformedResponse => "malformed_response",
            Self::Cancelled => "cancelled",
            Self::Stream(_) => "malformed_stream",
        }
    }
}

pub(crate) fn status_error(status: reqwest::StatusCode) -> ProviderStatus {
    let retryable = status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error();
    let class = if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        ErrorClass::RateLimit
    } else if status == reqwest::StatusCode::UNAUTHORIZED
        || status == reqwest::StatusCode::FORBIDDEN
    {
        ErrorClass::Authorization
    } else if status.is_server_error() {
        ErrorClass::Provider
    } else {
        ErrorClass::InvalidRequest
    };
    ProviderStatus {
        class,
        retryable,
        code: if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            "rate_limited"
        } else if status.is_server_error() {
            "provider_server_error"
        } else {
            "provider_request_rejected"
        },
    }
}

pub(crate) struct ProviderStatus {
    pub(crate) class: ErrorClass,
    pub(crate) retryable: bool,
    pub(crate) code: &'static str,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_statuses_have_stable_retry_and_authority_classes() {
        let rate = status_error(reqwest::StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(rate.class, ErrorClass::RateLimit);
        assert!(rate.retryable);
        assert_eq!(rate.code, "rate_limited");
        let unauthorized = status_error(reqwest::StatusCode::UNAUTHORIZED);
        assert_eq!(unauthorized.class, ErrorClass::Authorization);
        assert!(!unauthorized.retryable);
        let server = status_error(reqwest::StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(server.class, ErrorClass::Provider);
        assert!(server.retryable);
    }
}
