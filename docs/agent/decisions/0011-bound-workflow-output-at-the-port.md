# ADR-0011: Bound workflow output at the service port

- **Status:** Superseded by ADR-0021
- **Date:** 2026-07-30

## Context

The corrective workflow declared per-artifact byte limits but originally asked model and validator ports to return fully owned `String` and `ValidationReport` values. The executor checked size only after those values had already been allocated. Limits therefore protected artifact commit, not peak output growth, and every port had to invent its own buffering behavior.

Generated artifacts also lacked workflow ownership. Successful runs accumulated permanently, while a late-stage failure could leave earlier outputs consuming the fixed artifact store without a terminal workflow result.

## Decision

The capability engine owns output storage and generated-artifact lifecycle.

- Model ports append UTF-8 chunks to an executor-owned `BoundedTextSink` and return only operational success or failure.
- Validator ports append typed findings to an executor-owned `BoundedDiagnosticsSink` and return the validation verdict separately.
- Sinks enforce the stage output contract before each append, reserve storage fallibly, never truncate, and retain the first capacity/allocation failure.
- Sink failures are non-retryable workflow failures and take precedence over a simultaneous port error. Retry budgets remain for operational model/validator failures.
- Diagnostic normalization uses fallible construction, deterministic sort/dedup semantics, and validates the final normalized report against its own artifact bound.
- Root specifications remain shared unowned artifacts. Every generated artifact records its `WorkflowId` owner.
- A failed execution removes its generated artifacts and queued events while preserving monotonic identifiers.
- A successful execution retains artifacts until `release_workflow` explicitly removes that workflow's outputs.

The workflow crate remains independent from `host-runtime`; its synchronous sinks are capability contracts, not process-channel implementations.

## Rejected alternatives

- **Check returned payloads after allocation:** this preserves commit integrity but not bounded output ownership.
- **Silently truncate at the byte limit:** truncated drafts or diagnostics would be semantically corrupt artifacts.
- **Treat sink overflow as a retryable port error:** repeating the same bounded request cannot make an oversized result valid.
- **Store all artifacts forever:** fixed entry capacity would become an executor lifetime limit rather than backpressure.
- **Depend directly on host channels:** workflow composition should not require one process-host implementation.

## Consequences

- Local E0, peer, or hosted model-task adapters can stream chunks into the same coarse workflow boundary.
- Artifact byte limits are observable during production rather than only at commit.
- Failed workflows are transactional with respect to artifact/event visibility.
- Callers choose how long successful workflow evidence remains inspectable and release it explicitly.
- Port implementations must adapt existing whole-value APIs by appending through the supplied sink.

## Review trigger

Review this decision when workflows become asynchronous, require concurrent consumers, persist artifacts outside the in-memory store, or need resumable execution across process restarts. Preserve bounded ownership and explicit lifecycle even if the concrete sink or storage implementation changes.
