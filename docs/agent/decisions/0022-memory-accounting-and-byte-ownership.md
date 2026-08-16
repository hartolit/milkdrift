# ADR-0022: Make deterministic byte ownership typed and non-contradictory

- **Status:** Accepted
- **Date:** 2026-08-16

## Context

Deterministic memory facts crossed package boundaries as raw `u64` fields.
`MemoryFootprint`, `MemoryBudget`, and `SequenceReservation` could be constructed
with contradictory values, including a reservation total unrelated to its
persistent and transient components. Candle inspection also published a cache
byte rate chosen with CPU scalar semantics, while the loaded model recomputed the
same Llama formula for the actual execution scalar. Duplicate `usize` and `u64`
verification-buffer constants and mirrored intermediate-value tests made a
geometry change require coordinated edits in several places.

## Decision

`domain-contracts` owns one no-std-compatible `ByteCount`: a private `u64` with
explicit raw construction/access, named checked arithmetic, and checked `usize`
conversion at allocation boundaries. Portable footprints, budgets, byte-valued
failures, device observations, application projections, and benchmark observation
facts use it. Persistence and JSON records keep their existing raw numeric shape
through explicit boundary conversion.

`MemoryFootprint` and `MemoryBudget` have private components with named
constructors, modifiers, and accessors. `SequenceReservation` accepts only
persistent and transient footprints; its checked constructor privately derives
the aggregate. No consistency API or public struct literal remains.

Candle inspection reports device-independent metadata and its CPU-style tensor
estimate only. It publishes no execution cache byte rate. The adapter's private
`LlamaMemoryGeometry`, built from validated configuration plus the selected
execution dtype and device policy, is the sole implementation owner for concrete
sequence persistent/transient planning. The single execution-dtype-width mapping
is shared by load and sequence planners. E0 admits the reported generic total but
does not reconstruct Llama geometry.

The fixed verification workspace is one typed constant. Tests use checked
arithmetic properties, small hand-reviewed final boundary vectors, a representative
TinyLlama vector with provenance, and conformance against actual Candle cache
tensor shapes and fixture reports. They do not snapshot every planner-local
intermediate.

## Rejected alternatives

- **Keep raw public fields for compatibility:** this preserves contradictory
  construction and makes every consumer part of the invariant boundary.
- **Publish scalar-independent cache elements in the descriptor:** no current
  portable consumer needs that profile; the concrete loaded-model plan is the
  real admission boundary.
- **Add a generic cross-backend memory-model trait or units framework:** current
  requirements need one byte type and one Candle/Llama owner, not hypothetical
  abstractions.
- **Recompute Candle geometry in E0:** that would make admission a second backend
  implementation and recreate formula drift.

## Consequences

- Raw byte struct literals are an intentional breaking API change.
- A geometry/scalar correction changes one adapter owner and a small independent
  golden boundary; property and conformance tests detect accidental drift.
- Exact arithmetic never saturates. Observation counters may saturate only while
  separately invalidating their evidence record.
- Physical RSS/VRAM, allocator fragmentation, pools, contexts, and driver
  workspaces remain observations rather than deterministic ownership claims.

## Review trigger

Review when a second backend needs a portable memory fact not representable by
the current four-component footprint and concrete sequence reservation, or when a
versioned external schema needs to expose typed byte semantics rather than its
current numeric representation.
