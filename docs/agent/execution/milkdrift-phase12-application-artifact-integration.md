# Milkdrift Phase 12 — artifact and application integration

This is one ownership segment of Phase 12, not a new project phase. Work directly in the local codebase in Zed and implement the changes. Do not return a patch or code block.

Start from the reviewed commit produced by the core loader/runtime segment. Do not independently redesign its lower transaction unless an integration test proves a specific contract defect.

## Purpose

Integrate the new declared, observed, and execution scalar semantics through immutable artifact resolution and the current application kit without leaking Safetensors internals upward or expanding the frontend.

The future Milkdrift workflow runtime must remain lower and more neutral than `application-runtime`. This segment updates the existing E1 vertical slice; it does not make E1 the universal workflow, model, or plugin API.

## Read before editing

Read:

- `docs/vision.md`
- `docs/rules.md`
- `docs/project/architecture.md`
- `docs/project/application-runtime.md`
- `docs/project/candle-backend.md`
- `docs/agent/execution/analyzer.md`
- `docs/agent/execution/milkdrift-phase12-per-tensor-safetensors-compatibility.md`
- the Segment 1 completion report and current diff/commit

Then inspect:

- `crates/adapters/hf-hub`
- `crates/runtime/application-runtime`
- `crates/adapters/redb-storage`
- `crates/apps/desktop-slint` only where public API adaptation is required

## Scope

You may change:

- Hugging Face configuration/index parsing and immutable artifact metadata;
- E1 resolved-model, loaded-model, load-admission, compatibility, event, error, and state vocabulary;
- persistence records and migrations only if their existing meaning becomes false;
- Slint presenter/model adaptation required to compile and preserve the thin frontend;
- focused tests in these areas;
- Rustdoc local to these public application facts.

Do not change:

- the lower per-tensor load algorithm except for a narrowly proven integration bug;
- E0 scheduling, generation, cleanup, or memory policy without such a proven defect;
- workflow, workspace, plugin, provider, peer, or control-center architecture;
- the user interface to add tensor inspection, advanced settings, or a model-management product surface;
- chat behavior unrelated to correcting scalar/source semantics.

## Required integration model

### Artifact resolution

Hugging Face resolution must continue to pin the requested revision to an immutable commit and resolve the required config, tokenizer, index, and Safetensors shards.

Configuration fields such as `dtype` or `torch_dtype` are **configuration-declared scalar metadata**. Preserve them as optional evidence when recognized. Do not describe them as a verified dtype shared by every tensor.

Do not duplicate the Candle adapter's complete Safetensors header parser in `hf-hub`. Artifact resolution should identify immutable files and configuration metadata; backend preparation should own format-specific execution inspection unless a shared lower abstraction is already justified by another consumer.

### E1 public facts

Review every public type and method that currently uses terms such as `source_scalar_type`, `scalar_type`, or “validated source scalar.” Rename or reshape them where their meaning is now misleading.

E1 should expose only stable application-level facts with real consumers. A sound default is:

- resolved model: immutable artifact identity and optional configuration-declared scalar metadata;
- loaded model: actual execution scalar, actual device, and any compact format-neutral source-layout classification that is genuinely needed by callers;
- detailed per-tensor inventory: adapter-private diagnostic/evidence, not ordinary E1 state.

Do not add a large public tensor-layout DTO merely to display that Phase 12 exists. If no E1 consumer needs the observed layout, allow E0 and the adapter to validate it privately while E1 reports only that loading succeeded under the accepted compatibility policy.

### Load validation

E1 must not reproduce Candle's per-device or per-tensor conversion policy.

Update load-admission and receipt validation so that it relies on facts already verified by E0:

- ticket and model identity;
- immutable selected artifacts;
- requested and actual execution device;
- actual execution scalar;
- architecture, format, vocabulary, context, and capability compatibility;
- admitted/reserved footprint within the application policy;
- any stable prepared-source identity exported by the lower contract.

Remove equality checks that assume configuration-declared scalar metadata must equal every observed tensor dtype or must equal the execution scalar.

Retain explicit failure for unsupported or incoherent lower receipts. Do not weaken receipt validation simply to make a mixed fixture pass.

### Error and event truth

Normalize new lower failures into stable application categories without exposing Candle, Safetensors, filesystem, or driver error types through the E1 public API.

Keep these distinctions observable where they matter:

- resolution failure;
- unsupported artifact/layout;
- memory admission failure;
- load/materialization failure;
- retained cleanup pending or exhausted;
- incompatible receipt;
- successful loaded execution facts.

Do not report a retained cleanup failure as an ordinary unsupported model with no owned resources.

### Persistence

Audit what the current model catalogue persists.

Do not persist a singular “source scalar” if its documented meaning is now “the dtype stored by all tensors.” Choose the smallest honest alternative:

- reinterpret and rename it as optional configuration-declared metadata with a schema migration;
- replace it with a compact stable classification that has a real reload/use-case consumer;
- or remove it from new records while retaining old-version decoding.

Do not persist cache paths or a full per-tensor inventory. Preserve backward reading of existing schema versions and add deterministic migration tests. Do not bump a schema merely because a Rust field was renamed if the persisted semantics remain truthful.

### Thin frontend

The Slint application remains a reference host.

Adapt it only to:

- compile against corrected names/types;
- stop displaying a misleading source/execution scalar statement;
- preserve existing load, generation, cancellation, unload, and shutdown behavior.

Do not add new panels, tensor tables, workflow controls, or compatibility settings in this segment.

## Cleanup within the owned area

Consolidate duplicated scalar conversions, source-fact checks, or compatibility predicates inside the owned crates when the new model makes them redundant.

Keep each rule in one owner:

- Hub config metadata parsing in `hf-hub`;
- per-tensor artifact execution inspection in `candle-backend`;
- backend plan/result verification in E0;
- application compatibility and presentation facts in E1;
- persistence encoding in `redb-storage`;
- display formatting in the frontend.

Do not undertake an unrelated `application-runtime` rearchitecture. Preserve its current public use cases while preventing further scope growth.

## Required tests

Use deterministic local fixtures and fake events. Ordinary tests must not require network access.

Add or update tests for:

- recognized and absent configuration-declared scalar metadata;
- immutable resolution retaining declared metadata without claiming tensor homogeneity;
- a mixed-dtype prepared/load receipt accepted by E1 when E0 has verified it;
- E1 not deriving execution scalar from configuration metadata or device selection;
- incompatible execution facts still rejected and routed through retained cleanup;
- selected versus actual device validation;
- memory-policy validation against the lower accepted footprint;
- old persistence records remaining readable;
- new persistence semantics round-tripping exactly;
- direct completion and exact TinyLlama chat behavior remaining unchanged;
- conversation and output behavior remaining unchanged;
- Slint presenter/model tests compiling with no new backend knowledge.

## Validation

Run targeted gates:

```text
cargo fmt --all -- --check
cargo check --locked -p hf-hub-adapter -p redb-storage -p application-runtime -p desktop-slint --all-targets
cargo test --locked -p hf-hub-adapter -p redb-storage -p application-runtime -p desktop-slint
cargo clippy --locked -p hf-hub-adapter -p redb-storage -p application-runtime -p desktop-slint --all-targets -- -D warnings
cargo xtask architecture
cargo xtask hygiene
```

Also compile the complete opt-in CUDA feature chain through `application-runtime` and `desktop-slint`. Do not claim hardware execution from compilation alone.

## Completion report

Finish with a concise report containing:

- the final E1 vocabulary for declared source metadata and actual execution facts;
- which former scalar-equality assumptions were removed or relocated;
- whether persistence changed and how older records remain readable;
- confirmation that detailed per-tensor data remained outside E1;
- confirmation that the frontend gained no new product responsibility;
- tests and commands executed;
- any narrowly required lower-contract fix made after Segment 1.

Create one coherent commit only after the targeted gates pass, unless the repository's current execution instructions require a different commit policy.
