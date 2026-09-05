use milkdrift_contracts::{
    JsonLimits, parse_json_without_duplicates, preflight_json_structure, validate_json_value,
};
use milkdrift_persistence::{
    EventCursor, EventPageQuery, MAX_PAGE_SIZE, PageSize, RunEventEnvelope, RunQueryStore,
    RunSequence, SnapshotLoad, SnapshotStore,
};
use milkdrift_workspace::RunId;
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::{RunProjection, RuntimeError};

pub(crate) const RUN_PROJECTION_SNAPSHOT_SCHEMA_V4: u32 = 4;
const PROJECTION_PAYLOAD_JSON_LIMITS: JsonLimits = JsonLimits {
    maximum_depth: 64,
    maximum_string_bytes: 1_048_576,
    maximum_key_bytes: 256,
    maximum_container_items: 16_384,
};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ProjectionSnapshotPayload {
    schema_version: u32,
    projection: RunProjection,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct ProjectionSnapshotPayloadRef<'a> {
    schema_version: u32,
    projection: &'a RunProjection,
}

fn decode_projection_snapshot_payload(bytes: &[u8]) -> Result<ProjectionSnapshotPayload, String> {
    preflight_json_structure(bytes, PROJECTION_PAYLOAD_JSON_LIMITS).map_err(|violation| {
        format!(
            "projection payload {} exceeds {:?} limit {}",
            violation.path(),
            violation.kind(),
            violation.maximum()
        )
    })?;
    let value = parse_json_without_duplicates(bytes).map_err(|error| error.to_string())?;
    validate_json_value(&value, PROJECTION_PAYLOAD_JSON_LIMITS).map_err(|violation| {
        format!(
            "projection payload {} exceeds {:?} limit {}",
            violation.path(),
            violation.kind(),
            violation.maximum()
        )
    })?;
    serde_json::from_value(value).map_err(|error| error.to_string())
}

/// Folds one authoritative history range against the head observed by the first read.
///
/// Concurrent appends after that read are intentionally ignored; callers receive a
/// consistent prefix instead of restarting from sequence one. Every page must remain
/// contiguous and may never regress behind the pinned head.
pub(crate) fn fold_history_from<S, T, F>(
    store: &S,
    run: &RunId,
    covered_through: RunSequence,
    mut accumulator: T,
    mut fold: F,
) -> Result<T, RuntimeError>
where
    S: RunQueryStore + ?Sized,
    F: FnMut(&mut T, &RunEventEnvelope) -> Result<(), RuntimeError>,
{
    let limit = PageSize::new(MAX_PAGE_SIZE)?;
    let mut cursor = if covered_through == RunSequence::ZERO {
        None
    } else {
        Some(EventCursor {
            run: run.clone(),
            next_sequence: covered_through.next()?,
        })
    };
    let mut pinned_head = None;
    let mut folded_through = covered_through;
    let mut expected = if covered_through == RunSequence::ZERO {
        RunSequence::FIRST
    } else {
        covered_through.next()?
    };

    loop {
        let page = store.events(&EventPageQuery::new(run.clone(), cursor.clone(), limit)?)?;
        let head = *pinned_head.get_or_insert(page.observed_head);
        if page.observed_head < head {
            return Err(RuntimeError::InvalidHistory(
                "journal head regressed during a pinned history fold".to_owned(),
            ));
        }
        if covered_through > head {
            return Err(RuntimeError::InvalidHistory(format!(
                "projection checkpoint covers {covered_through}, beyond observed journal head {head}"
            )));
        }
        if folded_through == head {
            return Ok(accumulator);
        }
        if page.events.is_empty() {
            return Err(RuntimeError::InvalidHistory(format!(
                "history page was empty before pinned head {head} was reached"
            )));
        }

        let mut physical_tail = None;
        for event in &page.events {
            if event.sequence() != expected {
                return Err(RuntimeError::InvalidHistory(format!(
                    "history page expected sequence {expected}, found {}",
                    event.sequence()
                )));
            }
            physical_tail = Some(event.sequence());
            expected = expected.next()?;
            if event.sequence() > head {
                break;
            }
            fold(&mut accumulator, event)?;
            folded_through = event.sequence();
            if folded_through == head {
                return Ok(accumulator);
            }
        }

        let next = page.next.ok_or_else(|| {
            RuntimeError::InvalidHistory(format!(
                "history pagination ended at {folded_through} before pinned head {head}"
            ))
        })?;
        let expected_next = physical_tail
            .ok_or_else(|| {
                RuntimeError::InvalidHistory(
                    "non-empty history page had no physical tail".to_owned(),
                )
            })?
            .next()?;
        if next.next_sequence != expected_next {
            return Err(RuntimeError::InvalidHistory(
                "event pagination cursor did not advance beyond the page tail".to_owned(),
            ));
        }
        cursor = Some(next);
    }
}

/// Folds complete authoritative history without retaining prior pages.
pub(crate) fn fold_complete_history<S, T, F>(
    store: &S,
    run: &RunId,
    accumulator: T,
    fold: F,
) -> Result<T, RuntimeError>
where
    S: RunQueryStore + ?Sized,
    F: FnMut(&mut T, &RunEventEnvelope) -> Result<(), RuntimeError>,
{
    fold_history_from(store, run, RunSequence::ZERO, accumulator, fold)
}

/// Projects complete authoritative history while retaining only projection state and
/// the current bounded event page.
pub(crate) fn project_complete_history<S: RunQueryStore + ?Sized>(
    store: &S,
    run: &RunId,
) -> Result<RunProjection, RuntimeError> {
    fold_complete_history(store, run, RunProjection::new(), |projection, event| {
        projection.apply_replayed(event)
    })
}

fn discard_optional_snapshot<S: SnapshotStore + ?Sized>(
    store: &S,
    run: &RunId,
    snapshot: &milkdrift_persistence::SnapshotId,
    reason: &str,
) {
    if let Err(error) = store.discard_snapshot(run, snapshot) {
        warn!(
            run = %run,
            snapshot = %snapshot,
            reason = %error,
            rejection = reason,
            "invalid optional projection snapshot could not be discarded"
        );
    }
}

/// Loads a verified runtime checkpoint and replays only the authoritative tail.
/// Invalid or unsupported optional snapshots are discarded best-effort and never repair
/// or block authoritative history replay.
pub(crate) fn project_from_latest_snapshot<S>(
    store: &S,
    run: &RunId,
) -> Result<RunProjection, RuntimeError>
where
    S: RunQueryStore + SnapshotStore + ?Sized,
{
    let snapshot = match store.latest_snapshot(run)? {
        SnapshotLoad::Absent => return project_complete_history(store, run),
        SnapshotLoad::Rejected { snapshot, reason } => {
            if let Some(snapshot) = snapshot {
                discard_optional_snapshot(store, run, &snapshot, reason.as_str());
            }
            warn!(run = %run, reason = %reason.as_str(), "rejected snapshot ignored");
            return project_complete_history(store, run);
        }
        SnapshotLoad::Verified(snapshot) => snapshot,
    };

    if snapshot.projection_payload_schema() != RUN_PROJECTION_SNAPSHOT_SCHEMA_V4 {
        discard_optional_snapshot(
            store,
            run,
            snapshot.snapshot(),
            "unsupported runtime projection schema",
        );
        return project_complete_history(store, run);
    }
    let payload = match decode_projection_snapshot_payload(snapshot.payload()) {
        Ok(payload) => payload,
        Err(error) => {
            warn!(run = %run, snapshot = %snapshot.snapshot(), reason = %error, "projection snapshot payload rejected");
            discard_optional_snapshot(
                store,
                run,
                snapshot.snapshot(),
                "projection snapshot payload is not valid JSON",
            );
            return project_complete_history(store, run);
        }
    };
    let canonical_payload = match serde_json::to_vec(&ProjectionSnapshotPayloadRef {
        schema_version: payload.schema_version,
        projection: &payload.projection,
    }) {
        Ok(payload) => payload,
        Err(error) => {
            warn!(run = %run, snapshot = %snapshot.snapshot(), reason = %error, "projection snapshot payload could not be canonicalized");
            discard_optional_snapshot(
                store,
                run,
                snapshot.snapshot(),
                "projection snapshot payload could not be canonicalized",
            );
            return project_complete_history(store, run);
        }
    };
    if canonical_payload.as_slice() != snapshot.payload() {
        discard_optional_snapshot(
            store,
            run,
            snapshot.snapshot(),
            "projection snapshot payload is not the exact canonical schema",
        );
        return project_complete_history(store, run);
    }
    if payload.schema_version != RUN_PROJECTION_SNAPSHOT_SCHEMA_V4
        || payload.projection.run_id() != Some(run)
        || payload.projection.sequence() != snapshot.covered_sequence()
        || payload.projection.history_compacted_through() != snapshot.covered_sequence()
    {
        discard_optional_snapshot(
            store,
            run,
            snapshot.snapshot(),
            "projection snapshot identity or compaction boundary is inconsistent",
        );
        return project_complete_history(store, run);
    }
    if let Err(error) = payload.projection.validate_compacted_state() {
        warn!(run = %run, snapshot = %snapshot.snapshot(), reason = %error, "projection snapshot liveness references rejected");
        discard_optional_snapshot(
            store,
            run,
            snapshot.snapshot(),
            "projection snapshot contains invalid active references",
        );
        return project_complete_history(store, run);
    }

    fold_history_from(
        store,
        run,
        snapshot.covered_sequence(),
        payload.projection,
        |projection, event| projection.apply_replayed(event),
    )
}

pub(crate) fn encode_projection_snapshot(
    projection: &RunProjection,
) -> Result<Vec<u8>, RuntimeError> {
    if projection.history_compacted_through() != projection.sequence() {
        return Err(RuntimeError::InvalidHistory(
            "projection snapshot requires a live-compacted projection".to_owned(),
        ));
    }
    projection.validate_compacted_state()?;
    Ok(serde_json::to_vec(&ProjectionSnapshotPayloadRef {
        schema_version: RUN_PROJECTION_SNAPSHOT_SCHEMA_V4,
        projection,
    })?)
}

/// Materializes at most `maximum` events for diagnostics and bounded tests.
///
/// Runtime decision paths must use a projection fold. This helper never grows beyond
/// the caller's validated [`PageSize`] bound and returns an explicit bounds error.
pub(crate) fn load_bounded_history<S: RunQueryStore + ?Sized>(
    store: &S,
    run: &RunId,
    maximum: PageSize,
) -> Result<Vec<RunEventEnvelope>, RuntimeError> {
    fold_complete_history(
        store,
        run,
        Vec::with_capacity(maximum.get() as usize),
        |events, event| {
            if events.len() == maximum.get() as usize {
                return Err(milkdrift_persistence::PersistenceError::Bounds {
                    location: "runtime.bounded_history",
                    reason: format!(
                        "history exceeds the explicit diagnostic bound {}",
                        maximum.get()
                    ),
                }
                .into());
            }
            events.push(event.clone());
            Ok(())
        },
    )
}

#[cfg(test)]
mod tests;
