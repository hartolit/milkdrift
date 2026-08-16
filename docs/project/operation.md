# Local execution operation

This guide follows the implemented local path from model selection through
shutdown. It identifies the owner of each transition without copying the complete
support matrix or every failure enum. Current availability and evidence live in
[implementation status](implementation-status.md); component details are linked
at each boundary.

## End-to-end ownership map

| Step | Primary owner | Main type or boundary |
|---|---|---|
| Select device and artifact | `application-runtime` (E1) | `ApplicationRuntime`, `ModelSelection`, `ApplicationDevice` |
| Resolve immutable artifacts | E1 Hub worker and `hf-hub-adapter` | `ResolvedSafetensorsLlamaArtifacts`, public `ResolvedModel` |
| Inspect and prepare exact load | `candle-backend` | `CandleLlamaSource`, `CandleLlamaPreparedLoad`, `LoadPlan` |
| Admit loading peak | `inference-runtime` (E0) | `InferenceRuntime::load_model`, `MemoryFootprint` |
| Materialize and verify model | `candle-backend`, then E0 | `ModelLoader::load_prepared`, loaded descriptor and receipt |
| Publish application model | E1 | private `ModelLoadTransaction`, public `LoadedModel` |
| Admit generation | E1, then E0 | E1/E0 generation admission transactions |
| Prefill, sample, publish, decode | E0 scheduler and Candle model/sequence | hosted worker `WorkerState`, typed token/text output |
| Cancel, backpressure, terminal publication | E0, bridged by E1 | phase transitions and ordered output records |
| Destroy sequence and publish release | Candle and E0 | `LoadedModel::destroy_sequence`, cleanup quarantine |
| Unload model | E1 correlation, E0 ownership, Candle release | unload command/receipt and `prepare_unload` |
| Shutdown and join | E1 plus both hosted workers | explicit bounded `ApplicationRuntime::shutdown` |

## 1. Resolve and select the artifact

A host passes repository/revision through `ModelSelection` and selects
`ApplicationDevice::Cpu` or an explicitly available CUDA ordinal. Selection and
device are independent state; neither resolution nor a stored preference silently
changes the requested device.

E1 submits resolution to its bounded Hub worker. `hf-hub-adapter` resolves the
mutable revision to an immutable 40-character commit, discovers only the supported
configuration/tokenizer/Safetensors layout, and retains provider evidence for each
shard. Exact LFS length/SHA-256 or verified Git-blob identity is distinguished
from project-established identity.

E1 publishes a `ResolvedModel` only after selection, immutable identity,
configuration declaration state, tokenizer vocabulary, and chat compatibility are
coherent. Resolution initializes no execution device and makes no execution-scalar
claim. See [application runtime](application-runtime.md).

## 2. Inspect and prepare one exact load

E1 re-probes the selected device, converts resolved artifact identities into the
provider-neutral expectations accepted by `CandleWeightShard`, constructs a
private `CandleLlamaSource`, and retains a ticketed `ModelLoadTransaction`.

E0 calls `ModelLoader::prepare_load` exactly once. Candle then:

- reads bounded configuration and complete Safetensors headers before device
  initialization;
- verifies structure and the required Llama tensor schema;
- keeps configuration declaration, complete observed scalar categories, required
  scalar policy, and execution scalar as separate facts;
- retains open shard handles and expected content identities;
- selects the requested device/execution representation;
- constructs the immutable required-range and accelerator transfer plan; and
- reports separate exact final and loading-peak footprints.

The resulting `CandleLlamaPreparedLoad` is bound to the exact source,
configuration, device, budget, and plan. Before materialization it is ordinary
drop-safe. The scalar and loading algorithms are owned by the
[Candle guide](candle-backend.md), not duplicated by E0 or E1.

## 3. Admit the loading peak

E0 copies the stable `LoadPlan`, validates portable invariants, and checks both
existing ownership plus the loading peak and existing ownership plus final
footprint against its aggregate budget. It reserves the loading peak before any
fallible native materialization.

All cross-crate deterministic byte facts use `ByteCount`, whose representation is
portable `u64` and whose arithmetic is named and checked. Raw values are exposed
only at persistence, report serialization, display, and platform allocation
boundaries.

Admission also checks handle/generation identity, descriptor limits, requested
device, and checked arithmetic. E0 does not interpret tensor names, choose a
required primary scalar, partition transfers, or impose CPU/CUDA placement.

## 4. Materialize, validate, and publish the loaded handle

E0 consumes the same preparation through `ModelLoader::load_prepared`. Candle
sequentially re-verifies each retained shard, hashes ignored ranges through a
fixed buffer, allocates only required tensors, performs required conversion, and
uses bounded shard-aware transfer batches for accelerator loading. A shard's final
batch does not commit until whole-file identity succeeds.

After native construction, E0 verifies the complete model against the admitted
handle, descriptor, requested/actual device, planned/actual execution scalar,
final footprint, and lifecycle state. Only then does it replace the loading-peak
reservation with final ownership and publish a `LoadReceipt`.

E1 correlates that receipt with its `ModelLoadTransaction` and performs only
application-level checks: ticket, immutable model identity, declaration,
selected/actual device, execution scalar, budget totals, observed-evidence
nonemptiness, capabilities, limits, composition, and tokenizer vocabulary. A
public `LoadedModel` is published only after all checks pass.

If materialization acquires resources and fails, Candle returns the distinct
`CandleLlamaFailedPreparation` sole owner. If a complete model contradicts its
contract, E0 attempts explicit unload rather than publishing it. Failed cleanup
stays retained and blocks unsafe admission; see [inference runtime](inference-runtime.md)
and [model lifecycle](lifecycle.md).

## 5. Submit and admit generation

The host calls `start_generation` for direct completion or `submit_user_message`
for the exact supported chat profile. E1 validates state and settings, tokenizes
once, constructs any chat context/prompt, preallocates bounded decoding and output
state, allocates correlated request/sequence identities, and submits one E0
command before publishing the active application session.

E0 validates model lifecycle, prompt/context/prefill limits, output capacity,
sampling policy, stop data, and aggregate memory. Its generation admission owns
caller workspaces and a nested `SequenceAdmissionTransaction`. Candle plans the
sequence reservation from loaded execution geometry; its checked constructor
owns the derived total, and E0 admits the complete persistent-plus-transient total
before native sequence creation and verifies the created sequence's identity,
capacity, and immutable plan before scheduler visibility.

## 6. Prefill, sample, publish, and decode

The hosted E0 `WorkerState` owns the high-frequency loop. One scheduler
opportunity performs at most one backend phase:

```text
admitted
  -> prefill
  -> sample host F32 logits
  -> publish pending token
  -> decode
  -> repeat or stage terminal
```

Candle owns model weights and the independent sequence cache. E0 owns token
history, sampling, stop matching, generation workspace, fairness, and terminal
state. Token output enters `host-runtime`'s bounded typed token store. E1 pulls
those records, performs request-local streaming decode, and appends bounded UTF-8
fragments to the separate typed text store. Frontends pull text at their own
cadence; no frontend drives one backend command per token.

## 7. Backpressure, cancellation, and terminal state

When token output is full or temporarily consumer-busy, E0 retains the exact
pending token and performs no decode. It yields without growing storage. E1 text
output follows the same bounded behavior through a shared private storage core
while preserving a distinct public type.

Cancellation is observed before the next backend operation. EOS, token limit,
stop match, cancellation, backend failure, sampling failure, unload drain, and
shutdown all converge on the same explicit terminal and sequence-cleanup path.
Terminal generation outcome is not release evidence.

E0 publishes records in order:

```text
Terminal
  -> optional CleanupPending
  -> optional CleanupExhausted
  -> Released
```

Generation workspace accounting remains owned until `Released` is published and
scheduler storage is dropped.

## 8. Clean the sequence and publish release

Candle explicitly destroys the backend sequence and synchronizes the selected
device where required. E0 verifies identity, capacity, and the complete sequence
plan around destruction. Success releases the admitted sequence total exactly
once.

A destruction failure moves the sole sequence owner into cleanup quarantine.
Matching reports retain exact ownership; a contradiction becomes unverified
ownership, is excluded from exact aggregate bytes, and blocks new admission.
Maintenance performs bounded fair retry. Missing snapshot entries, zero exact
bytes, or a disconnected endpoint never prove release.

E1 keeps terminal conversation/application state distinct from lower resource
release. It stores complete retained evidence in `ApplicationState` and exposes
compact transition events for hosts.

## 9. Unload and clean retained model resources

E1 correlates unload behavior (`RejectIfBusy`, cancellation, or bounded drain)
with the selected model. E0 prevents new requests, completes required sequence
cleanup, asks Candle to synchronize and prepare unload, and drops native ownership
only after explicit success. The final reservation is removed once and the
unload receipt is correlated back through E1.

Failed-load, incompatible-model, normal-unload, disconnect, and shutdown cleanup
all pass through E1's private `ModelCleanupCoordinator`. Normal `LoadedModel` and
`ApplicationRetainedModel` never coexist. Retryable lower cleanup and E1
coordination retry are distinct; lower exhaustion cannot be reset by the host.

## 10. Shutdown and join

Hosts call `ApplicationRuntime::shutdown`; `Drop` does not perform an unbounded
join. E1 attempts Hub stop, E0 shutdown, and both joins independently so one
failure does not hide another. Retryable join timeouts retain their handles.

E0 consumes only the finite remaining cleanup budget. Clean correlated shutdown
is release evidence. If native ownership remains, E0 returns terminal cleanup
retention and the hosted worker deliberately keeps the runtime allocation alive
until process exit. It may still terminate and be joined; worker exit or handle
absence alone is not cleanup success.

The [validation guide](validation.md) explains how CPU, hosted, CUDA hardware, and
external-product evidence establish these boundaries without conflating them.
