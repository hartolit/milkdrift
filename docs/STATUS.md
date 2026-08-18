# Status

This document owns current implementation facts and limitations.

## Implemented

- A stable Rust 2024 workspace with two safe-Rust product crates and a curated lint/dependency policy.
- Private-invariant, bounded identities and versioned canonical JSON envelopes for capability descriptors and invocation contracts.
- Honest capability feature, cancellation, idempotency, side-effect, admission, locality, trust-zone, usage, and error representations; mutable observations remain separate from immutable descriptors.
- Private immutable blueprint revisions with deterministic semantic digests, explicit ancestry, optimistic base checks, and atomic mutation batches.
- Semantic representations and validation for task, conditional branch, fork, join, reducer, explicit repeat, wait, signal, pinned subworkflow, and terminal nodes.
- Typed interfaces, ports, bindings, safe conditions and paths, closed mutations, bounded diagnostics, canonical schema-v1 fixtures, hostile-input tests, property tests, and privacy compile-fail documentation tests.
- One CI workflow for formatting, checks, tests, Clippy, rustdoc, and dependency policy.

## Limitations

There is no run scheduler, event journal, projection, persistence adapter, crash recovery, live reconciliation, capability registry, executor, provider adapter, peer transport, daemon, CLI, or desktop application. Blueprint subworkflow validation proves compatibility with the pinned interface recorded in the reference; resolving that reference against a durable revision store belongs to the runtime/persistence boundary. Cost and resource fields are observations only and do not imply enforcement.
