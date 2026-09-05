//! Tests for shared orchestration helpers.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use milkdrift_persistence::NodeExecutionId;
use milkdrift_workspace::RunId;

use super::{bounded_projection_sweep_set, stable_idempotency_key};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

#[test]
fn finite_projection_sweep_clears_its_cursor_at_the_current_end() -> TestResult {
    let run = RunId::new("run-finite-projection-sweep")?;
    let values = BTreeSet::from([1_u8, 2, 3, 4, 5]);
    let cursors = Mutex::new(BTreeMap::new());

    let mut remaining = 2;
    assert_eq!(
        bounded_projection_sweep_set(&run, &values, &cursors, &mut remaining, "test finite sweep",)?,
        vec![1, 2]
    );
    assert_eq!(remaining, 0);
    assert_eq!(
        cursors
            .lock()
            .map_err(|_| "finite sweep cursor lock poisoned")?
            .get(&run)
            .copied(),
        Some(2)
    );

    let mut remaining = 2;
    assert_eq!(
        bounded_projection_sweep_set(&run, &values, &cursors, &mut remaining, "test finite sweep",)?,
        vec![3, 4]
    );
    assert_eq!(
        cursors
            .lock()
            .map_err(|_| "finite sweep cursor lock poisoned")?
            .get(&run)
            .copied(),
        Some(4)
    );

    let mut remaining = 2;
    assert_eq!(
        bounded_projection_sweep_set(&run, &values, &cursors, &mut remaining, "test finite sweep",)?,
        vec![5]
    );
    assert!(
        !cursors
            .lock()
            .map_err(|_| "finite sweep cursor lock poisoned")?
            .contains_key(&run)
    );
    Ok(())
}

#[test]
fn finite_projection_sweep_removes_a_cursor_for_an_empty_frontier() -> TestResult {
    let run = RunId::new("run-empty-projection-sweep")?;
    let cursors = Mutex::new(BTreeMap::from([(run.clone(), 9_u8)]));
    let mut remaining = 1;
    assert!(
        bounded_projection_sweep_set(
            &run,
            &BTreeSet::new(),
            &cursors,
            &mut remaining,
            "test empty sweep",
        )?
        .is_empty()
    );
    assert!(
        !cursors
            .lock()
            .map_err(|_| "empty sweep cursor lock poisoned")?
            .contains_key(&run)
    );
    Ok(())
}

#[test]
fn maximum_length_durable_identities_have_a_fixed_length_idempotency_key() -> TestResult {
    let run = RunId::new("r".repeat(128))?;
    let execution = NodeExecutionId::new("e".repeat(192))?;
    let other = NodeExecutionId::new("f".repeat(192))?;
    let key = stable_idempotency_key(&run, &execution)?;
    assert!(key.as_str().len() <= 192);
    assert_eq!(key, stable_idempotency_key(&run, &execution)?);
    assert_ne!(key, stable_idempotency_key(&run, &other)?);
    Ok(())
}
