//! Streams transport adaptation over typed daemon calls.
use super::{
    ApiError, AppState, CAPABILITY_FEED_ITEMS, ListQuery, STREAM_PAGE_ITEMS, authenticate,
    bearer_header, owner_error, protocol_error,
};
use crate::{DaemonHost, auth::ActorSession, host::StreamAuthority};
use async_stream::stream;
use axum::{
    extract::Path, extract::Query, extract::State, http::HeaderMap, response::Sse,
    response::sse::Event, response::sse::KeepAlive,
};
use futures_util::Stream;
use milkdrift_control_protocol::{
    CapabilityRead, Cursor, CursorBinding, ErrorCode, Observation, ObservationEnvelope,
    ProtocolVersion,
};
use std::{collections::VecDeque, convert::Infallible, time::Duration};
use tracing::{info, warn};

#[derive(Default)]
pub(super) struct CapabilityFeed {
    last_snapshot_digest: Option<String>,
    next_position: u64,
    entries: VecDeque<(u64, CapabilityRead)>,
}

pub(super) async fn run_stream(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(run): Path<String>,
    Query(query): Query<ListQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let (request_id, initial_session) = authenticate(&state, &headers)?;
    let bearer =
        bearer_header(&headers).ok_or_else(|| ApiError::unauthenticated(request_id.clone()))?;
    let feed = format!("run:{run}");
    let initial_decision = stream_authority(
        &state,
        initial_session.clone(),
        StreamAuthority::Run(run.clone()),
        &request_id,
    )
    .await?;
    let initial_binding = stream_cursor_binding(&initial_session, &feed);
    let mut stream_position = query
        .cursor
        .as_ref()
        .map(|cursor| {
            cursor.position_for_bound(&feed, &initial_binding, initial_session.cursor_key())
        })
        .transpose()
        .map_err(|error| protocol_error(error, request_id.clone()))?
        .unwrap_or(0);
    info!(
        request_id,
        feed,
        resume = query.cursor.is_some(),
        "run stream subscription established"
    );
    let output = stream! {
        'events: loop {
            let Some(current_session) = state.host.authenticate_header(Some(&bearer)) else {
                if let Ok(event) = observation_event(&state.host, &feed, stream_position.saturating_add(1), Observation::StreamClosing { reason: "authorization was revoked or rotated".to_owned() }, &initial_session, &initial_decision).await {
                    yield Ok(event);
                }
                break;
            };
            let session = current_session;
            let decision = match stream_authority(
                &state,
                session.clone(),
                StreamAuthority::Run(run.clone()),
                &request_id,
            ).await {
                Ok(decision) => decision,
                Err(_) => break,
            };
            if state.host.health().draining {
                if let Ok(event) = observation_event(&state.host, &feed, stream_position.saturating_add(1), Observation::StreamClosing { reason: "daemon is draining".to_owned() }, &session, &decision).await {
                    yield Ok(event);
                }
                break;
            }
            let last_sequence = stream_position / 2;
            if stream_position != 0 && stream_position % 2 == 0 {
                match state.host.run(session.clone(), run.clone()).await {
                    Ok(status) => {
                        stream_position = stream_position.saturating_add(1);
                        let Ok(event) = observation_event(&state.host, &feed, stream_position, Observation::RunStatus(status), &session, &decision).await else {
                            break 'events;
                        };
                        yield Ok(event);
                    }
                    _ => break,
                }
            }
            let timeline_cursor = if last_sequence == 0 {
                None
            } else {
                let timeline_feed = format!("timeline:{run}");
                Some(stream_cursor_binding(&session, &timeline_feed))
                    .and_then(|binding| Cursor::new_bound(
                        &timeline_feed,
                        last_sequence,
                        binding,
                        &decision,
                        session.cursor_key(),
                    ).ok())
            };
            match state.host.timeline(
                session.clone(),
                run.clone(),
                timeline_cursor,
                STREAM_PAGE_ITEMS,
            ).await {
                Ok(page) if !page.items.is_empty() => {
                    for entry in page.items {
                        stream_position = entry.sequence.saturating_mul(2);
                        let Ok(event) = observation_event(&state.host, &feed, stream_position, Observation::Timeline(entry), &session, &decision).await else {
                            break 'events;
                        };
                        yield Ok(event);
                    }
                    match state.host.run(session.clone(), run.clone()).await {
                        Ok(status) => {
                            stream_position = stream_position.saturating_add(1);
                            let Ok(event) = observation_event(&state.host, &feed, stream_position, Observation::RunStatus(status), &session, &decision).await else {
                                break 'events;
                            };
                            yield Ok(event);
                        }
                        _ => break,
                    }
                }
                Ok(_) => tokio::time::sleep(Duration::from_millis(250)).await,
                Err(error) => {
                    let reason = if error.code == ErrorCode::Unauthorized { "authorization changed" } else { "timeline cursor must be resynchronized" };
                    if let Ok(event) = observation_event(&state.host, &feed, stream_position.saturating_add(1), Observation::ResyncRequired { reason: reason.to_owned() }, &session, &decision).await {
                        yield Ok(event);
                    }
                    break;
                }
            }
        }
    };
    Ok(Sse::new(output).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("heartbeat"),
    ))
}

pub(super) async fn capability_stream(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let (request_id, initial_session) = authenticate(&state, &headers)?;
    let bearer =
        bearer_header(&headers).ok_or_else(|| ApiError::unauthenticated(request_id.clone()))?;
    let feed = "capability-health".to_owned();
    let initial_decision = stream_authority(
        &state,
        initial_session.clone(),
        StreamAuthority::Capabilities,
        &request_id,
    )
    .await?;
    let initial_binding = stream_cursor_binding(&initial_session, &feed);
    let mut position = query
        .cursor
        .as_ref()
        .map(|cursor| {
            cursor.position_for_bound(&feed, &initial_binding, initial_session.cursor_key())
        })
        .transpose()
        .map_err(|error| protocol_error(error, request_id.clone()))?
        .unwrap_or(0);
    info!(
        feed,
        resume = query.cursor.is_some(),
        "capability stream subscription established"
    );
    let output = stream! {
        'events: loop {
            let Some(session) = state.host.authenticate_header(Some(&bearer)) else {
                if let Ok(event) = observation_event(&state.host, &feed, position.saturating_add(1), Observation::StreamClosing { reason: "authorization was revoked or rotated".to_owned() }, &initial_session, &initial_decision).await {
                    yield Ok(event);
                }
                break;
            };
            let decision = match stream_authority(
                &state,
                session.clone(),
                StreamAuthority::Capabilities,
                &request_id,
            ).await {
                Ok(decision) => decision,
                Err(_) => break,
            };
            let binding = stream_cursor_binding(&session, &feed);
            if state.host.health().draining {
                if let Ok(event) = observation_event(&state.host, &feed, position.saturating_add(1), Observation::StreamClosing { reason: "daemon is draining".to_owned() }, &session, &decision).await {
                    yield Ok(event);
                }
                break;
            }
            let capabilities = match state.host.capabilities(session.clone()).await {
                Ok(values) => values,
                _ => break,
            };
            let (resync, entries) = {
                let mut capability_feeds = state.capability_feeds.lock().await;
                let capability_feed = capability_feeds
                    .entry(binding.scope_digest.clone())
                    .or_insert_with(|| CapabilityFeed {
                        next_position: 1,
                        ..CapabilityFeed::default()
                    });
                record_capability_snapshot(capability_feed, &capabilities);
                let latest = capability_feed.next_position.saturating_sub(1);
                let oldest = capability_feed
                    .entries
                    .front()
                    .map_or(capability_feed.next_position, |(entry, _)| *entry);
                let resync = position > latest
                    || (position != 0 && position.saturating_add(1) < oldest);
                let entries = capability_feed
                    .entries
                    .iter()
                    .filter(|(entry, _)| *entry > position)
                    .cloned()
                    .collect::<Vec<_>>();
                (resync, entries)
            };
            if resync {
                if let Ok(event) = observation_event(&state.host, &feed, position.saturating_add(1), Observation::ResyncRequired { reason: "capability cursor is outside the retained health window".to_owned() }, &session, &decision).await {
                    yield Ok(event);
                }
                break;
            }
            for (entry_position, capability) in entries {
                position = entry_position;
                let Ok(event) = observation_event(&state.host, &feed, position, Observation::Capability(capability), &session, &decision).await else {
                    break 'events;
                };
                yield Ok(event);
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    };
    Ok(Sse::new(output).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("heartbeat"),
    ))
}

fn record_capability_snapshot(feed: &mut CapabilityFeed, capabilities: &[CapabilityRead]) {
    let Ok(bytes) = milkdrift_control_protocol::encode_json(&capabilities) else {
        return;
    };
    let digest = blake3::hash(&bytes).to_hex().to_string();
    if feed.last_snapshot_digest.as_deref() == Some(&digest) {
        return;
    }
    feed.last_snapshot_digest = Some(digest);
    for capability in capabilities {
        let position = feed.next_position;
        feed.next_position = feed.next_position.saturating_add(1);
        feed.entries.push_back((position, capability.clone()));
        while feed.entries.len() > CAPABILITY_FEED_ITEMS {
            feed.entries.pop_front();
        }
    }
}

pub(super) async fn health_stream(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let (request_id, initial_session) = authenticate(&state, &headers)?;
    let bearer =
        bearer_header(&headers).ok_or_else(|| ApiError::unauthenticated(request_id.clone()))?;
    let feed = "daemon-health".to_owned();
    let initial_decision = stream_authority(
        &state,
        initial_session.clone(),
        StreamAuthority::Health,
        &request_id,
    )
    .await?;
    let initial_binding = stream_cursor_binding(&initial_session, &feed);
    let mut position = query
        .cursor
        .as_ref()
        .map(|cursor| {
            cursor.position_for_bound(&feed, &initial_binding, initial_session.cursor_key())
        })
        .transpose()
        .map_err(|error| protocol_error(error, request_id.clone()))?
        .unwrap_or(0);
    info!(
        feed,
        resume = query.cursor.is_some(),
        "health stream subscription established"
    );
    let output = stream! {
        'events: loop {
            let Some(session) = state.host.authenticate_header(Some(&bearer)) else {
                if let Ok(event) = observation_event(&state.host, &feed, position.saturating_add(1), Observation::StreamClosing { reason: "authorization was revoked or rotated".to_owned() }, &initial_session, &initial_decision).await {
                    yield Ok(event);
                }
                break;
            };
            let decision = match stream_authority(
                &state,
                session.clone(),
                StreamAuthority::Health,
                &request_id,
            ).await {
                Ok(decision) => decision,
                Err(_) => break,
            };
            let (generation, health) = state.host.health_snapshot();
            if generation > position {
                position = generation;
                let Ok(event) = observation_event(&state.host, &feed, position, Observation::DaemonHealth(health.clone()), &session, &decision).await else {
                    break 'events;
                };
                yield Ok(event);
            }
            if health.draining {
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    };
    Ok(Sse::new(output).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("heartbeat"),
    ))
}

async fn stream_authority(
    state: &AppState,
    session: ActorSession,
    stream: StreamAuthority,
    request_id: &str,
) -> Result<String, ApiError> {
    let decision = state
        .host
        .authorize_stream(session, stream)
        .await
        .map_err(|error| owner_error(error, request_id.to_owned()))?;
    Ok(decision)
}

fn stream_cursor_binding(session: &ActorSession, exact_resource_and_filter: &str) -> CursorBinding {
    let claim = session.context.authority();
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"milkdrift.continuation-scope.v1\0");
    hasher.update(exact_resource_and_filter.as_bytes());
    hasher.update(format!("{:?}", session.grant.resources()).as_bytes());
    CursorBinding {
        actor: session.actor.as_str().to_owned(),
        grant_id: claim.grant().as_str().to_owned(),
        grant_revision: claim.grant_revision(),
        grant_digest: claim.grant_digest().as_str().to_owned(),
        scope_digest: format!("b3_{}", hasher.finalize()),
    }
}

async fn observation_event(
    host: &DaemonHost,
    feed: &str,
    position: u64,
    observation: Observation,
    session: &ActorSession,
    decision_digest: &str,
) -> Result<Event, ()> {
    let observed_at_ms = host.now().await.map_err(|error| {
        warn!(
            outcome = "closed",
            code = "observation_clock_unavailable",
            "observation stream closed because its timestamp boundary failed: {}",
            error.message
        );
    })?;
    let cursor = Cursor::new_bound(
        feed,
        position,
        stream_cursor_binding(session, feed),
        decision_digest,
        session.cursor_key(),
    )
    .map_err(|_| ())?;
    let envelope = ObservationEnvelope {
        protocol: ProtocolVersion::CURRENT,
        cursor: cursor.clone(),
        observed_at_ms,
        feed: feed.to_owned(),
        observation,
    };
    let data =
        String::from_utf8(milkdrift_control_protocol::encode_json(&envelope).map_err(|_| ())?)
            .map_err(|_| ())?;
    Ok(Event::default()
        .id(cursor.as_str())
        .event("observation")
        .data(data))
}
