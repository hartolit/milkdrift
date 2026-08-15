//! Rust-owned repository hygiene validation.

mod documentation;
mod invocation;
mod manifest;
mod orchestration;

pub use orchestration::{
    HygieneError, HygieneReport, HygieneViolation, validate_repository_hygiene,
};
