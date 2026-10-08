//! Poll authorized read models and frame them as resumable SSE observations.
//!
//! Each poll rechecks credentials and authority. Timeline and run-stream cursors use distinct
//! feeds; capability observations retain only a bounded window per exact authority. Closing a
//! connection ends disclosure, not the underlying run or invocation.
use super::{
    ApiError, AppState, CAPABILITY_FEED_ITEMS, ListQuery, STREAM_PAGE_ITEMS, authenticate,
    bearer_header, owner_error, protocol_error,
};
use crate::{auth::ActorSession, host::StreamAuthority};
use async_stream::stream;
use axum::{
    extract::Path, extract::Query, extract::State, http::HeaderMap, response::Sse,
    response::sse::Event, response::sse::KeepAlive,
};
use futures_util::Stream;
use milkdrift_control_protocol::{
    CapabilityRead, Cursor, CursorBinding, ErrorCode, MAX_DOCUMENT_BYTES, Observation,
    ObservationEnvelope, ProtocolVersion,
};
use std::{collections::VecDeque, convert::Infallible, time::Duration};
use tracing::{info, warn};

#[cfg(test)]
mod tests;

#[derive(Default)]
struct CapabilityFeed {
    last_snapshot_digest: Option<String>,
    next_position: u64,
    entries: VecDeque<(u64, Vec<CapabilityRead>, usize)>,
    retained_bytes: usize,
}

impl CapabilityFeed {
    fn after(&self, position: u64) -> Option<Vec<(u64, Vec<CapabilityRead>)>> {
        let latest = self.next_position.saturating_sub(1);
        let oldest = self
            .entries
            .front()
            .map_or(self.next_position, |(entry, _, _)| *entry);
        if position > latest || (position != 0 && position.saturating_add(1) < oldest) {
            return None;
        }
        // A fresh reader needs only the authoritative present. Reconnects replay whole snapshots
        // after their acknowledged position; no client-side merge or inferred removal is needed.
        Some(
            self.entries
                .iter()
                .filter(|(entry, _, _)| {
                    if position == 0 {
                        *entry == latest
                    } else {
                        *entry > position
                    }
                })
                .map(|(position, values, _)| (*position, values.clone()))
                .collect(),
        )
    }
}

/// Grants are immutable for one daemon lifetime. Retain at most one window per configured
/// authority, even across credential rotations; never evict and reuse its sequence numbers.
#[derive(Default)]
pub(super) struct CapabilityFeeds(Vec<(CursorBinding, CapabilityFeed)>);

impl CapabilityFeeds {
    fn get(&mut self, binding: CursorBinding) -> Option<&mut CapabilityFeed> {
        let index = match self.0.iter().position(|(key, _)| key == &binding) {
            Some(index) => index,
            None => {
                if self.0.len() >= crate::config::MAX_ACTOR_BINDINGS {
                    return None;
                }
                self.0.push((
                    binding,
                    CapabilityFeed {
                        next_position: 1,
                        ..CapabilityFeed::default()
                    },
                ));
                self.0.len() - 1
            }
        };
        self.0.get_mut(index).map(|(_, feed)| feed)
    }
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
    let initial_binding = initial_session.cursor_binding(&feed);
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
        let mut establish = true;
        'events: loop {
            let Some(current_session) = state.host.authenticate_header(Some(&bearer))
                .filter(|session| session.cursor_binding(&feed) == initial_binding) else {
                if let Ok(event) = observation_event(&state, &feed, stream_position.saturating_add(1), Observation::StreamClosing { reason: "authorization was revoked or rotated".to_owned() }, &initial_session, &initial_decision).await {
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
                if let Ok(event) = observation_event(&state, &feed, stream_position.saturating_add(1), Observation::StreamClosing { reason: "daemon is draining".to_owned() }, &session, &decision).await {
                    yield Ok(event);
                }
                break;
            }
            if establish {
                establish = false;
                let status = match state.host.run(session.clone(), run.clone()).await {
                    Ok(status) => status,
                    Err(_) => break,
                };
                if stream_position / 2 > status.sequence {
                    if let Ok(event) = observation_event(&state, &feed, stream_position, Observation::ResyncRequired { reason: "run cursor is ahead of the retained run head".to_owned() }, &session, &decision).await {
                        yield Ok(event);
                    }
                    break;
                }
                if stream_position == 0 {
                    stream_position = status.sequence.saturating_mul(2).saturating_add(1);
                    let Ok(event) = observation_event(&state, &feed, stream_position, Observation::RunStatus(status), &session, &decision).await else { break; };
                    yield Ok(event);
                }
            }
            let last_sequence = stream_position / 2;
            if stream_position != 0 && stream_position % 2 == 0 {
                match state.host.run(session.clone(), run.clone()).await {
                    Ok(status) => {
                        stream_position = stream_position.saturating_add(1);
                        let Ok(event) = observation_event(&state, &feed, stream_position, Observation::RunStatus(status), &session, &decision).await else {
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
                Some(session.cursor_binding(&timeline_feed))
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
                        let Ok(event) = observation_event(&state, &feed, stream_position, Observation::Timeline(entry), &session, &decision).await else {
                            break 'events;
                        };
                        yield Ok(event);
                    }
                    match state.host.run(session.clone(), run.clone()).await {
                        Ok(status) => {
                            stream_position = stream_position.saturating_add(1);
                            let Ok(event) = observation_event(&state, &feed, stream_position, Observation::RunStatus(status), &session, &decision).await else {
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
                    if let Ok(event) = observation_event(&state, &feed, stream_position.saturating_add(1), Observation::ResyncRequired { reason: reason.to_owned() }, &session, &decision).await {
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
    let initial_binding = stream_binding(&state, &initial_session, &feed);
    let resume = query
        .cursor
        .as_ref()
        .map(|cursor| {
            cursor.position_for_bound(&feed, &initial_binding, initial_session.cursor_key())
        })
        .transpose();
    let resync = resume.is_err();
    let mut position = resume.ok().flatten().unwrap_or(0);
    info!(
        feed,
        resume = query.cursor.is_some(),
        "capability stream subscription established"
    );
    let output = stream! {
        if resync {
            if let Ok(event) = observation_event(&state, &feed, 0, Observation::ResyncRequired { reason: "process-local cursor does not belong to this daemon and authority; subscribe without a cursor".to_owned() }, &initial_session, &initial_decision).await {
                yield Ok(event);
            }
            return;
        }
        'events: loop {
            let Some(session) = state.host.authenticate_header(Some(&bearer))
                .filter(|session| stream_binding(&state, session, &feed) == initial_binding) else {
                if let Ok(event) = observation_event(&state, &feed, position.saturating_add(1), Observation::StreamClosing { reason: "authorization was revoked or rotated".to_owned() }, &initial_session, &initial_decision).await {
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
            let binding = stream_binding(&state, &session, &feed);
            if state.host.health().draining {
                if let Ok(event) = observation_event(&state, &feed, position.saturating_add(1), Observation::StreamClosing { reason: "daemon is draining".to_owned() }, &session, &decision).await {
                    yield Ok(event);
                }
                break;
            }
            let capabilities = match state.host.capabilities(session.clone()).await {
                Ok(values) => values,
                _ => break,
            };
            let entries = {
                let mut capability_feeds = state.capability_feeds.lock().await;
                let Some(capability_feed) = capability_feeds.get(binding) else { break; };
                record_capability_snapshot(capability_feed, &capabilities)
                    .ok().and_then(|()| capability_feed.after(position))
            };
            let Some(entries) = entries else {
                if let Ok(event) = observation_event(&state, &feed, position.saturating_add(1), Observation::ResyncRequired { reason: "capability snapshot or cursor is outside the retained stream bounds".to_owned() }, &session, &decision).await {
                    yield Ok(event);
                }
                break;
            };
            for (entry_position, capabilities) in entries {
                position = entry_position;
                let Ok(event) = observation_event(&state, &feed, position, Observation::CapabilitySnapshot(capabilities), &session, &decision).await else {
                    if let Ok(event) = observation_event(&state, &feed, position, Observation::ResyncRequired { reason: "capability snapshot exceeds the observation envelope bounds".to_owned() }, &session, &decision).await {
                        yield Ok(event);
                    }
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

fn record_capability_snapshot(
    feed: &mut CapabilityFeed,
    capabilities: &[CapabilityRead],
) -> Result<(), ()> {
    let bytes = milkdrift_control_protocol::encode_json(&capabilities).map_err(|_| ())?;
    let digest = blake3::hash(&bytes).to_hex().to_string();
    if feed.last_snapshot_digest.as_deref() == Some(&digest) {
        return Ok(());
    }
    let position = feed.next_position;
    let next_position = position.checked_add(1).ok_or(())?;
    feed.last_snapshot_digest = Some(digest);
    feed.next_position = next_position;
    feed.retained_bytes += bytes.len();
    feed.entries
        .push_back((position, capabilities.to_vec(), bytes.len()));
    while feed.entries.len() > CAPABILITY_FEED_ITEMS || feed.retained_bytes > MAX_DOCUMENT_BYTES {
        if let Some((_, _, bytes)) = feed.entries.pop_front() {
            feed.retained_bytes -= bytes;
        }
    }
    Ok(())
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
    let initial_binding = stream_binding(&state, &initial_session, &feed);
    let resume = query
        .cursor
        .as_ref()
        .map(|cursor| {
            cursor.position_for_bound(&feed, &initial_binding, initial_session.cursor_key())
        })
        .transpose();
    let resync = resume.is_err();
    let mut position = resume.ok().flatten().unwrap_or(0);
    info!(
        feed,
        resume = query.cursor.is_some(),
        "health stream subscription established"
    );
    let output = stream! {
        if resync {
            if let Ok(event) = observation_event(&state, &feed, 0, Observation::ResyncRequired { reason: "process-local cursor does not belong to this daemon and authority; subscribe without a cursor".to_owned() }, &initial_session, &initial_decision).await {
                yield Ok(event);
            }
            return;
        }
        'events: loop {
            let Some(session) = state.host.authenticate_header(Some(&bearer))
                .filter(|session| stream_binding(&state, session, &feed) == initial_binding) else {
                if let Ok(event) = observation_event(&state, &feed, position.saturating_add(1), Observation::StreamClosing { reason: "authorization was revoked or rotated".to_owned() }, &initial_session, &initial_decision).await {
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
                let Ok(event) = observation_event(&state, &feed, position, Observation::DaemonHealth(health.clone()), &session, &decision).await else {
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

async fn observation_event(
    state: &AppState,
    feed: &str,
    position: u64,
    observation: Observation,
    session: &ActorSession,
    decision_digest: &str,
) -> Result<Event, ()> {
    let observed_at_ms = state.host.now().await.map_err(|error| {
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
        stream_binding(state, session, feed),
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

fn stream_binding(state: &AppState, session: &ActorSession, feed: &str) -> CursorBinding {
    if matches!(feed, "daemon-health" | "capability-health") {
        // A new incarnation changes the MAC-bound scope even if counters restart at the same
        // value. Durable run and timeline feeds deliberately retain their ordinary binding.
        session.cursor_binding(&format!(
            "{feed}:{}",
            blake3::Hash::from(state.stream_incarnation)
        ))
    } else {
        session.cursor_binding(feed)
    }
}
