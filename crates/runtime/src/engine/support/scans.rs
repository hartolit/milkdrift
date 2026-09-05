//! Bounded round-robin and finite projection scans.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Bound::{Excluded, Unbounded};
use std::sync::Mutex;

use milkdrift_workspace::RunId;

use crate::RuntimeError;

pub(in crate::engine) fn bounded_projection_set<K: Clone + Ord>(
    run: &RunId,
    values: &BTreeSet<K>,
    cursor: &Mutex<BTreeMap<RunId, K>>,
    remaining: &mut usize,
    label: &'static str,
) -> Result<Vec<K>, RuntimeError> {
    let limit = (*remaining).min(values.len());
    if limit == 0 {
        return Ok(Vec::new());
    }
    let mut cursors = cursor.lock().map_err(|_error| {
        RuntimeError::Scheduling(format!("{label} coordination lock is poisoned"))
    })?;
    let previous = cursors.get(run).cloned();
    let mut selected = Vec::with_capacity(limit);
    if let Some(previous) = previous {
        selected.extend(
            values
                .range((Excluded(previous.clone()), Unbounded))
                .take(limit)
                .cloned(),
        );
        if selected.len() < limit {
            selected.extend(
                values
                    .range(..=previous)
                    .take(limit.saturating_sub(selected.len()))
                    .cloned(),
            );
        }
    } else {
        selected.extend(values.iter().take(limit).cloned());
    }
    if let Some(last) = selected.last() {
        cursors.insert(run.clone(), last.clone());
    }
    *remaining = remaining.saturating_sub(selected.len());
    Ok(selected)
}

/// Selects one finite, non-wrapping page from a per-run ordered set.
///
/// Unlike the round-robin scan helpers used by continuously driven scheduler
/// maintenance, a recovery sweep must expose when it reached the current end of
/// the set. The cursor is removed at that boundary so startup recovery can prove
/// that every currently visible attempt was examined and terminate.
pub(in crate::engine) fn bounded_projection_sweep_set<K: Clone + Ord>(
    run: &RunId,
    values: &BTreeSet<K>,
    cursor: &Mutex<BTreeMap<RunId, K>>,
    remaining: &mut usize,
    label: &'static str,
) -> Result<Vec<K>, RuntimeError> {
    let mut cursors = cursor.lock().map_err(|_error| {
        RuntimeError::Scheduling(format!("{label} coordination lock is poisoned"))
    })?;
    if values.is_empty() {
        cursors.remove(run);
        return Ok(Vec::new());
    }
    let limit = (*remaining).min(values.len());
    if limit == 0 {
        return Ok(Vec::new());
    }

    let previous = cursors.get(run).cloned();
    let mut selected: Vec<K> = match previous {
        Some(previous) => values
            .range((Excluded(previous), Unbounded))
            .take(limit)
            .cloned()
            .collect(),
        None => values.iter().take(limit).cloned().collect(),
    };
    // A cursor may become stale when recovery changed the active frontier. Start
    // a new sweep immediately instead of retaining an unreachable resume point.
    if selected.is_empty() && cursors.contains_key(run) {
        cursors.remove(run);
        selected.extend(values.iter().take(limit).cloned());
    }

    let reached_end = selected
        .last()
        .zip(values.last())
        .is_some_and(|(selected, final_value)| selected == final_value);
    if reached_end {
        cursors.remove(run);
    } else if let Some(last) = selected.last() {
        cursors.insert(run.clone(), last.clone());
    }
    *remaining = remaining.saturating_sub(selected.len());
    Ok(selected)
}

pub(in crate::engine) fn bounded_projection_map_keys<K: Clone + Ord, V>(
    run: &RunId,
    values: &BTreeMap<K, V>,
    cursor: &Mutex<BTreeMap<RunId, K>>,
    remaining: &mut usize,
    label: &'static str,
) -> Result<Vec<K>, RuntimeError> {
    let limit = (*remaining).min(values.len());
    if limit == 0 {
        return Ok(Vec::new());
    }
    let mut cursors = cursor.lock().map_err(|_error| {
        RuntimeError::Scheduling(format!("{label} coordination lock is poisoned"))
    })?;
    let previous = cursors.get(run).cloned();
    let mut selected = Vec::with_capacity(limit);
    if let Some(previous) = previous {
        selected.extend(
            values
                .range((Excluded(previous.clone()), Unbounded))
                .take(limit)
                .map(|(key, _)| key.clone()),
        );
        if selected.len() < limit {
            selected.extend(
                values
                    .range(..=previous)
                    .take(limit.saturating_sub(selected.len()))
                    .map(|(key, _)| key.clone()),
            );
        }
    } else {
        selected.extend(values.keys().take(limit).cloned());
    }
    if let Some(last) = selected.last() {
        cursors.insert(run.clone(), last.clone());
    }
    *remaining = remaining.saturating_sub(selected.len());
    Ok(selected)
}
