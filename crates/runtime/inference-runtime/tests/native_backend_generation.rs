//! Download-free Candle real-fixture coverage for ordinary E0 generation and lifecycle.

#[path = "support/native_backend/mod.rs"]
mod native_backend;

use native_backend::TestResult;

#[test]
fn candle_fixture_covers_generation_sampling_eos_and_lifecycle() -> TestResult {
    native_backend::candle_fixture_covers_generation_sampling_eos_and_lifecycle()
}

#[test]
fn mixed_f16_f32_fixture_covers_e0_generation_accounting_and_lifecycle() -> TestResult {
    native_backend::mixed_f16_f32_fixture_covers_e0_generation_accounting_and_lifecycle()
}

#[test]
fn candle_fixture_covers_output_backpressure_and_cancellation() -> TestResult {
    native_backend::candle_fixture_covers_output_backpressure_and_cancellation()
}
