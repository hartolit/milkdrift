# Milkdrift Candle loading subsystem: correctness, single-pass materialization, and structure

## Objective

Turn the Phase 12 Candle loader into a clean, scalable subsystem whose dtype policy is based on required tensors, whose observed artifact facts remain complete, whose normal loading path reads each shard payload once, and whose exact memory plan matches the actual algorithm.

This prompt owns Safetensors inspection, Llama schema validation, dtype selection, materialization, conversion, device transfer, and partial-load cleanup inside `candle-backend`.

## Read first

Read:

- the committed result of the model-artifact trust prompt;
- `docs/agent/decisions/0020-transactional-prepared-model-loading.md`;
- `docs/project/candle-backend.md`;
- `crates/adapters/candle-backend/src/loader.rs`;
- `source.rs`, `model.rs`, `failure.rs`, and device code;
- CPU/CUDA adapter tests and native E0 fixture tests.

Do not preserve the current algorithm solely because tests describe it.

## Owned area

Primary ownership:

- `crates/adapters/candle-backend/src/`
- Candle adapter CPU/CUDA tests
- model fixture generation/provenance required for deterministic tests
- adapter-local documentation and durable ADR amendments for the loading algorithm

Only make minimal contract changes outside this area when the prior artifact handoff requires them. Leave E0/E1 cleanup-state redesign to the next prompt.

## Mandatory correctness changes

### Separate complete artifact evidence from execution-relevant evidence

Retain both facts explicitly:

```text
all observed tensor scalar types
required Llama tensor scalar types
```

The full observed set describes the artifact. The required set determines primary execution policy.

An unused auxiliary tensor must never change the execution scalar or make a compatible required tensor set appear contradictory.

Required examples:

```text
required {F32}, extra {F16}  -> primary F32, observed {F32,F16}
required {F32}, extra {BF16} -> primary F32, observed {F32,BF16}
required {F16,F32}           -> primary F16
required {BF16,F32}          -> primary BF16
required {F16,BF16}          -> unsupported
```

Configuration-declared dtype must be compared against the required primary policy, not an unrelated extra tensor.

### Do not materialize unused tensors

Continue to parse and validate every selected Safetensors tensor so artifact evidence, offsets, duplicate detection, and whole-file identity remain trustworthy.

Only tensors required by the reviewed Llama schema may be converted, transferred, or inserted into the Candle variable map. Supported-but-unused tensors must be skipped without device allocation.

Do not retain the existing “materialize extras as loading headroom” behavior or its tests.

### One normal payload pass

Remove preparation-time per-tensor payload hashing and the second per-tensor digest pass.

Use the verified whole-file identity supplied by the artifact boundary. Materialization should stream each shard payload in deterministic offset order once:

- update the whole-file verifier as bytes are consumed;
- materialize required tensor ranges;
- stream/skip unused ranges without constructing tensors;
- compare the completed content identity before model publication;
- return an ownership-bearing failed materialization if verification fails after resources were created.

A structural test must prove the normal path does not perform two complete payload reads. Do not use a timing assertion as a substitute.

### Exact memory planning

Rebuild final and loading-peak calculations around the actual single-pass algorithm.

The plan must account for:

- retained required execution tensors;
- at most the actual source staging buffer(s);
- conversion overlap only where conversion occurs;
- host-to-device transfer overlap only where it occurs;
- Candle map/model handle ownership that remains live during construction;
- sequence cache bytes separately;
- no device or execution headroom for skipped extras;
- checked arithmetic for every component and aggregate.

The footprint calculator and materializer must share one reviewed representation of tensor disposition so they cannot drift into separate algorithms.

### Bound metadata amplification

Keep the existing shard/header bounds and add explicit, tested limits for:

- total tensor count;
- per-shard tensor count if useful;
- tensor-name byte length;
- tensor rank;
- total shape-dimension storage;
- aggregate metadata allocations.

Reject before large allocation or device initialization. Keep limits documented and justified rather than using arbitrary enormous values.

## Structural refactor

`loader.rs` is now a subsystem and must stop pretending to be one file.

Refactor it into cohesive private modules, for example:

```text
loader/
    mod.rs
    inspection.rs
    schema.rs
    policy.rs
    footprint.rs
    prepared.rs
    materialize.rs
```

Choose names based on actual responsibilities. Avoid micro-modules that only wrap one trivial function.

Remove duplicated cleanup calls such as repeated `shards.clear()`. Remove `too_many_lines` suppressions that are only needed because responsibilities were not separated.

Investigate `VarBuilder::from_tensors(self.final_tensors.clone(), ...)` and avoid unnecessary map duplication or handle retention if Candle offers a safe ownership-preserving construction path. If shallow cloning is genuinely required by Candle, document the ownership reason precisely and test that final accounting still matches live required tensors.

## Cleanup and failure invariants

Preserve or improve all Phase 12 guarantees:

- no model publication before content identity, descriptor, device, execution scalar, footprint, and lifecycle validation;
- every partially constructed host/device tensor has exactly one owner;
- cleanup failure leaves that owner valid for retry;
- device synchronization occurs at the correct cleanup and handoff boundaries;
- cleanup is idempotent and releases accounting exactly once;
- unsupported dtype/layout failure occurs before device materialization whenever possible;
- panic conversion remains contained without losing native ownership.

Do not replace explicit cleanup with reliance on `Drop`.

## Required test matrix

Retain homogeneous and mixed CPU/CUDA coverage and add at least:

- required F32 plus unused F16;
- required F32 plus unused BF16;
- declaration F32 with unused F16/BF16;
- unused tensor is parsed and included in observed evidence but never materialized or transferred;
- exact peak excludes skipped-extra device ownership;
- one-pass reader/byte-count proof;
- whole-file mismatch discovered after partial materialization enters cleanup ownership;
- malformed offset gaps/overlaps, duplicates, excessive tensor count/name/rank, overflow;
- deterministic shard ordering;
- required tensor disposition and footprint plan cannot diverge;
- CPU/CUDA cleanup retry and exhaustion with mixed required tensors.

Do not depend on network access or an external checkpoint for correctness.

## Validation

Run focused adapter and native E0 tests for default CPU and CUDA compile graphs. Execute CUDA hardware tests only on the accepted guarded machine; do not fabricate hardware evidence elsewhere.

At minimum:

```text
cargo test --locked -p candle-backend
cargo test --locked -p inference-runtime --test native_backend_generation
cargo test --locked -p inference-runtime --test fault_injection
cargo check --locked -p candle-backend -p inference-runtime --features cuda
cargo test --locked -p candle-backend -p inference-runtime --features cuda --no-run
cargo clippy --locked -p candle-backend -p inference-runtime --all-targets -- -D warnings
cargo clippy --locked -p candle-backend -p inference-runtime --all-targets --features cuda -- -D warnings
cargo fmt --all -- --check
git diff --check
```

## Finish

Commit one coherent loader-subsystem change. Record exact supported layout policy and any real hardware evidence separately. Do not push.
