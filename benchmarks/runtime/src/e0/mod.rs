//! Hosted public-E0 baseline ownership.

mod harness;
mod synthetic;

pub(crate) use synthetic::{
    GENERATION_TOKEN_COUNT, POST_FIRST_TOKEN_WINDOW, SyntheticCycles, run_cycles,
};
