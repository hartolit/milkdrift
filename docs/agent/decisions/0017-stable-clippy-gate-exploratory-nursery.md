# ADR-0017: Keep stable Clippy lints mandatory and nursery exploratory

- **Status:** Accepted
- **Date:** 2026-08-01

## Context

The workspace applies strict Clippy policy to all targets and promotes warnings to errors in its canonical gate. Applying the entire `clippy::nursery` group through the same mandatory workspace policy makes acceptance depend on a deliberately evolving lint set whose diagnostics and false-positive profile can change with the pinned Rust toolchain.

Nursery findings are still useful as early design feedback, but their exploratory value does not require every finding to block ordinary changes.

## Decision

The mandatory workspace Clippy policy consists of the stable `clippy::all` and `clippy::pedantic` groups plus the explicitly selected `clippy::unwrap_used`, `clippy::expect_used`, `clippy::panic`, and `clippy::indexing_slicing` lints. The canonical `cargo xtask verify` command runs Clippy for the whole workspace and all targets with warnings denied. Findings from this mandatory set are acceptance failures unless handled by a narrow, intentional code-level exception.

Do not enable the blanket `clippy::nursery` group in inherited workspace lints or in the mandatory composite gate. Run it separately as an exploratory, non-blocking report, including on the scheduled CI review. Nursery diagnostics must remain visible, but they do not determine the success of the mandatory gate.

A nursery lint may become mandatory only through exact review of that lint after its signal, churn, applicability, and necessary exceptions are understood. Promotion selects the individual lint or accepts it through a stable mandatory group; it does not promote the entire nursery by implication.

## Rejected alternatives

- **Keep all nursery lints mandatory:** the acceptance gate would inherit churn from an explicitly experimental lint category.
- **Stop running nursery lints:** that would discard useful early feedback and delay awareness of likely future lint improvements.
- **Make all Clippy output advisory:** this would weaken the established workspace-wide quality gate to solve a problem limited to nursery stability.
- **Add broad crate-level nursery suppressions:** blanket suppression hides signal and makes later promotion harder to assess.

## Consequences

- `cargo xtask verify` retains a strict, deterministic Clippy acceptance boundary over the selected stable policy.
- Scheduled nursery runs can surface new findings without turning toolchain experimentation into merge-blocking churn.
- Nursery findings require triage rather than automatic cleanup or blanket suppression.
- The mandatory set can evolve one reviewed lint at a time without weakening existing explicit safety and correctness policy.

## Review trigger

Review when the pinned Rust/Clippy toolchain changes materially, an exploratory lint repeatedly finds actionable defects with low noise, a selected mandatory lint becomes obsolete, or the scheduled nursery report ceases to provide useful signal.
