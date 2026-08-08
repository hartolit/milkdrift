# Phase 12 — Per-tensor Safetensors scalar compatibility

**Status:** Superseded historical planning input
**Current execution authority:** [Phase 12 execution guide](milkdrift-phase12-execution-guide.md) and its [Segment 1](milkdrift-phase12-core-loader-runtime.md), [Segment 2](milkdrift-phase12-application-artifact-integration.md), and [Segment 3](milkdrift-phase12-validation-project-truth.md) specifications
**Historical scope:** Broader compatibility inside the existing unquantized Candle Llama Safetensors path

This is the original monolithic Phase 12 plan. It is retained without a wholesale rewrite so its planning rationale and discarded assumptions remain reviewable, but it is not active doctrine or an execution prompt. Phase 12 was activated and split by stable ownership boundaries; current status, commands, evidence requirements, and closure decisions come from the segmented guide/specifications and [current execution context](current.md).

Where this historical plan conflicts with the segmented authority or implemented contracts, follow the segmented authority and current code.

## Why this is a separate phase

The current loader derives one source scalar from model configuration and
requires every tensor in every Safetensors shard to match it. Supporting
repositories containing a primary F16 or BF16 weight dtype plus supported F32
auxiliary tensors is not a one-line relaxation.

A correct implementation requires coordinated changes to:

- source metadata vocabulary;
- Safetensors header inspection;
- exact per-tensor conversion policy;
- conversion-aware host/device memory planning;
- ownership of partially loaded resources;
- E0 load evidence;
- E1 and Slint presentation;
- deterministic fixtures;
- CPU and CUDA regression coverage;
- external model evidence;
- documentation and support claims.

For that reason, actual mixed-dtype support is large enough to be Phase 12.

## Objective

Broaden the existing unquantized Llama path so that reviewed mixed floating
tensor dtypes can be loaded and converted to one selected execution scalar
without weakening explicit ownership, bounded admission, failure cleanup,
device truth, or current CPU/CUDA behavior.

The initial compatibility matrix is intentionally narrow:

```text
configuration-declared F32
    -> observed tensor set {F32}

configuration-declared F16
    -> observed tensor set {F16}
       or {F16, F32}

configuration-declared BF16
    -> observed tensor set {BF16}
       or {BF16, F32}
```

Initially reject:

- F16 and BF16 mixed together;
- a tensor set that does not contain the declared primary scalar;
- FP8, integer, boolean, quantized, or unknown tensor dtypes;
- malformed shapes or arithmetic overflow;
- model architectures or tensor layouts outside the current Llama path.

The execution-scalar policy remains:

```text
declared F32
    CPU  -> F32
    CUDA -> F32

declared F16
    CPU  -> F16
    CUDA -> F16

declared BF16
    CPU  -> F32
    supported CUDA -> BF16
```

Every accepted tensor is converted independently to the selected execution
scalar when required.

## Product boundary

Phase 12 does not add:

- GGUF or another serialization format;
- quantized model loading;
- another local engine;
- another model architecture;
- generic NVIDIA support;
- Metal;
- automatic CPU fallback;
- GPU-side sampling;
- multi-GPU;
- generalized chat templates;
- multiple resident models;
- a generic arbitrary-repository benchmark CLI;
- project-owned unsafe code.

Direct completion remains the compatibility floor. A model loading
successfully does not imply chat support.

## Required architectural principles

1. Configuration-declared source scalar, observed per-tensor scalar set, and
   execution scalar are separate facts.
2. Every shard header is inspected before model tensors are loaded.
3. Unsupported tensor dtypes fail before partial device residency.
4. Memory planning uses checked per-tensor arithmetic.
5. Partial-load failure publishes no model receipt.
6. Native resources remain explicitly owned until cleanup is proven.
7. No hidden `mem::forget` fallback is permitted.
8. E0 remains the transactional admission and resident-model owner.
9. E1 remains one concrete frontend-neutral façade.
10. Slint remains presentation-only.
11. Ordinary tests remain download-free.
12. Hardware and external-model claims require observed execution.

## Activation prerequisites

Before changing production behavior, record all of the following:

- latest clean commit and tree;
- successful current CPU quality workflow;
- successful current self-hosted CUDA workflow;
- one reproduced mixed-dtype failure on a pinned immutable Llama repository;
- exact repository, immutable commit, declared license metadata, architecture,
  observed tensor dtype set, and current failure;
- confirmation that the model does not require a new architecture or quantized
  path;
- explicit project-owner approval to activate Phase 12.

The external model must remain in an ignored cache. Do not commit its weights.

## Required reading

Read only the ownership areas relevant to this phase:

1. `docs/agent/persona.md`
2. `docs/rules.md`
3. `docs/conventions.md`
4. `docs/project/architecture.md`
5. `docs/project/dependency-policy.md`
6. `docs/project/candle-backend.md`
7. `docs/project/inference-runtime.md`
8. `docs/project/application-runtime.md`
9. `docs/project/desktop-runtime.md`
10. `docs/project/validation.md`
11. `docs/project/performance.md`
12. ADR-0006, ADR-0013, ADR-0018, and ADR-0019
13. current execution plan and current context
14. source, model, backend, memory, and error contracts in `domain-contracts`
15. Candle source inspection, planning, loading, model, device, and failure code
16. current project-authored fixture generator and CPU/CUDA adapter tests
17. E0 admission, pending cleanup, receipts, snapshots, and fault injection
18. E1 model resolution, load admission, retained cleanup, state, and tests
19. Slint loaded-model summaries and tests
20. external runtime observer, report schema, and exact profile identity
21. self-hosted CUDA workflow.

Do not ingest unrelated future research tracks.

## Starting state

Begin from a clean accepted `main`:

```bash
git status --short --untracked-files=all
git rev-parse HEAD
git rev-parse 'HEAD^{tree}'
git log -10 --oneline

export CARGO_TARGET_DIR="$(git rev-parse --show-toplevel)/target"

cargo xtask verify
```

Use one Cargo process at a time.

Do not:

- run `cargo clean`;
- use package-local target directories;
- run multiple model or CUDA processes concurrently;
- download models into the source tree;
- commit generated external evidence;
- print repository-wide diffs into the agent conversation.

## Work package 12.1 — Decision record and scalar vocabulary

Create a new ADR before changing the public contract.

The ADR must decide:

- exact initial mixed-dtype matrix;
- meaning of configuration-declared source scalar;
- meaning and representation of observed tensor scalar set;
- execution-scalar selection;
- partial-load ownership strategy;
- memory-planning model;
- failure diagnostic shape;
- evidence required before support is claimed.

### Domain representation

Replace ambiguous scalar semantics with explicit names.

A suitable direction is:

```rust
pub struct ModelMetadata {
    pub architecture: ModelArchitecture,
    pub declared_source_scalar_type: ScalarType,
    pub observed_source_scalar_types: SourceScalarSet,
    pub quantization: QuantizationFormat,
    pub vocabulary_size: u32,
    pub context_length: u32,
}
```

Exact names may differ, but the singular `ModelMetadata::scalar_type` must not
remain ambiguous after Phase 12.

Use one portable, fixed-size, allocation-free observed-set type. For example:

```rust
#[repr(transparent)]
pub struct SourceScalarSet(u8);
```

It should represent at least:

```text
F32
F16
BF16
```

Required operations:

- empty construction;
- insertion during inspection;
- `contains`;
- subset validation;
- deterministic display/translation outside the domain;
- equality and copying;
- no heap allocation;
- `no_std` compatibility.

Do not use a `Vec`, `HashSet`, string, Candle dtype, or backend-owned type in
the domain contract.

### Error diagnostics

Unsupported tensor dtype failures must identify the offending location without
carrying arbitrary paths or unbounded tensor names.

Use a fixed-size structured diagnostic such as:

```text
shard ordinal
tensor ordinal
stable tensor-name hash
observed scalar/dtype classification
stable adapter failure code
```

Exact structure is an ADR decision.

Requirements:

- `Copy`;
- allocation-free;
- portable;
- no filesystem path;
- no access token;
- no arbitrary user-controlled string;
- deterministic across runs.

Do not use `DefaultHasher` for a persisted or externally compared hash because
its stability is not a project contract.

## Work package 12.2 — Safetensors header inspection

Inspect every Safetensors shard before execution-device initialization and
before loading tensor payloads.

Create a coherent adapter-private inspection result, for example:

```text
InspectedSafetensorsLayout
    tensor count
    observed source scalar set
    total tensor elements
    total source tensor bytes
    largest source shard bytes
    largest tensor element count
    largest execution-sized tensor candidate
    per-shard conversion summary
```

Exact structure may differ.

Inspection must:

1. parse every shard header safely;
2. enumerate every tensor;
3. validate dtype against the initial reviewed matrix;
4. validate shape element-count arithmetic with checked operations;
5. detect duplicate tensor names across shards before loading;
6. collect the observed scalar set;
7. compute exact tensor payload bytes;
8. retain no mapped tensor payload or execution device;
9. reject malformed metadata deterministically;
10. produce no model receipt or partial residency.

The configuration-declared scalar remains metadata and compatibility input. It
must not be treated as evidence that every tensor has that dtype.

### Compatibility policy

Centralize one adapter-owned compatibility function.

Initially accept only:

```text
declared F32 + observed {F32}
declared F16 + observed {F16}
declared F16 + observed {F16, F32}
declared BF16 + observed {BF16}
declared BF16 + observed {BF16, F32}
```

Reject every other set explicitly.

Do not duplicate this matrix in E0, E1, Slint, and benchmark code.

E0 and E1 validate coherent receipts; the Candle adapter owns the file-format
compatibility policy.

## Work package 12.3 — Exact conversion-aware memory planning

Replace repository-wide file-size scaling with checked planning derived from
inspected tensor element counts and actual source dtypes.

The plan must distinguish:

```text
source artifact bytes
final execution-weight bytes
host conversion/load working bytes
device conversion/load working bytes
sequence cache bytes
rope/sequence working bytes
```

At minimum, compute:

- final execution bytes for every tensor;
- total final execution weight bytes;
- largest simultaneously retained source shard;
- largest conversion temporary required by the chosen load pipeline;
- host peak during loading;
- device peak during loading;
- final CPU or CUDA accounting;
- cache bytes per token from execution scalar.

Do not divide a repository-wide byte total by one source scalar width.

Use checked arithmetic throughout. Overflow is an explicit load failure.

### Choose one auditable load pipeline

Prefer a pipeline whose transient ownership can be planned and released
clearly. A suitable candidate is:

```text
inspect all headers
  -> initialize selected device
  -> load one shard into CPU-owned tensors
  -> consume tensors one by one
  -> convert each accepted tensor to execution dtype on CPU when needed
  -> transfer the final tensor to the selected execution device
  -> drop source and conversion temporaries promptly
  -> continue with the next tensor/shard
  -> construct the Llama model from final execution tensors
```

Do not adopt this exact sequence blindly. Verify it against the pinned Candle
and Safetensors APIs, then document the selected pipeline and its peak-memory
formula in the ADR.

Avoid loading mixed source tensors directly to CUDA when that would make
source-dtype device residency and conversion peaks unaccounted.

The existing conservative policy may continue reserving load headroom for the
model's lifetime if changing reservation phases would broaden the task too
far. Do not silently claim transient accounting is final physical residency.

## Work package 12.4 — Transactional partial-load ownership

This is a mandatory design gate.

The current loader returns either a completed model or a `LoadError`. A failure
before a completed model is returned means E0 has no loaded-model handle to
quarantine.

Phase 12 must prove that partially loaded resources cannot escape unowned.

### Required investigation

Determine, from the exact pinned Candle implementation and executed tests:

- whether tensor conversion and transfer can leave asynchronous CUDA work;
- what synchronization is required before tensor/device drop;
- whether failed model construction can leave pending native work;
- whether safe Rust drop alone proves the project's cleanup contract.

### Acceptable designs

#### Design A — Adapter-local explicit transaction

Use only when every failure path can synchronously and observably release all
partial resources before returning.

The transaction must:

- own the selected device;
- own loaded source tensors;
- own converted tensors;
- own final tensors pending model construction;
- synchronize at required boundaries;
- expose explicit abort success/failure;
- disarm only after a complete `CandleLlamaModel` is constructed.

A `Drop` implementation may be a final safety net, but it must not be the only
cleanup protocol.

#### Design B — Retained failed-load ownership through E0

Use when cleanup can fail and must be retried or quarantined.

Extend the lower-layer contract narrowly so a failed load can return:

```text
primary load error
retained partial-load cleanup owner
accounted footprint
cleanup retry capability
```

E0 must then:

- retain the owner;
- reserve its footprint;
- expose pending/exhausted cleanup state;
- retry boundedly;
- publish no model receipt;
- release accounting only after cleanup success or proven process termination.

Avoid introducing a generic service framework or dynamic dispatch.

### Forbidden design

Do not hide a terminal failed-load owner with:

```rust
std::mem::forget(...)
```

unless the retention is explicitly surfaced, accounted, documented, tested,
and consistent with the project's terminal-cleanup policy.

An invisible adapter-local leak is not acceptable.

### Failure injection

Add test-only fault points without public production hooks for failure:

- after complete header inspection;
- after the first shard is loaded;
- after one dtype conversion;
- after one host-to-device transfer;
- before duplicate insertion;
- during model construction;
- during partial-load synchronization;
- during partial-load cleanup retry, if Design B is selected.

Every test must prove:

- no load receipt is published;
- no resident model is visible;
- owned accounting is either zero or explicitly retained;
- successful retry releases all retained accounting;
- primary and cleanup failures remain distinguishable.

## Work package 12.5 — Per-tensor load and conversion

Implement per-tensor conversion only after work packages 12.1–12.4 are
accepted.

For each tensor:

1. verify its actual loaded dtype matches preflight inspection;
2. verify the tensor identity and shape still match the inspected header;
3. accept only the centralized reviewed source-dtype matrix;
4. convert to selected execution dtype when needed;
5. transfer to the selected execution device through the chosen pipeline;
6. insert exactly one final tensor by name;
7. reject duplicates;
8. release source and conversion temporaries as early as possible.

Do not silently reinterpret bytes.

Do not cast unsupported integer, FP8, boolean, or quantized values to floating
point.

`VarBuilder` and `Llama::load` must receive only final tensors in the selected
execution dtype.

The loaded model must report:

- declared source scalar;
- observed source scalar set;
- actual execution scalar;
- actual execution device;
- accounted footprint.

## Work package 12.6 — E0, E1, Slint, and persistence

### E0

Preserve transactional admission.

E0 verifies before publication:

- handle;
- descriptor;
- declared source scalar;
- observed source scalar set;
- execution scalar;
- requested versus actual device;
- planned versus actual accounted footprint;
- lifecycle transition.

A mismatch follows the existing explicit cleanup path.

Receipts and snapshots carry the new truthful descriptor facts.

### E1

Keep one concrete `ApplicationRuntime`.

A resolved model may expose the configuration-declared source scalar because
tensor headers have not yet been inspected by E0.

A loaded model exposes:

```text
declared source scalar
observed tensor scalar set
execution scalar
actual loaded device
```

E1 must not reproduce Candle's complete compatibility matrix. It validates
that:

- immutable resolution and receipt identify the same model;
- declared scalar matches resolution metadata;
- the observed set is non-empty and supported by application vocabulary;
- execution scalar and device are represented;
- receipt facts remain internally coherent.

Incompatible evidence enters the existing private cleanup path.

### Slint

Present loaded facts truthfully, for example:

```text
Declared source scalar: BF16
Observed tensor scalars: BF16, F32
Execution scalar: BF16
Actual device: CUDA 0
```

Resolved summaries show only configuration-declared source scalar.

Do not parse labels for semantics.

### Persistence

Observed tensor scalar set and execution scalar are runtime/load evidence, not
user preferences.

Do not persist either merely for display.

Retain the existing settings migration unless a real model-catalogue contract
requires change. Any schema bump must be justified independently and tested.

## Work package 12.7 — Deterministic fixtures

Do not commit multiple copies of a model blob.

Extend the project-owned fixture tooling so ordinary tests can generate or
materialize tiny deterministic variants beneath a temporary or ignored target
directory.

Required fixture cases:

1. homogeneous F32;
2. homogeneous F16;
3. homogeneous BF16;
4. declared F16 with observed F16/F32;
5. declared BF16 with observed BF16/F32;
6. unsupported tensor dtype;
7. disallowed F16/BF16 mixture;
8. duplicate tensor identity across shards;
9. malformed/overflowing shape metadata where safely constructible.

Prefer deriving variants from one tiny project-authored tensor specification
rather than tracking several nearly identical binary fixtures.

Record fixture provenance and deterministic hashes where the current fixture
policy requires it.

No external model license enters the repository.

## Work package 12.8 — CPU and CUDA validation

### Ordinary CPU tests

Prove:

- exact observed scalar sets;
- exact compatibility matrix;
- homogeneous regressions;
- F16/F32 tensors execute as F16 under declared F16;
- BF16/F32 tensors execute as F32 on CPU under declared BF16;
- final vocabulary logits remain host F32;
- per-tensor memory planning;
- conversion expansion and transient peaks;
- unsupported dtype rejection before device initialization;
- partial-load failure ownership and cleanup;
- no model publication on failure;
- duplicate rejection;
- unload and zero accounting;
- no behavior regression for the accepted TinyLlama path.

### CUDA tests

Keep tests ignored and explicitly gated by:

```text
MILKDRIFT_CUDA_TEST=1
```

On the accepted RTX 5070 Ti matrix prove:

- F16/F32 source tensors execute as F16;
- BF16/F32 source tensors execute as BF16;
- CPU remains explicitly usable in a CUDA-enabled build;
- actual device remains CUDA ordinal 0 when selected;
- adapter load, generation, synchronization, unload, and drop succeed;
- hosted E0 returns zero model/request/workspace/cleanup accounting;
- E1 reports declared, observed, execution, and device facts;
- no fallback occurs;
- partial-load CUDA failure cleanup is explicit.

Add the new fixture tests to the existing guarded CUDA hardware workflow.
Do not add network access.

## Work package 12.9 — External mixed-dtype model evidence

Select one representative immutable unquantized Llama repository that:

- genuinely contains the accepted mixed tensor set;
- uses the existing Llama architecture path;
- has reviewable license metadata;
- does not require a new quantized format;
- can run direct completion without generalized chat support.

Do not accept a model merely because it downloads or reaches model
construction.

Extend the existing external runner rather than creating another binary.

Use one fixed reviewed model-profile enum, for example:

```text
TinyLlama accepted profile
Mixed-dtype Llama Phase 12 profile
```

Do not accept arbitrary repository/revision command arguments.

The mixed-dtype profile must:

- pin exact repository and immutable revision;
- record declared and observed source scalars;
- use direct completion only unless an exact chat profile is separately
  reviewed;
- load on CPU;
- generate;
- release;
- unload;
- shut down;
- execute on the accepted CUDA matrix;
- record truthful accounting and physical observations;
- omit generated text and token IDs;
- leave raw reports under ignored root `target`.

No timing threshold is required.

Phase 12 cannot close without one successful CPU and one successful CUDA
external-model lifecycle on the exact accepted tree.

## Work package 12.10 — Evidence schema and documentation

Increment the external report schema when serialized fields change.

Record separately:

```text
declared_source_scalar
observed_source_scalars
execution_scalar
requested_device
selected_e1_device
actual_loaded_e0_device
accounted_footprint
physical memory observations
```

Do not reinterpret schema 3 silently.

Update canonical owners:

- new ADR for the design;
- `docs/project/candle-backend.md`;
- `docs/project/inference-runtime.md`;
- `docs/project/application-runtime.md`;
- `docs/project/desktop-runtime.md`;
- `docs/project/implementation-status.md`;
- `docs/project/validation.md`;
- `docs/project/performance.md` only for accepted external evidence;
- execution plan;
- current context;
- history.

When acceptance is complete, remove the unsupported mixed-dtype statement and
replace it with the exact supported matrix.

Do not claim arbitrary mixed dtypes or generic Llama compatibility.

## Suggested commit boundaries

Use a small number of coherent commits:

1. **Define per-tensor scalar and planning contracts**
   - ADR;
   - domain vocabulary;
   - header inspection;
   - checked memory plan;
   - deterministic inspection tests.

2. **Implement transactional mixed-dtype loading**
   - partial-load ownership;
   - per-tensor conversion;
   - adapter and E0 fault tests;
   - CPU fixtures.

3. **Propagate evidence and close CPU/CUDA acceptance**
   - E1 and Slint;
   - report schema;
   - CUDA fixtures/workflow;
   - external model evidence;
   - final documentation.

Do not fragment work into many cosmetic commits.

Do not start the next commit while the current ownership boundary is failing
its focused checks.

## Historical validation commands

> This command list is preserved as original planning input. Use the current [validation procedure](../../project/validation.md) and Segment 3 specification for active execution; do not infer a pass from this list.

### CPU and repository gates

```bash
cargo fmt --all -- --check

cargo check --locked -p domain-contracts --all-targets
cargo test --locked -p domain-contracts
cargo clippy --locked -p domain-contracts --all-targets -- -D warnings

cargo check --locked -p candle-backend --all-targets
cargo test --locked -p candle-backend
cargo clippy --locked -p candle-backend --all-targets -- -D warnings

cargo check --locked -p inference-runtime --all-targets
cargo test --locked -p inference-runtime
cargo clippy --locked -p inference-runtime --all-targets -- -D warnings

cargo check --locked -p application-runtime --all-targets
cargo test --locked -p application-runtime
cargo clippy --locked -p application-runtime --all-targets -- -D warnings

cargo check --locked -p desktop-slint --all-targets
cargo test --locked -p desktop-slint
cargo clippy --locked -p desktop-slint --all-targets -- -D warnings

cargo check --locked -p runtime-benchmarks --all-targets
cargo test --locked -p runtime-benchmarks
cargo clippy --locked -p runtime-benchmarks --all-targets -- -D warnings

cargo xtask verify
```

### Portable domain checks

```bash
cargo check --locked \
  --target wasm32-unknown-unknown \
  --lib \
  -p domain-contracts

cargo check --locked \
  --target thumbv7em-none-eabihf \
  --lib \
  -p domain-contracts
```

### CUDA compile and hardware tests

```bash
export CUDA_COMPUTE_CAP=120
export MILKDRIFT_CUDA_TEST=1

cargo check --locked \
  -p candle-backend \
  -p inference-runtime \
  -p application-runtime \
  -p desktop-slint \
  -p runtime-benchmarks \
  --all-targets \
  --features cuda

cargo clippy --locked \
  -p candle-backend \
  -p inference-runtime \
  -p application-runtime \
  -p desktop-slint \
  -p runtime-benchmarks \
  --all-targets \
  --features cuda \
  -- -D warnings
```

Run the exact mixed-dtype adapter, hosted E0, and E1 ignored tests
sequentially with one test thread.

### Final policy and documentation

```bash
cargo deny --workspace --locked check \
  advisories bans licenses sources

lychee --config lychee.toml --offline '**/*.md'

git diff --check
git status --short --untracked-files=all
```

Push only after local CPU and CUDA checks pass.

Observe both:

- normal shared CPU quality workflow;
- self-hosted CUDA hardware workflow.

## Historical Phase 12 acceptance criteria

The following was the monolithic plan's proposed closure checklist. It is not the active checklist; the segmented execution guide/specifications own current closure, including honest closure of deterministic compatibility when no suitable immutable license-reviewed external mixed checkpoint can be established.

The historical plan required all of the following:

- declared source scalar, observed tensor scalar set, and execution scalar are
  distinct in contracts and evidence;
- every Safetensors header is inspected before tensor loading;
- only the reviewed initial mixed-dtype matrix is accepted;
- every accepted tensor is converted independently to execution dtype;
- unsupported dtypes fail before partial device residency;
- memory plans use exact checked per-tensor arithmetic;
- conversion and load peaks are conservatively accounted;
- partial-load failures publish no model;
- partial resources are either released or explicitly retained and accounted;
- no hidden `mem::forget` fallback exists;
- homogeneous F32, F16, and BF16 behavior remains intact;
- mixed F16/F32 works on CPU and accepted CUDA;
- mixed BF16/F32 works as F32 on CPU and BF16 on accepted CUDA;
- E0, E1, Slint, and schema-4 evidence report truthful scalar and device facts;
- ordinary tests remain download-free;
- the guarded CUDA workflow passes;
- one pinned external mixed-dtype Llama model completes CPU and CUDA lifecycle
  evidence;
- chat support is not broadened merely because loading succeeds;
- documentation claims only the exact executed compatibility boundary;
- no unrelated engine, format, device, or public framework was added.

## Historical proposed closure status

The monolithic plan proposed the following wording. Do not use it as current status; see [current execution context](current.md):

```text
Phase 10 complete.
Phase 11 complete for the executed CPU + Linux CUDA matrix.
Post-Phase 11 quality maintenance complete.
Phase 12 complete for the reviewed homogeneous and mixed F16/F32 or BF16/F32
unquantized Llama Safetensors matrix.
No subsequent product phase is active.
```

That proposed wording and its external-checkpoint prerequisite are superseded by the segmented closure rules.

## Historical proposed final report

Report:

1. activation commit/tree and final commit/tree;
2. pinned external mixed-dtype repository and immutable revision;
3. final declared/observed/execution scalar contracts;
4. exact accepted compatibility matrix;
5. header-inspection and memory-plan design;
6. partial-load ownership design and failure behavior;
7. fixture strategy and provenance;
8. CPU validation;
9. CUDA validation;
10. external CPU/CUDA lifecycle evidence;
11. workflow run results;
12. final documentation/support boundary;
13. any intentionally deferred dtype combinations.

Do not paste full diffs, generated model output, raw tokens, credentials, cache
paths, or complete command logs.
