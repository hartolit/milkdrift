//! Reject unconfigured browser origins before authentication or durable owner dispatch.

use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Request, State},
    http::{HeaderValue, Method, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use milkdrift_control_protocol::ErrorCode;

use super::response::ApiError;

const REQUEST_HEADERS: [&str; 5] = [
    "authorization",
    "content-type",
    "last-event-id",
    "x-request-id",
    "x-milkdrift-daemon",
];

pub(super) async fn allow_origin(
    State(origins): State<Arc<Vec<String>>>,
    request: Request,
    next: Next,
) -> Response {
    let Some(origin) = request.headers().get(header::ORIGIN).cloned() else {
        return next.run(request).await;
    };
    let allowed = origin
        .to_str()
        .is_ok_and(|origin| origins.iter().any(|allowed| allowed == origin));
    if !allowed {
        return ApiError::new(
            StatusCode::FORBIDDEN,
            ErrorCode::Unauthorized,
            "browser origin is not configured for this daemon",
            false,
            None,
        )
        .into_response();
    }
    let preflight = request.method() == Method::OPTIONS;
    let mut response = if preflight {
        let valid_method = request
            .headers()
            .get(header::ACCESS_CONTROL_REQUEST_METHOD)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| matches!(value, "GET" | "POST" | "PUT"));
        let valid_headers = request
            .headers()
            .get(header::ACCESS_CONTROL_REQUEST_HEADERS)
            .is_none_or(|value| {
                value.to_str().is_ok_and(|value| {
                    value.len() <= 256
                        && value.split(',').all(|name| {
                            REQUEST_HEADERS
                                .iter()
                                .any(|allowed| name.trim().eq_ignore_ascii_case(allowed))
                        })
                })
            });
        if !valid_method || !valid_headers {
            ApiError::new(
                StatusCode::FORBIDDEN,
                ErrorCode::Unauthorized,
                "browser preflight method or headers are not allowed",
                false,
                None,
            )
            .into_response()
        } else {
            let mut response = Response::new(Body::empty());
            *response.status_mut() = StatusCode::NO_CONTENT;
            response.headers_mut().insert(
                header::ACCESS_CONTROL_ALLOW_METHODS,
                HeaderValue::from_static("GET, POST, PUT"),
            );
            response.headers_mut().insert(
                header::ACCESS_CONTROL_ALLOW_HEADERS,
                HeaderValue::from_static(
                    "authorization, content-type, last-event-id, x-request-id, x-milkdrift-daemon",
                ),
            );
            response.headers_mut().insert(
                header::ACCESS_CONTROL_MAX_AGE,
                HeaderValue::from_static("300"),
            );
            response
        }
    } else {
        next.run(request).await
    };
    response
        .headers_mut()
        .insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
    response.headers_mut().insert(
        header::VARY,
        HeaderValue::from_static(
            "Origin, Access-Control-Request-Method, Access-Control-Request-Headers",
        ),
    );
    response.headers_mut().insert(
        header::ACCESS_CONTROL_EXPOSE_HEADERS,
        HeaderValue::from_static(
            "content-type, content-length, content-range, x-request-id, x-milkdrift-daemon",
        ),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

#[cfg(test)]
mod tests {
    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode, header},
        routing::post,
    };
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tower::ServiceExt as _;

    #[tokio::test]
    async fn origin_refusal_precedes_effect_and_preflight_is_bounded()
    -> Result<(), Box<dyn std::error::Error>> {
        let entries = Arc::new(AtomicUsize::new(0));
        let observed = entries.clone();
        let router = Router::new()
            .route(
                "/task",
                post(move || {
                    let entries = observed.clone();
                    async move {
                        entries.fetch_add(1, Ordering::SeqCst);
                        "accepted"
                    }
                }),
            )
            .layer(axum::middleware::from_fn_with_state(
                Arc::new(vec!["https://work.example".to_owned()]),
                super::allow_origin,
            ));
        let refused = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/task")
                    .header(header::ORIGIN, "https://other.example")
                    .body(Body::empty())?,
            )
            .await?;
        assert_eq!(refused.status(), StatusCode::FORBIDDEN);
        assert_eq!(entries.load(Ordering::SeqCst), 0);
        assert!(
            !refused
                .headers()
                .contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN)
        );
        let preflight = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("OPTIONS")
                    .uri("/task")
                    .header(header::ORIGIN, "https://work.example")
                    .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
                    .header(
                        header::ACCESS_CONTROL_REQUEST_HEADERS,
                        "authorization, content-type, x-milkdrift-daemon",
                    )
                    .body(Body::empty())?,
            )
            .await?;
        assert_eq!(preflight.status(), StatusCode::NO_CONTENT);
        assert_eq!(
            preflight
                .headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .and_then(|value| value.to_str().ok()),
            Some("https://work.example")
        );
        assert_eq!(entries.load(Ordering::SeqCst), 0);
        let invalid = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("OPTIONS")
                    .uri("/task")
                    .header(header::ORIGIN, "https://work.example")
                    .header(header::ACCESS_CONTROL_REQUEST_METHOD, "DELETE")
                    .body(Body::empty())?,
            )
            .await?;
        assert_eq!(invalid.status(), StatusCode::FORBIDDEN);
        let accepted = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/task")
                    .header(header::ORIGIN, "https://work.example")
                    .body(Body::empty())?,
            )
            .await?;
        assert_eq!(accepted.status(), StatusCode::OK);
        assert_eq!(entries.load(Ordering::SeqCst), 1);
        Ok(())
    }
}
