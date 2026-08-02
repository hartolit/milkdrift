//! Hosted public-E0 Criterion boundaries over the deterministic fixture.
//!
//! `e0_hosted_checked_prefill/4_tokens` accumulates only checked-prefill
//! submission through the matching completion event. The incremental-decode
//! target accumulates the equivalent one-token decode boundary after an untimed
//! two-token prefill. Fixture verification, worker/model lifecycle, request
//! setup, command construction, validation, completion, unload, shutdown, and
//! join remain outside accumulated time. Reusable vocabulary-sized logits are
//! moved out and restored without per-iteration allocation.
//!
//! These targets provide comparative synthetic integration evidence. They do
//! not measure E1/product latency, full-generation throughput, RSS, allocation
//! counts, native-resource attribution, or production steady state.

#![forbid(unsafe_code)]

use std::process;
use std::time::Duration;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use runtime_benchmarks::e0::{
    CRITERION_VOCABULARY_SIZE, HostedE0Harness, criterion_checked_prefill_iteration,
    criterion_harness, criterion_incremental_decode_iteration,
};
use runtime_benchmarks::{BenchmarkError, BenchmarkResult};

const PREFILL_THROUGHPUT: u64 = 4;
const DECODE_THROUGHPUT: u64 = 1;
const BENCHMARK_FAILURE_EXIT_CODE: i32 = 2;

type Iteration = fn(&mut HostedE0Harness, &mut Vec<f32>) -> BenchmarkResult<Duration>;

fn benchmark_runtime_components(criterion: &mut Criterion) {
    benchmark_target(
        criterion,
        "e0_hosted_checked_prefill",
        "4_tokens",
        PREFILL_THROUGHPUT,
        criterion_checked_prefill_iteration,
    );
    benchmark_target(
        criterion,
        "e0_hosted_incremental_decode",
        "1_token_after_2_token_prefill",
        DECODE_THROUGHPUT,
        criterion_incremental_decode_iteration,
    );
}

fn benchmark_target(
    criterion: &mut Criterion,
    group_name: &str,
    target_name: &str,
    throughput: u64,
    iteration: Iteration,
) {
    let mut group = criterion.benchmark_group(group_name);
    group.throughput(Throughput::Elements(throughput));
    group.bench_function(target_name, |benchmark| {
        let mut harness = result_or_exit(criterion_harness(), "Criterion harness setup");
        let mut logits = vec![0.0_f32; CRITERION_VOCABULARY_SIZE];
        let mut failure: Option<BenchmarkError> = None;
        benchmark.iter_custom(|iterations| {
            if failure.is_some() {
                return Duration::from_nanos(1);
            }
            let mut measured = Duration::ZERO;
            for _ in 0..iterations {
                match iteration(&mut harness, &mut logits) {
                    Ok(elapsed) => measured = measured.saturating_add(elapsed),
                    Err(error) => {
                        failure = Some(error);
                        break;
                    }
                }
            }
            if failure.is_some() && measured.is_zero() {
                Duration::from_nanos(1)
            } else {
                measured
            }
        });
        let primary = match failure {
            Some(error) => Err(error),
            None => Ok(()),
        };
        result_or_exit(
            harness.finish(primary).map(|_| ()),
            "Criterion harness cleanup",
        );
    });
    group.finish();
}

fn result_or_exit<T>(result: BenchmarkResult<T>, operation: &str) -> T {
    match result {
        Ok(value) => value,
        Err(error) => {
            eprintln!("{operation} failed: {error}");
            process::exit(BENCHMARK_FAILURE_EXIT_CODE);
        }
    }
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .warm_up_time(Duration::from_millis(500))
        .measurement_time(Duration::from_secs(2))
        .sample_size(10);
    targets = benchmark_runtime_components
}
criterion_main!(benches);
