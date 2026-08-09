# Milkdrift portable loading contracts, E0 ownership, and application boundary

## Objective

Audit and refine the public loading/runtime contracts after the artifact and Candle changes so Milkdrift exposes a precise, backend-neutral ownership model suitable for future embedders and execution endpoints.

This prompt owns portable loading contracts, E0 state/accounting, E1 translation, and persistence-facing application semantics. It does not redesign the future workflow runtime.

## Read first

Read:

- `README.md`
- `docs/vision.md`
- `docs/project/architecture.md`
- `docs/project/inference-runtime.md`
- `docs/project/application-runtime.md`
- ADRs 0006, 0010, 0013, 0019, and 0020
- `domain-contracts` backend/model/lifecycle/error modules
- `inference-runtime` admission, cleanup, memory, unload, shutdown, command, and snapshot code
- `application-runtime` model, retained cleanup, state, support, shutdown, and persistence integration
- the committed artifact and Candle changes from the preceding prompts

## Owned area

Primary ownership:

- `crates/domain/domain-contracts`
- `crates/runtime/inference-runtime`
- `crates/platform/host-runtime` where required by runtime ownership
- `crates/runtime/application-runtime`
- `crates/adapters/redb-storage` for application-state schema changes
- minimal Slint presenter adaptation only when a public E1 type changes

## Loading-contract typestate

Review the current relationship among `PreparedLoad`, `FailedLoad<P>`, and `ModelLoader::load_prepared`.

The API currently uses the same backend type for:

- a drop-safe unmaterialized preparation; and
- an ownership-bearing failed materialization that requires explicit cleanup.

Do not retain that conflation solely to minimize changes. Implement the clearest type-state contract.

A preferred direction is separate associated types or equivalent typestate:

```text
Preparation       -> safe to drop before materialization
PartialLoadOwner  -> created only after materialization began; explicit retryable cleanup
LoadedModel       -> final ownership after successful validation
```

The compiler and trait surface should make it difficult for a backend or runtime implementation to mistake one state for another.

Requirements:

- a rejected plan must be ordinary-drop-safe;
- failed materialization must return the sole partial owner;
- cleanup failure must preserve that owner for retry;
- primary failure and cleanup failure remain separate facts;
- E0 never infers release from error conversion or owner disappearance;
- the contract remains statically dispatched and portable;
- no filesystem, Candle, Hugging Face, or device-vendor type enters the portable API.

If you determine the current type is already the superior design, provide concrete type-safety reasoning in the ADR and strengthen tests accordingly. “Smaller diff” is not a reason.

## E0 ownership and accounting audit

Make the load transaction explicit in state and snapshots:

```text
prepared but unadmitted
loading peak admitted
materializing
partial cleanup pending/exhausted
loaded final reservation
unloading/cleanup pending
empty
```

Audit every success and failure edge for:

- aggregate host/device reservation;
- model identity indexes;
- partial owner indexes;
- cleanup retry counts;
- shutdown behavior;
- terminal retain-until-process-exit behavior;
- exactly-once transition from loading peak to final ownership;
- no moment where native resources exist outside admitted accounting.

Remove redundant or impossible states. Prefer a small typed state machine over flags that can contradict each other.

Preserve backend substitution validation for descriptor, device, scalar, footprint, capabilities, sequence identity, logits, and lifecycle.

## E1 cleanup-state consolidation

The current E1 path contains separate `retained_model_cleanup` and `incompatible_model_cleanup` tracking that can overlap conceptually.

Refactor this into one typed application-level cleanup state machine that distinguishes causes without duplicating ownership truth.

It must cover:

- lower failed-materialization cleanup;
- post-load receipt incompatibility followed by unload;
- normal unload cleanup failure;
- retryable submission/wait failures;
- lower cleanup exhaustion;
- disconnection;
- clean zero-ownership confirmation.

E1 must not recreate E0 retry policy or infer backend release. It should translate lower facts into stable application events and admission locks.

Remove duplicated error formatting and repeated transition code where one shared operation is semantically correct.

## Public API and fact boundaries

Tighten the public API around durable facts:

- declared configuration scalar;
- complete observed artifact scalar set;
- required/primary scalar only if it is useful outside the adapter and format-neutral;
- planned execution scalar;
- actual loaded execution scalar;
- selected and actual device;
- final and loading-peak footprints;
- cleanup ownership state.

Do not expose per-tensor names, shards, offsets, paths, or Candle conversion policy through E0/E1.

Review whether every newly public Phase 12 type is genuinely required by an embedder. Reduce accidental API surface while there are no external consumers.

Ensure the portable subset remains `no_std`; use `alloc` only in crates that explicitly opt into it. Do not make full hosted E0 `no_std` claims.

## Persistence

Version and migrate any state affected by source identity or scalar semantics.

Requirements:

- old records remain readable under their original meaning;
- absent declarations stay absent;
- observed artifact facts are not fabricated from old scalar fields;
- verified artifact identity is not reconstructed from a path;
- incompatible or stale records fail explicitly or trigger re-resolution rather than silently loading different bytes.

## Required tests

Add or retain fault coverage for:

- rejected preparation is drop-safe and creates no cleanup owner;
- failed materialization produces the distinct partial owner;
- cleanup failure/retry/exhaustion preserves exact peak accounting;
- successful retry releases exactly once;
- descriptor/device/scalar/footprint mismatch after native load follows explicit unload ownership;
- application state cannot be simultaneously loaded and cleanup-pending;
- device/model selection remains locked for every retained-ownership state;
- zero-ownership snapshot is the only automatic return to idle;
- disconnection never implies release;
- persistence migrations preserve old meanings;
- no E1 test needs Candle tensor internals to assert public behavior.

## Validation

Run focused default and CUDA compile tests for all owned packages, including all E0 fault injection and E1 lifecycle/retained-cleanup tests.

At minimum:

```text
cargo test --locked -p domain-contracts -p host-runtime -p inference-runtime -p application-runtime -p redb-storage
cargo check --locked -p inference-runtime -p application-runtime --features cuda
cargo test --locked -p inference-runtime -p application-runtime --features cuda --no-run
cargo clippy --locked -p domain-contracts -p host-runtime -p inference-runtime -p application-runtime -p redb-storage --all-targets -- -D warnings
cargo clippy --locked -p inference-runtime -p application-runtime --all-targets --features cuda -- -D warnings
cargo check --locked --target wasm32-unknown-unknown --lib -p domain-contracts
cargo check --locked --target thumbv7em-none-eabihf --lib -p domain-contracts
cargo fmt --all -- --check
git diff --check
```

## Finish

Commit one coherent runtime/API cleanup. Do not preserve parallel legacy APIs when there are no external users. Do not push.
