//! Normal-runner orchestration for deterministic synthetic E0 scenarios.

use std::time::Duration;

use super::generation::{
    BackpressureObservation, CancellationObservation, FirstTokenMeasurement, measure_backpressure,
    measure_cancellation, measure_first_token_and_proxy,
};
use super::harness::{HostedE0Harness, ShutdownDurations};
use super::lifecycle::{
    LoadedFixture, checked_prefill_iteration, load_fixture, unload_loaded_model,
};
use super::observation::{
    capture_snapshot, validate_empty_snapshot, validate_loaded_idle_snapshot,
};
use crate::error::{BenchmarkError, BenchmarkResult};
use crate::fixture::{VOCABULARY_SIZE, VerifiedFixture};
use crate::report::{
    BackpressureMeasurement, CancellationMeasurement, CycleSet, ShutdownMeasurement,
    SnapshotCheckpoint, SyntheticCycle, SyntheticLoadEvidence, checked_duration_ns,
};

pub(crate) fn run_cycles(
    fixture: &VerifiedFixture,
    warmup_cycles: u32,
    sample_cycles: u32,
) -> BenchmarkResult<CycleSet<SyntheticCycle>> {
    let mut warmups = Vec::new();
    let mut samples = Vec::new();
    warmups
        .try_reserve_exact(usize_from_u32(warmup_cycles)?)
        .map_err(|error| {
            BenchmarkError::new(format!("warmup record allocation failed: {error}"))
        })?;
    samples
        .try_reserve_exact(usize_from_u32(sample_cycles)?)
        .map_err(|error| {
            BenchmarkError::new(format!("sample record allocation failed: {error}"))
        })?;

    for _ in 0..warmup_cycles {
        warmups.push(run_cycle(fixture)?);
    }
    for _ in 0..sample_cycles {
        samples.push(run_cycle(fixture)?);
    }
    Ok(CycleSet { warmups, samples })
}

fn run_cycle(fixture: &VerifiedFixture) -> BenchmarkResult<SyntheticCycle> {
    let (mut harness, start_duration) = HostedE0Harness::start(1, 128)?;
    let body = run_cycle_body(&mut harness, fixture);
    let (body, shutdown) = harness.finish(body)?;
    to_report(start_duration, body, shutdown)
}

struct CycleBody {
    model_load: Duration,
    load_evidence: SyntheticLoadEvidence,
    checked_prefill: Duration,
    first_token: Duration,
    post_first_token_proxy: Duration,
    backpressure_controlled_hold: Duration,
    backpressure_recovery_to_next_token: Duration,
    cancellation: CancellationObservation,
    model_unload: Duration,
    snapshots: Vec<SnapshotCheckpoint>,
}

fn run_cycle_body(
    harness: &mut HostedE0Harness,
    fixture: &VerifiedFixture,
) -> BenchmarkResult<CycleBody> {
    let mut snapshots = Vec::new();

    let before_load = capture_snapshot(harness, "before-load")?;
    validate_empty_snapshot(&before_load, "before load")?;
    snapshots.push(before_load.record);

    let LoadedFixture {
        receipt: loaded,
        elapsed: model_load,
        evidence: load_evidence,
    } = load_fixture(harness, fixture)?;
    let handle = loaded.handle;
    let loaded_footprint = loaded.reserved_footprint;

    let after_load = capture_snapshot(harness, "after-load")?;
    validate_loaded_idle_snapshot(&after_load, handle, loaded_footprint, "after load")?;
    snapshots.push(after_load.record);

    let mut logits = vec![0.0_f32; usize_from_u32(VOCABULARY_SIZE)?];
    let checked_prefill = checked_prefill_iteration(harness, &mut logits)?;
    let after_prefill = capture_snapshot(harness, "after-checked-prefill-release")?;
    validate_loaded_idle_snapshot(
        &after_prefill,
        handle,
        loaded_footprint,
        "after checked prefill release",
    )?;
    snapshots.push(after_prefill.record);

    let FirstTokenMeasurement {
        first_token,
        post_first_proxy,
    } = measure_first_token_and_proxy(harness, handle)?;
    let after_first_release = capture_snapshot(harness, "after-first-token-proxy-release")?;
    validate_loaded_idle_snapshot(
        &after_first_release,
        handle,
        loaded_footprint,
        "after first-token proxy release",
    )?;
    snapshots.push(after_first_release.record);

    let BackpressureObservation {
        controlled_hold: backpressure_controlled_hold,
        recovery_to_next_token: backpressure_recovery_to_next_token,
        during_backpressure,
    } = measure_backpressure(harness, handle, loaded_footprint)?;
    snapshots.push(during_backpressure.record);
    let after_backpressure_release = capture_snapshot(harness, "after-backpressure-release")?;
    validate_loaded_idle_snapshot(
        &after_backpressure_release,
        handle,
        loaded_footprint,
        "after backpressure release",
    )?;
    snapshots.push(after_backpressure_release.record);

    let cancellation = measure_cancellation(harness, handle)?;
    let after_cancellation_release = capture_snapshot(harness, "after-cancellation-release")?;
    validate_loaded_idle_snapshot(
        &after_cancellation_release,
        handle,
        loaded_footprint,
        "after cancellation release",
    )?;
    snapshots.push(after_cancellation_release.record);

    let model_unload = unload_loaded_model(harness)?;
    let after_unload = capture_snapshot(harness, "after-unload")?;
    validate_empty_snapshot(&after_unload, "after unload")?;
    snapshots.push(after_unload.record);

    Ok(CycleBody {
        model_load,
        load_evidence,
        checked_prefill,
        first_token,
        post_first_token_proxy: post_first_proxy,
        backpressure_controlled_hold,
        backpressure_recovery_to_next_token,
        cancellation,
        model_unload,
        snapshots,
    })
}

fn to_report(
    start_duration: Duration,
    body: CycleBody,
    shutdown: ShutdownDurations,
) -> BenchmarkResult<SyntheticCycle> {
    Ok(SyntheticCycle {
        e0_start_ns: checked_duration_ns(start_duration, "synthetic E0 startup")?,
        model_load_ns: checked_duration_ns(body.model_load, "synthetic E0 model load")?,
        load_evidence: body.load_evidence,
        checked_prefill_ns: checked_duration_ns(
            body.checked_prefill,
            "synthetic E0 checked prefill",
        )?,
        first_token_ns: checked_duration_ns(body.first_token, "synthetic E0 first token")?,
        post_first_token_proxy_ns: checked_duration_ns(
            body.post_first_token_proxy,
            "synthetic E0 post-first-token proxy",
        )?,
        backpressure: BackpressureMeasurement {
            controlled_hold_ns: checked_duration_ns(
                body.backpressure_controlled_hold,
                "synthetic E0 controlled backpressure hold",
            )?,
            recovery_to_next_token_ns: checked_duration_ns(
                body.backpressure_recovery_to_next_token,
                "synthetic E0 backpressure recovery",
            )?,
        },
        cancellation: CancellationMeasurement {
            generated_tokens: body.cancellation.generated_tokens,
            acknowledgement_ns: checked_duration_ns(
                body.cancellation.acknowledgement,
                "synthetic E0 cancellation acknowledgement",
            )?,
            terminal_ns: checked_duration_ns(
                body.cancellation.terminal,
                "synthetic E0 cancellation terminal",
            )?,
            released_ns: checked_duration_ns(
                body.cancellation.released,
                "synthetic E0 cancellation release",
            )?,
        },
        model_unload_ns: checked_duration_ns(body.model_unload, "synthetic E0 model unload")?,
        shutdown: ShutdownMeasurement {
            event_ns: checked_duration_ns(shutdown.event, "synthetic E0 shutdown event")?,
            join_ns: checked_duration_ns(shutdown.join, "synthetic E0 shutdown join")?,
            total_ns: checked_duration_ns(shutdown.total, "synthetic E0 shutdown total")?,
        },
        snapshots: body.snapshots,
    })
}

fn usize_from_u32(value: u32) -> BenchmarkResult<usize> {
    usize::try_from(value)
        .map_err(|_| BenchmarkError::new("u32-to-usize capacity conversion failed"))
}
