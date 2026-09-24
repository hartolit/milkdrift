use super::{
    Candidate,
    bookings::{Bookings, Failure, Reservation, Timestamp},
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::json;
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Application {
    resource: String,
    capacity: u32,
}
struct ApplicationState {
    resource: String,
    token: String,
    candidate: Candidate,
    bookings: Mutex<Bookings>,
}
type Shared = Arc<ApplicationState>;

impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::Invalid => (
                StatusCode::BAD_REQUEST,
                "invalid reservation or UTC interval",
            ),
            Self::Conflict => (StatusCode::CONFLICT, "capacity or cancellation constraint"),
            Self::Missing => (StatusCode::NOT_FOUND, "reservation absent"),
            Self::Full => (
                StatusCode::INSUFFICIENT_STORAGE,
                "booking retention limit reached",
            ),
            Self::Unavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "booking storage unavailable",
            ),
        };
        (status, Json(json!({"error": message}))).into_response()
    }
}
fn authorize(
    state: &ApplicationState,
    headers: &HeaderMap,
    mutation: bool,
) -> Result<(), StatusCode> {
    if mutation && state.candidate == Candidate::SeededFixture {
        return Ok(());
    }
    if headers.get("authorization").and_then(|v| v.to_str().ok())
        == Some(format!("Bearer {}", state.token).as_str())
    {
        return Ok(());
    }
    Err(StatusCode::UNAUTHORIZED)
}
async fn health() -> Json<serde_json::Value> {
    Json(json!({"status":"ready"}))
}
async fn active(State(state): State<Shared>, headers: HeaderMap) -> Response {
    if let Err(response) = authorize(&state, &headers, false) {
        return response.into_response();
    }
    match state
        .bookings
        .lock()
        .map_err(|_| Failure::Unavailable)
        .and_then(|store| store.active())
    {
        Ok(rows) => Json(rows.into_iter().map(|row| json!({
            "id":row.id,"name":row.name,"start":row.start,"end":row.end,"quantity":row.quantity
        })).collect::<Vec<_>>()).into_response(),
        Err(error) => error.into_response(),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Interval {
    start: String,
    end: String,
}
async fn availability(
    State(state): State<Shared>,
    Query(interval): Query<Interval>,
) -> Result<Json<serde_json::Value>, Failure> {
    let start = Timestamp::parse(&interval.start)?;
    let end = Timestamp::parse(&interval.end)?;
    let remaining = state
        .bookings
        .lock()
        .map_err(|_| Failure::Unavailable)?
        .remaining(&start, &end)?;
    Ok(Json(
        json!({"resource":state.resource,"start":interval.start,"end":interval.end,"remaining":remaining}),
    ))
}
async fn create(
    State(state): State<Shared>,
    headers: HeaderMap,
    body: Result<Json<Reservation>, axum::extract::rejection::JsonRejection>,
) -> Response {
    if let Err(response) = authorize(&state, &headers, true) {
        return response.into_response();
    }
    let Ok(Json(request)) = body else {
        return Failure::Invalid.into_response();
    };
    match state
        .bookings
        .lock()
        .map_err(|_| Failure::Unavailable)
        .and_then(|mut store| store.create(request))
    {
        Ok(id) => (StatusCode::CREATED, Json(json!({"id":id}))).into_response(),
        Err(error) => error.into_response(),
    }
}
async fn cancel(State(state): State<Shared>, headers: HeaderMap, Path(id): Path<u64>) -> Response {
    if let Err(response) = authorize(&state, &headers, true) {
        return response.into_response();
    }
    let result = fs::read_to_string("/config/clock")
        .map_err(|_| Failure::Unavailable)
        .and_then(|clock| Timestamp::parse(clock.trim()))
        .and_then(|clock| {
            state
                .bookings
                .lock()
                .map_err(|_| Failure::Unavailable)?
                .cancel(id, &clock)
        });
    match result {
        Ok(()) => Json(json!({"cancelled":true})).into_response(),
        Err(error) => error.into_response(),
    }
}
pub(super) async fn serve(candidate: Candidate) -> Result<(), Box<dyn std::error::Error>> {
    let application: Application = serde_json::from_slice(&fs::read("/config/application.json")?)?;
    if application.resource.is_empty() || application.resource.len() > 128 {
        return Err("invalid resource identity".into());
    }
    let token = fs::read_to_string("/config/token")?.trim().to_owned();
    if token.is_empty() || token.len() > 1024 {
        return Err("invalid service credential".into());
    }
    let bookings = Bookings::open(
        application.capacity,
        (candidate == Candidate::Corrected).then(|| PathBuf::from("/data/bookings.json")),
    )?;
    let state = Arc::new(ApplicationState {
        resource: application.resource,
        token,
        candidate,
        bookings: Mutex::new(bookings),
    });
    let app = Router::new()
        .route("/health", get(health))
        .route("/availability", get(availability))
        .route("/reservations", post(create).get(active))
        .route("/reservations/{id}", axum::routing::delete(cancel))
        .layer(DefaultBodyLimit::max(8192))
        .with_state(state);
    axum::serve(tokio::net::TcpListener::bind("0.0.0.0:8080").await?, app).await?;
    Ok(())
}
