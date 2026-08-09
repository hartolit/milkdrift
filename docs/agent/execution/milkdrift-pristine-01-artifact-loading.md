# Work package: pristine artifact identity and Candle tensor loading

## Mission

Bring the local artifact-to-Candle loading subsystem to a durable production-quality design. Correct the Phase 12 scalar-policy flaw, stop materializing tensors that the supported Llama implementation does not consume, remove avoidable full-model I/O on trusted immutable artifacts, strengthen configuration-declaration truth, bound hostile metadata, and refactor the monolithic loader into auditable modules.

Do not make the smallest patch that satisfies existing tests. Implement the architecture that should still be correct when models contain many shards and tens or hundreds of gigabytes of weights.

## Read before editing

Read the repository context map and the documents that own this area, including at minimum:

- `README.md`
- `docs/vision.md`
- `docs/agent/README.md`
- `docs/agent/persona.md`
- `docs/project/architecture.md`
- `docs/project/candle-backend.md`
- `docs/project/implementation-status.md`
- `docs/project/validation.md`
- `docs/agent/decisions/0010-verify-backend-contracts-at-e0.md`
- `docs/agent/decisions/0013-candle-only-local-execution.md`
- `docs/agent/decisions/0019-explicit-cuda-execution-foundation.md`
- `docs/agent/decisions/0020-transactional-prepared-model-loading.md`
- the original Phase 12 specification and the three executed Phase 12 prompts

Then inspect the current source and tests in:

- `crates/adapters/candle-backend`
- `crates/adapters/hf-hub`
- the committed Candle fixture/provenance files
- any artifact identity types used by `application-runtime` or `runtime-benchmarks`

Treat the current Phase 12 behavior as evidence to preserve where correct, not as an API that must preserve a flawed implementation.

## Owned area

Primary ownership:

- `crates/adapters/candle-backend/**`
- `crates/adapters/hf-hub/**`
- adapter-owned fixture generation and provenance needed to test this subsystem

You may make narrowly necessary changes to portable load contracts or downstream call sites to keep the workspace compiling, but do not redesign E0/E1 ownership here. Record any generic-contract concern for the next work package and keep adapter-specific inventory private.

## Required architectural outcomes

### 1. Separate four scalar facts

The implementation must distinguish:

1. configuration-declared scalar evidence;
2. the complete scalar-category set observed across all structurally valid tensors in the artifact;
3. the scalar-category set of tensors required by the supported Llama execution schema;
4. the execution scalar selected for the requested device.

The execution-primary scalar must be derived only from the **required execution tensors**. An unused auxiliary tensor must never downcast required F32 weights, select BF16/F16 execution, or make an otherwise compatible declaration appear contradictory.

Required regression cases include at least:

- required `{F32}` plus unused F16 extra;
- required `{F32}` plus unused BF16 extra;
- required `{F32}` plus both F16 and BF16 extras;
- declared F32 with each of those layouts;
- required mixed `{F16,F32}` plus unrelated extras;
- required mixed `{BF16,F32}` plus unrelated extras;
- a genuine required F16/BF16 mixture, which remains rejected unless a new explicit policy and evidence justify support.

The complete observed set remains truthful artifact evidence. It must not be repurposed as the execution-policy input.

### 2. Treat configuration declaration presence honestly

The Hugging Face configuration parser must not collapse these different facts into `None`:

- field absent or null;
- recognized declaration;
- present but unsupported declaration;
- two present declarations that contradict one another;
- malformed declaration value.

The modern `dtype` field must not silently fall back to `torch_dtype` merely because its value is unknown. Define and implement a clear precedence/conflict policy. A present unsupported or contradictory declaration must produce an explicit compatibility failure under the current “absent or matches required primary” rule; it must never masquerade as absent metadata.

Keep raw vendor strings inside the artifact adapter. Cross stable boundaries only with project-owned status/error vocabulary. Do not persist unsupported vendor strings as user preferences.

### 3. Materialize only tensors consumed by the model

All selected shards and headers must still be structurally validated, duplicate-aware, bounds-checked, and included in artifact observation. However, only tensors required by the supported Llama schema may be converted, transferred, inserted into the Candle load map, or retained during model construction.

Unused tensors must not:

- consume execution-device memory;
- contribute to final weight ownership;
- create transient device-headroom requirements;
- select execution precision;
- cause rejection merely because their dtype is not executable, provided their Safetensors representation is structurally understood and they are not required by the selected architecture.

Map known non-executed Safetensors categories into the existing portable scalar-category vocabulary where possible. If a category cannot be represented safely, make the representation gap explicit rather than conflating it with a required-tensor execution failure.

Add tests that prove unused tensors are inspected but never materialized or transferred. Use deterministic instrumentation/test doubles rather than timing as the assertion.

### 4. Build a source-identity path that scales

Preserve the Phase 12 guarantee that the accepted preparation cannot silently materialize different required bytes. Improve the implementation so a trusted immutable artifact does not require an avoidable baseline pass over every tensor followed by another full payload pass on every load.

Implement a durable source-identity model with two honest paths:

- **verified immutable artifact path:** the artifact resolver supplies a cryptographically bound shard identity from a source whose immutability and identity semantics are actually proven. Materialization verifies that identity while reading each shard sequentially and materializes only required tensor ranges. Do not trust a cache filename, symlink target, ETag, inode, mtime, or provider convention without checking the actual library/cache contract and documenting the proof.
- **unverified or mutable local path:** retain a safe fallback that establishes content identity before admission and detects required-payload changes before publication. It may stage into a content-addressed immutable cache or perform a verification pass, but it must not weaken TOCTOU protection to save I/O.

The preferred final algorithm for verified shards is one sequential shard pass that:

- verifies the expected cryptographic identity;
- validates the retained header/source identity;
- skips allocation/conversion for unused ranges;
- reads required tensor payloads into bounded aligned staging;
- converts/transfers each required tensor independently;
- leaves one exact partial-load owner on failure.

Avoid per-tensor random seeks and repeated hashing when a verified whole-shard identity is available. Repeated loads of the same immutable artifact should reuse valid identity evidence rather than recomputing it needlessly.

Do not use unsafe memory mapping. Do not retain an entire large model payload in host memory merely to avoid a second read.

If the installed Hugging Face cache API cannot provide a trustworthy expected digest, implement and persist project-owned content identity in the adapter/cache layer rather than pretending a weak identity is cryptographic proof.

### 5. Make memory planning match the actual algorithm

Recalculate final and loading-peak footprints around selective required-tensor materialization and the chosen source-verification algorithm.

Requirements:

- exact checked arithmetic;
- final ownership contains only required execution tensors and cache ownership;
- loading peak reflects only simultaneously live deterministic tensor/staging allocations;
- no device headroom for ignored extras;
- CPU and CUDA formulas correspond to real ordering of source staging, cast tensors, transfer tensors, retained tensors, and synchronization;
- deterministic non-tensor buffers and parsed metadata are either included in an appropriate bounded resource model or strictly bounded by separate inspection limits and documented as such;
- a model must not be rejected for device capacity needed only by a tensor that will not be materialized.

Update ADR-0020 if the algorithm or formulas change. Do not preserve obsolete formulas for documentary continuity.

### 6. Bound metadata structurally, not only by a coarse byte ceiling

The current aggregate header ceiling permits substantial allocation amplification. Add explicit checked limits suitable for the supported Llama path, covering at least:

- shard count;
- aggregate and per-shard header bytes;
- tensor count;
- tensor-name bytes/length;
- tensor rank and shape dimensions;
- metadata entry count or ignored metadata growth where relevant;
- aggregate owned inspection metadata.

Derive and document limits from the supported model family and realistic headroom rather than arbitrary enormous constants. Rejections must be deterministic and occur before large unbounded allocations or device initialization.

Use streaming/bounded parsing where it materially reduces peak metadata ownership. Do not replace one unbounded allocation with several less obvious ones.

### 7. Refactor the loader as a subsystem

`candle-backend/src/loader.rs` currently owns too many independent responsibilities. Refactor it into cohesive internal modules, for example along these responsibilities:

- Safetensors bounded inspection and source manifest;
- required Llama tensor schema validation;
- scalar/declaration compatibility policy;
- source identity and verification;
- footprint simulation;
- prepared-load ownership and materialization;
- loader trait integration.

The exact file names are up to you. The outcome must be:

- no generic “utils” dumping ground;
- no duplicated tensor iteration/policy formulas;
- private adapter-specific inventory;
- small functions with explicit invariants;
- tests located beside the policy/algorithm they validate where appropriate;
- no broad Clippy suppressions used to excuse avoidable structure.

Keep the public adapter surface compact and stable. Do not create a new workspace crate for this internal split.

### 8. Preserve explicit partial-load ownership

The prepared owner must remain the sole owner of every completed and in-flight resource after a materialization failure. Cleanup remains retryable and idempotent. A failed cleanup must preserve all handles needed for another attempt.

Maintain CPU and CUDA synchronization before explicit release. Do not add adapter-local `mem::forget`; terminal process-lifetime retention remains an E0 decision.

## Testing requirements

Expand deterministic adapter tests to cover:

- all required-vs-observed scalar cases above;
- strict declaration status and conflicts;
- supported unused integer/boolean/other extras if the Safetensors crate can parse them;
- required unsupported dtypes;
- selective materialization and absence of extra device transfers;
- exact CPU/CUDA final and peak formulas after the algorithm change;
- immutable verified source fast path;
- mutable/unverified fallback mutation detection;
- header mutation, payload mutation, truncation, duplicate tensors, overlapping/gapped offsets, shard reorder, and cross-shard duplicates;
- metadata structural limits and allocation failures;
- partial materialization failure at each ownership stage and idempotent cleanup;
- no device initialization before format/schema rejection.

Use small generated fixtures. Do not commit downloaded third-party checkpoints.

## Validation

Run targeted formatting, checks, tests, Clippy, and rustdoc for the owned adapters and any changed portable contracts. Run both default CPU and CUDA compilation. Execute the deterministic CUDA adapter matrix on available hardware. Do not claim hardware evidence that was not executed.

Do not run the entire expensive canonical gate if it only repeats unrelated packages; the final infrastructure and closure agents own clean whole-tree validation. Still leave all changed packages warning-free and test-complete.

## Completion

Before committing:

- inspect the diff for duplicated policy, stale formulas, temporary compatibility branches, TODOs, and broad lint suppressions;
- update the component documentation and ADR-0020 where the algorithm changed;
- ensure the working tree contains no generated targets, caches, downloaded models, or reports.

Create one coherent commit and do not push. Report:

- commit SHA and tree SHA;
- the final required/observed/declaration/execution policy;
- the final source-identity algorithm for verified and mutable sources;
- the final materialization and footprint algorithm;
- exact tests and hardware validation executed;
- any generic runtime concern that the next work package must inspect.
