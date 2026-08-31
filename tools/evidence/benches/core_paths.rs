//! Repeatable Divan benchmarks for product-critical bounded paths.

use divan::black_box;
use milkdrift_evidence::{
    EvidenceResult, ScenarioMeasurement, application_receipt_paths, artifact_range_read,
    context_discovery_and_selection, context_materialization, daemon_owner_round_trip,
    journal_append_batch, journal_append_one, local_process_stream_drain, model_stream_parsers,
    peer_observation_paths, projection_rebuild, projection_snapshot_tail,
};

fn main() {
    divan::main();
}

fn measured(result: EvidenceResult<ScenarioMeasurement>) -> ScenarioMeasurement {
    match result {
        Ok(measurement) => measurement,
        Err(error) => {
            eprintln!("benchmark fixture failed: {error}");
            std::process::exit(1);
        }
    }
}

#[divan::bench(sample_count = 10, sample_size = 1)]
fn persistence_journal_append_one() {
    black_box(measured(journal_append_one()));
}

#[divan::bench(sample_count = 10, sample_size = 1)]
fn persistence_journal_append_batch() {
    black_box(measured(journal_append_batch()));
}

#[divan::bench(sample_count = 10, sample_size = 1)]
fn runtime_projection_rebuild() {
    black_box(measured(projection_rebuild()));
}

#[divan::bench(sample_count = 10, sample_size = 1)]
fn runtime_projection_snapshot_tail() {
    black_box(measured(projection_snapshot_tail()));
}

#[divan::bench(sample_count = 10, sample_size = 1)]
fn persistence_application_receipts() {
    black_box(measured(application_receipt_paths()));
}

#[divan::bench(sample_count = 10, sample_size = 1)]
fn persistence_peer_observations() {
    black_box(measured(peer_observation_paths()));
}

#[divan::bench(sample_count = 10, sample_size = 1)]
fn context_metadata_selection() {
    black_box(measured(context_discovery_and_selection()));
}

#[divan::bench(sample_count = 10, sample_size = 1)]
fn context_selected_materialization() {
    black_box(measured(context_materialization()));
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
