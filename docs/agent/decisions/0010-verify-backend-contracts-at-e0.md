# ADR-0010: Verify backend contracts at E0

- **Status:** Accepted
- **Date:** 2026-07-30

## Context

`inference-runtime` supports statically dispatched model backends through portable contracts. During Phase 8, Candle and GGUF both satisfied those Rust traits, demonstrating that trait conformance alone does not prove that a loaded model agrees with its admitted plan or that each backend step preserves the sequence invariants E0 relies on.

Accepting partial logits, contradictory positions, mutable sequence identity, or capabilities that disagree with numeric limits would let an adapter silently change scheduler behavior. The Phase 8 second-backend composition made that risk concrete; the verification rule remains valid after the later production-engine cleanup.

## Decision

E0 treats backend descriptors, plans, and operation receipts as claims that must be verified at their ownership boundary.

- `LoadedModel` returns its complete retained `ModelDescriptor`, not metadata alone.
- Model admission compares the complete loaded descriptor with the accepted load plan before publication.
- Descriptor numeric limits must be non-zero and internally ordered. Multiple advertised sequences require the corresponding capability bit.
- Requested and backend-planned sequence capacities must fit the admitted model context and prefill limits.
- Scheduled generation requires prefill and incremental-decode capabilities; direct calls also reject operations the descriptor does not advertise.
- Every successful prefill/decode receipt must preserve the admitted sequence identity and fixed capacity, leave the sequence ready, advance the exact expected position, and report the exact logits count required by the model vocabulary.
- A contradiction is a backend contract violation. E0 follows its existing explicit sequence/model cleanup and quarantine rules rather than dropping ownership implicitly.

Static dispatch remains below E0. These checks do not introduce a dynamic backend plugin interface or make remote providers local backends.

## Rejected alternatives

- **Trust adapter tests and debug assertions:** production ownership and sampling cannot depend on every adapter being correct in all builds.
- **Sample the logits prefix a backend reports:** this silently changes the effective vocabulary and can produce invalid cross-backend behavior.
- **Validate only metadata:** backend identity, capabilities, and admitted limits are also scheduler contracts.
- **Move checks into every adapter:** E0 is the common owner that can enforce one substitution contract consistently.

## Consequences

- A backend that satisfies the traits but contradicts its descriptor fails explicitly before invalid state reaches sampling or application code.
- Candle and deterministic test loaders share stronger executable conformance requirements without dynamic dispatch in token-sensitive paths.
- Fault-injection tests characterize the boundary independently from native libraries.
- Any future implementation requires accurate descriptors and receipts; this is intentional integration work rather than incidental trust.
- [ADR-0013](0013-candle-only-local-execution.md) later removed the Phase 8 second production engine without weakening this E0 verification boundary.

## Review trigger

Review this decision if a backend needs asynchronous/pending step receipts, variable logits domains, mutable sequence capacity, or another execution model that cannot honestly satisfy the current local E0 invariants. Such a target may require a different explicit contract rather than weakening this one.
