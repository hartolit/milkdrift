//! Allocation enforcement for prepared text and token output cycles.

#![forbid(unsafe_code)]

use std::alloc::System;
use std::num::NonZeroUsize;
use std::process::ExitCode;

use domain_contracts::{RequestId, TokenId};
use host_runtime::{text_output_accumulator, token_output_accumulator};
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, Stats, StatsAlloc};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

const CYCLES: usize = 64;

fn main() -> ExitCode {
    match measure_prepared_cycles() {
        Ok((change, checksum))
            if change.allocations == 0 && change.reallocations == 0 && checksum == CYCLES * 6 =>
        {
            ExitCode::SUCCESS
        }
        Ok((change, checksum)) => {
            eprintln!(
                "prepared output cycles changed allocation state or data: {change:?}, checksum={checksum}"
            );
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("prepared output cycle failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn measure_prepared_cycles() -> Result<(Stats, usize), &'static str> {
    let (text_producer, text_consumer) =
        text_output_accumulator::<u8>(NonZeroUsize::MIN, NonZeroUsize::MIN)
            .map_err(|_| "text initialization")?;
    let (token_producer, token_consumer) =
        token_output_accumulator::<u8>(NonZeroUsize::MIN, NonZeroUsize::MIN)
            .map_err(|_| "token initialization")?;
    let request = RequestId::new(1);
    let token = TokenId::new(7);
    let mut checksum = 0_usize;

    let region = Region::new(GLOBAL);
    for _ in 0..CYCLES {
        text_producer
            .try_push_text(request, "a")
            .map_err(|_| "text push")?;
        checksum = checksum.saturating_add(
            text_consumer
                .pull(|batch| batch.bytes.len() + batch.records.len())
                .map_err(|_| "text pull")?,
        );

        token_producer
            .try_push_token(request, token)
            .map_err(|_| "token push")?;
        checksum = checksum.saturating_add(
            token_consumer
                .pull(|batch| batch.tokens.len() + batch.records.len())
                .map_err(|_| "token pull")?,
        );

        text_producer
            .try_push_state(request, 1)
            .map_err(|_| "text state push")?;
        checksum = checksum.saturating_add(
            text_consumer
                .pull(|batch| batch.bytes.len() + batch.records.len())
                .map_err(|_| "text state pull")?,
        );

        token_producer
            .try_push_state(request, 1)
            .map_err(|_| "token state push")?;
        checksum = checksum.saturating_add(
            token_consumer
                .pull(|batch| batch.tokens.len() + batch.records.len())
                .map_err(|_| "token state pull")?,
        );
    }
    Ok((region.change(), checksum))
}
