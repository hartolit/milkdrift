# Inference runtime

`crates/runtime/inference-runtime` is the E0 single-owner model registry and
backend-independent generation scheduler. It is generic over one concrete
`ModelLoader` and owns every loaded model, backend sequence, generation workspace,
lifecycle transition, and aggregate memory reservation.

## Ownership and accounting

```text
Hosted worker
├── InferenceRuntime<L>
│   ├── normal model registry
│   │   └── ModelSlot<L::Model>
│   │       ├── exclusively owned model
│   │       ├── ModelLifecycle
│   │       ├── active request sequences
│   │       └── quarantined sequences
│   ├── quarantined post-load models
│   ├── active and pending-cleanup identity indexes
│   ├── aggregate normal + quarantined memory accounting
│   └── generation-workspace accounting retained through output release
├── fair generation scheduler
└── nonblocking token-output producer
```

Models and sequences are never placed in `Arc` or borrowed across the command
boundary. Public clients retain only typed identifiers and generation-safe model
handles. A resource remains counted until its explicit backend cleanup succeeds.
`RuntimeSnapshot` distinguishes active requests, retained generation workspaces,
pending model cleanup, pending sequence cleanup, exhausted cleanup, and total
reserved memory. Per-model snapshots expose degraded state and pending sequence
counts.

## Transaction and cleanup semantics

Model and sequence admission follow prepare, validate, commit. Host-side generation
workspaces are reserved before sequence creation. Registry indexes, lifecycle state,
and normal active-request accounting are published only after validation succeeds.

Cleanup failure does not imply release:

- a model that fails post-load validation and `prepare_unload` is retained outside
  the normal model registry;
- an uncommitted sequence whose `destroy_sequence` fails is retained outside the
  active request registry;
- a terminal request whose sequence destruction fails moves from active ownership
  to quarantine;
- quarantined bytes and sequence slots remain admitted against hard limits;
- an affected model is degraded and rejects new requests;
- `poll_cleanup` attempts at most one retained operation per call;
- the initial failure counts as attempt one and the configurable total-attempt
  limit defaults to three;
- exhausted resources are skipped by later automatic maintenance and remain
  quarantined and accounted;
- successful retry releases identity, capacity, and memory exactly once.

`CleanupFailureReport` is allocation-free and preserves the primary operation and
failure class independently from the cleanup operation and failure class. It avoids
recursive boxed error chains while retaining stable categories for later E1
translation.

Backend cleanup hooks are retry contracts: `destroy_sequence(&mut sequence)` must
leave the borrowed sequence valid after failure, and `prepare_unload(&mut model)`
must leave the model valid after failure. The runtime never treats unverified
`Drop` behavior as successful explicit cleanup.

## Backend contract verification

Rust trait conformance is necessary but not sufficient for backend substitution.
During model admission E0 validates internally ordered non-zero descriptor limits,
capability consistency, and equality between the accepted plan and the complete
descriptor retained by the loaded model. Sequence requests and backend plans must
remain within the descriptor's context and prefill limits.

A successful prefill or decode result is accepted only when it:

- uses an advertised operation;
- preserves the admitted sequence identity and fixed token capacity;
- leaves the sequence in `Ready` state;
- advances the exact expected position;
- reports the exact consumed prompt count where applicable;
- writes exactly the model vocabulary's logits when logits are requested.

A contradiction becomes `BackendContractViolation` before sampling. The request then
uses the ordinary explicit destruction/quarantine path, so a malformed adapter cannot
bypass ownership cleanup. Unsupported caller requests and missing advertised
operations remain explicit unsupported-operation failures. The decision and rejected
alternatives are recorded in [ADR-0010](../agent/decisions/0010-verify-backend-contracts-at-e0.md).

## Generation admission

`RuntimeCommand::Generate` carries the minimum token-level runtime request:

- request and sequence identity;
- prompt token storage;
- sequence capacity and maximum generated tokens;
- sampling configuration and seed;
- EOS tokens and owned token stop patterns;
- scheduler quantum;
- minimum token and record capacity required from the shared pull accumulator.

It does not carry tokenizer objects, decoded text, paths, display strings, frontend
DTOs, or UI state. Before backend sequence creation, E0 validates prompt and total
sequence length, model state, identities, required prefill/decode capabilities,
advertised context/prefill limits, and sampling configuration, then reserves:

- vocabulary-sized logits;
- sampling indices and repetition epochs;
- prompt/repetition history;
- generated-token history;
- caller-owned prompt and EOS token storage;
- stop-pattern descriptors and token storage;
- terminal and backpressure state.

The backend still prepares its sequence-owned prefill/decode workspace through its
normal `SequencePlan`. No vector resize occurs in the scheduler decode loop.
Workspace payload bytes remain in aggregate admission accounting until the
`Released` record has been published and the scheduler drops the terminal task.
This prevents output backpressure from making retained host allocations appear
available prematurely.

## Scheduler lifecycle and fairness

A scheduled request moves through explicit phases:

```text
admitted -> prefill -> pending token publication -> decode
    -> terminal publication -> cleanup pending (optional) -> released
```

The worker checks one control command, advances one request by a bounded opportunity,
processes one cleanup retry and unload maintenance, and flushes bounded events on
each loop. Request selection uses a rotating ordered cursor, so runnable requests
each receive an opportunity. A request waiting on full output does not perform
another backend step and therefore cannot monopolize model execution.

The current scheduler intentionally performs at most one token-producing backend
step before token publication even if a larger configured quantum is retained. This
is the correctness baseline; later measured tuning may batch a small number of
steps without changing the contract.

Prefill occurs once. Sampling runs inside E0 immediately from checked logits using
request-owned `sampling::Sampler` state. The selected token is appended to bounded
history before any subsequent decode. EOS, generated-token limit, and token stop
suffixes are checked after ordered token publication.

## Pull output and backpressure

`host-runtime` supplies a separate token accumulator rather than encoding token IDs
as UTF-8 byte ranges. It preallocates token and record vectors during worker setup.
The producer uses `try_lock`; the application pulls a borrowed batch and clears its
logical contents while retaining both allocations.

Records preserve request identity and contain either an absolute monotonic token
range or one `GenerationOutputState`:

- `Yielded(OutputBackpressure)`;
- `Terminal(original outcome)`;
- `CleanupPending { original outcome, failure report, retry state }`;
- `CleanupExhausted { original outcome, failure report, retry state }`;
- `Released(original outcome)`.

When token or record capacity is full, the sampled token remains request-owned,
no decode step is performed, and no token is discarded or emitted twice. After a
pull frees capacity, the yield record and exact pending token are published before
decode resumes. Generation completion and backend resource release are therefore
observable as separate ordered facts.

## Cancellation, unload, and shutdown

User cancellation is recorded as a control operation and observed before the next
backend step. Latency is bounded by one currently executing backend operation, the
one-step correctness quantum, and the worker command polling cadence. Cancellation
always enters the same terminal cleanup path as EOS, token limits, stop patterns,
and failures.

Immediate model unload marks scheduled requests with `ModelUnload`; drain timeout
maintenance marks them with `DrainTimeout`. The runtime may have already destroyed
the sequence at that safe boundary, but the scheduler still publishes the stable
cancellation outcome during normal operation.

Explicit runtime shutdown is a terminal worker transition. It performs bounded
sequence and model cleanup, releases retained generation-workspace accounting, and
discards unpublished scheduler records rather than waiting for downstream token
capacity. Shutdown therefore cannot depend on the UI continuing to drain output.
The worker sends exactly one shutdown result and terminates after that event is
delivered.

Shutdown consumes the remaining finite retry budget. If ownership still remains,
it returns `CleanupRetryExhausted` and does not report successful shutdown. Failed
explicit cleanup preserves the unresolved runtime allocation rather than falling
back to an unverified implicit backend drop. The same preservation rule applies
when client endpoints disconnect before cleanup can complete.

## Production Candle integration and backend-independent tests

Ordinary scheduler and fault-injection coverage remains backend-independent. Deterministic test loaders in `tests/generation.rs`, `tests/runtime.rs`, and `tests/fault_injection.rs` exercise transaction rollback, descriptor/receipt verification, sampling, fairness, cancellation, output backpressure, cleanup retry/exhaustion, unload, accounting, disconnection, and shutdown without requiring another production engine.

`tests/native_backend_generation.rs` is the download-free real-adapter E0 contract. It drives the committed tiny Safetensors fixture through `CandleLlamaLoader` and the hosted scheduler. The two tests cover:

- model inspection, admission, load, and descriptor verification;
- prompt prefill and incremental decode through `RuntimeCommand::Generate`;
- deterministic greedy output and token-limit finish;
- repeatable seeded sampling;
- EOS finish and ordered terminal/released publication;
- observable output backpressure without token loss;
- user cancellation at a backend boundary;
- sequence destruction plus complete request, generation-workspace, cleanup, and memory accounting release;
- model unload, an empty post-unload runtime/model snapshot, terminal shutdown, and worker join.

The fixture requires no network access and validates execution and lifecycle contracts rather than language quality. The CPU suite does not exercise a GPU path or establish an allocation-free Candle hot path. See [download-free focused validation](validation.md#download-free-focused-validation).

External artifact resolution belongs to E1 rather than E0. The opt-in [Rust-native Candle/Hub smoke](validation.md#rust-native-candle-hub-smoke) resolves an exact immutable Hub revision through the production Hub worker and then exercises E1, E0, and Candle. The E0-only `candle_llama_smoke` example remains a local diagnostic for artifacts that are already resolved; it performs no network or Hub work.

Product-level composition and unsupported capabilities are tracked in [implementation status](implementation-status.md); this guide remains focused on E0 behavior rather than roadmap sequencing.
