//! Repeatable Divan benchmarks for product-critical bounded paths.

use divan::black_box;
use milkdrift_evidence::{
    EvidenceResult, ScenarioMeasurement, artifact_range_read, daemon_owner_round_trip,
    explicit_input_materialization, explicit_input_selection, local_process_stream_drain,
    model_stream_parsers, peer_observation_paths, workflow_commit_batch, workflow_commit_one,
    workflow_read_page, workflow_receipt_paths, workflow_snapshot_reopen,
};

fn main() {
    divan::main();
}

fn measured(result: EvidenceResult<ScenarioMeasurement>) -> ScenarioMeasurement {
    match result {
        Ok(measurement) => measurement,
        Err(error) => {
            #[expect(
                clippy::panic,
                reason = "A failed benchmark fixture cannot yield a measurement; unwinding runs its owners' cleanup instead of exiting without destructors."
            )]
            {
                panic!("benchmark fixture failed: {error}");
            }
        }
    }
}

#[divan::bench(sample_count = 10, sample_size = 1)]
fn persistence_workflow_commit_one() {
    black_box(measured(workflow_commit_one()));
}

#[divan::bench(sample_count = 10, sample_size = 1)]
fn persistence_workflow_commit_batch() {
    black_box(measured(workflow_commit_batch()));
}

#[divan::bench(sample_count = 10, sample_size = 1)]
fn runtime_workflow_read_page() {
    black_box(measured(workflow_read_page()));
}

#[divan::bench(sample_count = 10, sample_size = 1)]
fn runtime_workflow_snapshot_reopen() {
    black_box(measured(workflow_snapshot_reopen()));
}

#[divan::bench(sample_count = 10, sample_size = 1)]
fn persistence_application_receipts() {
    black_box(measured(workflow_receipt_paths()));
}

#[divan::bench(sample_count = 10, sample_size = 1)]
fn persistence_peer_observations() {
    black_box(measured(peer_observation_paths()));
}

#[divan::bench(sample_count = 10, sample_size = 1)]
fn context_metadata_selection() {
    black_box(measured(explicit_input_selection()));
}

#[divan::bench(sample_count = 10, sample_size = 1)]
fn context_selected_materialization() {
    black_box(measured(explicit_input_materialization()));
}

#[divan::bench(sample_count = 10, sample_size = 1)]
fn adapters_artifact_range_read() {
    black_box(measured(artifact_range_read()));
}

#[divan::bench(sample_count = 10, sample_size = 1)]
fn adapters_local_process_stream_drain() {
    black_box(measured(local_process_stream_drain()));
}

#[divan::bench(sample_count = 10, sample_size = 1)]
fn adapters_model_stream_parsers() {
    black_box(measured(model_stream_parsers()));
}

#[divan::bench(sample_count = 10, sample_size = 1)]
fn daemon_authenticated_owner_round_trip() {
    black_box(measured(daemon_owner_round_trip()));
}
