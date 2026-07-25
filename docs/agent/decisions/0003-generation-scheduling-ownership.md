# ADR-0003: Schedule generation beside model execution

- **Status:** Accepted
- **Date:** 2026-07-22

## Context

The current hosted inference command/event API can issue prefill and decode operations, but a frontend- or E1-driven command round trip for every token would couple throughput to event polling, increase channel traffic, and split cancellation and sequence ownership across layers.

## Decision

The inference worker owns the high-frequency prefill/decode/sample scheduler. Sampling, stop-token matching, cancellation boundaries, and request cleanup execute beside the exclusively owned model and sequence.

`application-runtime` owns prompt preparation, tokenizer/decode state, public generation state, and conversion of bounded token batches into frontend-facing text batches. Host transport provides bounded pull-oriented accumulation. Frontends pull finite batches on their own cadence and do not drive individual token steps.

## Rejected alternatives

- **One command and event per generated token:** rejected because it introduces avoidable channel and scheduling overhead and makes generation depend on frontend cadence.
- **Run sampling in the frontend:** rejected because it duplicates model policy and exposes token-frequency orchestration to presentation code.
- **Run all text/UI state in the inference worker:** rejected because tokenizer presentation state and application semantics do not belong to model resource ownership.

## Consequences

- E0 gains a bounded generation scheduler and sampling dependency during the generation phase.
- Output APIs must support bounded pull batches and explicit saturation behavior.
- Cancellation can be checked between bounded model steps without handing sequence ownership to callers.
- Tests need a deterministic fake model to verify scheduling independent of vendor backends.

## Review trigger

Review if backend APIs require a materially different batching model, profiling shows the ownership split causes a measured bottleneck, or remote/process isolation changes where the model-execution boundary lives.
