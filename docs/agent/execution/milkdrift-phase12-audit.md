# Milkdrift Phase 12 implementation audit

**Audit date:** 2026-08-09  
**Repository basis:** uploaded repository archive  
**Audited commit:** `181a069ce81525e9c144fe8de051ced8e3c0b9d7`  
**Audited tree:** `310e437c0729f51fe6c0ba3dcb5fbf9f1935a80f`  
**Comparison base:** `a28008a369214e26522cf027977b67292962d058`  
**Disposition:** **conditional acceptance; corrective work required before Phase 12 should be treated as fully closed**

## Executive verdict

Phase 12 is not a superficial dtype relaxation and it is not a failed implementation. The most important architecture is strong:

- source inspection precedes requested-device initialization;
- a load preparation binds the exact source, configuration, device, execution scalar, final footprint, and loading peak;
- E0 reserves the loading peak before materialization;
- `load_prepared` consumes the accepted preparation without replanning;
- materialization failure returns one sole cleanup owner;
- failed cleanup remains quarantined, retryable, and accounted;
- no model is published until the loaded model agrees with the accepted plan;
- E1 does not implement Candle's per-tensor conversion policy;
- Slint remains a thin reference host;
- no project-authored unsafe block was added.

Those are high-quality systems changes and should be retained.

However, Phase 12 should not yet be accepted as a prestige-quality closure. The audit found two policy/truth defects, an unmet diagnostic requirement, incomplete native fault-injection coverage, a significant unmeasured load-time cost, and a CI layout that exhausted the hosted runner. The strongest defect is that the documented configuration-declaration policy is not what the code implements.

The appropriate conclusion is:

> Keep the Phase 12 architecture. Do not revert it. Apply one focused Phase 12 corrective follow-up, rerun clean shared CI and CUDA CI, and only then mark the phase fully accepted.

### Overall assessment

| Area | Assessment |
|---|---|
| Prepared-load and ownership architecture | Excellent |
| E0 admission, quarantine, and accounting | Excellent |
| Per-tensor inspection and conversion implementation | Strong |
| Memory-planning model | Strong and conservative |
| E1/frontend layering | Strong |
| Configuration truth and primary-scalar policy | Needs correction |
| Failure diagnostics | Below the original acceptance requirement |
| Native adapter fault injection | Good baseline, incomplete |
| Load-time efficiency | Potentially expensive and not measured |
| Shared CI robustness | Failed because of target/disk lifecycle |
| Final Phase 12 closure claim | Premature |

A reasonable aggregate rating is **approximately 7.5/10**: an unusually strong foundation with several important closure defects. The ownership architecture itself is closer to 9/10; policy truth, diagnostics, and acceptance discipline pull down the total.

## Audit method and limitations

The audit covered:

- the full `a28008a..181a069` change set;
- `domain-contracts` model/load contracts;
- Candle source inspection, policy, footprint calculation, materialization, and cleanup;
- E0 admission, failed-load ownership, cleanup retry, snapshots, and tests;
- Hugging Face configuration parsing;
- E1 load admission and receipt validation;
- redb schema migration;
- Slint changes;
- CPU/CUDA test inventories;
- benchmark/evidence schemas;
- `quality.yml` and `cuda-hardware.yml`;
- ADR-0020 and canonical status documentation.

The archive is clean and internally identifies the exact commit and tree reported by the closure agent. The Phase 12 comparison contains **97 changed files, 11,425 insertions, and 3,329 deletions**.

This audit environment does not contain `cargo` or `rustc`, so I could not independently rerun the Rust suites. Static test inventories match the reported focused counts closely, and the supplied GitHub log establishes that the shared runner reached the portable-target step after the preceding canonical native gate. Local CUDA execution statements remain operator/agent evidence rather than evidence independently reproduced in this audit.

## CI failure diagnosis

### Finding

The supplied failure is an **infrastructure/resource failure**, not evidence that `task-graph`, `libm`, or the `no_std` contracts fail to compile.

The first decisive error is:

```text
No space left on device (os error 28)
```

The later `rust-lld` bus error occurred while another build process was linking after the filesystem had already been exhausted. It is best treated as a secondary failure consistent with the same resource exhaustion, not as an independent LLVM or project-code defect.

### Why this workflow ran out of space

`.github/workflows/quality.yml` currently does the following in one hosted job:

1. runs the complete workspace gate using
   `CARGO_TARGET_DIR=${{ runner.temp }}/llm-app-clean-quality-target`;
2. leaves that target directory in place;
3. optionally runs scheduled workspace-wide nursery Clippy in the default repository target;
4. runs the WASM check in the default repository `target`;
5. runs the embedded check in that same default repository `target`.

The canonical gate itself performs workspace `check`, `test`, `clippy`, `doc`, and benchmark compilation. It therefore leaves a large native target in the runner temp filesystem. The portable steps then create a second target tree at:

```text
/home/runner/work/milkdrift/milkdrift/target
```

Both trees consume the same hosted-runner disk. The failure path in the log is exactly that second tree.

Phase 12 did not introduce this workflow structure—the last changes to `quality.yml` predate Phase 12—but the larger compile surface likely exposed the latent problem. The uploaded source tree itself is only about 16 MiB, its `.git` directory about 11 MiB, and it contains no committed `target` tree or large model fixture. Repository bloat is not the cause.

### What the failed run still proves

In the current workflow, the portable check appears after `cargo xtask verify`. Reaching the portable step means that the earlier canonical native gate completed successfully in that same Rust job. It does **not** prove that the portable step passed, nor does it say anything about an independently running policy job unless its result is inspected separately.

### Required CI correction

The best fix is to give portability its own fresh job:

```yaml
portable:
  name: Portable domain crates
  runs-on: ubuntu-24.04
  env:
    CARGO_INCREMENTAL: "0"
  steps:
    - uses: actions/checkout@<pinned-sha>
    - name: Check WebAssembly portability
      env:
        CARGO_TARGET_DIR: ${{ runner.temp }}/milkdrift-wasm-target
      run: |
        set -eu
        rm -rf "${CARGO_TARGET_DIR}"
        cargo check --locked --target wasm32-unknown-unknown --lib \
          -p domain-contracts -p tokenization -p context-planner -p sampling -p task-graph
        rm -rf "${CARGO_TARGET_DIR}"
    - name: Check embedded portability
      env:
        CARGO_TARGET_DIR: ${{ runner.temp }}/milkdrift-thumb-target
      run: |
        set -eu
        rm -rf "${CARGO_TARGET_DIR}"
        cargo check --locked --target thumbv7em-none-eabihf --lib \
          -p domain-contracts -p tokenization -p context-planner -p sampling -p task-graph
        rm -rf "${CARGO_TARGET_DIR}"
```

Also add an `if: always()` cleanup step to the native Rust job:

```yaml
- name: Remove canonical target
  if: always()
  run: rm -rf "${RUNNER_TEMP}/milkdrift-clean-quality-target"
```

The scheduled nursery lint should use its own job or isolated target. Add `df -h` and `du -sh` diagnostics before and after major gates so a future increase is observable before ENOSPC.

Until this is merged and a clean remote run passes, documentation should say:

> Phase 12 implementation and local validation complete; shared portability CI acceptance pending runner-target isolation and rerun.

## What was implemented particularly well

### 1. The generic load contract is materially better

`crates/domain/domain-contracts/src/backend.rs` introduces:

- `PreparedLoad` as an exact backend-owned preparation;
- retryable explicit cleanup after failed materialization;
- `FailedLoad<P>` retaining the primary failure and sole cleanup owner;
- `ModelLoader::prepare_load` and `ModelLoader::load_prepared`.

This is substantially better than `plan_load` followed by an unrelated `load` that may inspect or plan again. The API now makes the exact accepted plan consumable and gives failure ownership a first-class type.

`LoadPlan` also binds:

- the accepted `LoadConfiguration`;
- the complete descriptor;
- execution scalar;
- final accounted footprint;
- loading-peak footprint.

The contract is portable and does not leak Candle or Safetensors types into E0.

### 2. Source inspection is pre-device, bounded, deterministic, and source-bound

`candle-backend/src/loader.rs`:

- caps selected shards at 256;
- caps aggregate Safetensors header bytes at 100,000,000;
- preopens files before header allocation;
- parses duplicate JSON keys explicitly rather than accepting serde's usual last-write behavior;
- validates Safetensors metadata and exact payload bounds;
- sorts shards and tensor metadata deterministically;
- detects cross-shard duplicate tensor identities;
- validates the complete required Llama tensor schema and shapes;
- hashes each tensor payload during preparation;
- retains open file handles;
- rechecks file length and per-tensor digest during materialization.

The retained file plus digest design is strong protection against path replacement and same-inode mutation between preparation and load.

### 3. Per-tensor ownership during fallible work is carefully staged

The prepared Candle owner retains:

- inspected shards;
- selected device;
- final tensors already materialized;
- pending source tensor;
- pending converted host tensor;
- pending device tensor;
- a constructed model if final synchronization fails.

The implementation deliberately places transfer endpoints and the constructed model into the owner before later fallible validation/synchronization. That is exactly the correct direction for explicit partial-load ownership.

Cleanup synchronizes first. If synchronization fails, all owners remain intact for another attempt. On success it clears all tensors, files, config, and device state and becomes idempotently complete.

### 4. E0 reserves peak ownership and fails closed

`inference-runtime/src/runtime/admission.rs`:

- creates one exact `LoadConfiguration` from remaining aggregate budget;
- obtains one prepared load;
- validates that the plan is bound to that exact configuration;
- verifies loading footprint contains final ownership component-wise;
- reserves loading peak before materialization;
- immediately attempts cleanup after materialization failure;
- quarantines `PendingModelOwner::FailedLoad` if cleanup fails;
- keeps the full loading peak reserved during quarantine;
- validates handle, descriptor, actual device, execution scalar, and final accounted footprint before model publication;
- transitions reservation from loading peak to final footprint only after commit.

This is coherent and directly aligned with Milkdrift's strongest values: explicit ownership, observable failure, no false release, and backend contract verification beyond trait conformance.

### 5. E1 and Slint stayed thin

E1 retains configuration-declared metadata as application evidence but does not choose the per-tensor primary scalar or conversion path. Detailed tensor inventories remain below E1. Public loaded state reports actual execution scalar/device from the verified receipt.

The redb `LAM1` migration is also clean:

- version 2 stores an optional declaration;
- exact version 1 remains readable and maps its former mandatory scalar to `Some(...)`;
- observed tensor sets and execution facts are not persisted as immutable catalogue facts.

Slint changes are minimal and presentation-only.

### 6. The deterministic test surface is substantial

Static test inventory confirms:

- 20 Candle CPU tests;
- 6 Candle CUDA tests, of which 4 are intentionally ignored hardware tests;
- 4 native E0 tests, one ignored hardware test;
- 32 E0 fault-injection tests;
- 79 benchmark test declarations, with feature/target configuration explaining the normal reported count of 78.

The CPU Candle suite includes a real prepared-source mutation test that causes failure after earlier tensors can already be materialized, then verifies cleanup. The generic E0 suite covers immediate cleanup, retained cleanup, retry, exhaustion, reservation accounting, and contract violations. This is far beyond a happy-path dtype test.

## Release-blocking and high-priority findings

## P1 — Unsupported configuration declarations are silently erased or replaced

### Evidence

In `crates/adapters/hf-hub/src/lib.rs:309-347`, configuration is parsed as:

```rust
dtype: Option<String>,
torch_dtype: Option<String>,
```

Then the result is selected with:

```rust
configuration.dtype.as_deref().and_then(parse_scalar_type)
    .or_else(|| configuration.torch_dtype.as_deref().and_then(parse_scalar_type))
```

An unknown present `dtype` therefore becomes `None`. If legacy `torch_dtype` is recognized, it replaces the unknown modern field. The test at roughly lines 546-560 explicitly expects:

```json
{"dtype":"float8_e4m3fn","torch_dtype":"float16"}
```

to resolve as F16.

This contradicts ADR-0020 and the project documentation, which say that an unsupported present declaration is rejected. It also means `None` cannot distinguish:

- declaration absent;
- declaration explicitly null;
- declaration present but unsupported.

There is a second truth problem at the direct adapter boundary. `CandleLlamaSource::new` accepts `Option<ScalarType>` supplied by its caller. The Candle loader parses the exact config as `LlamaConfig` but does not independently derive the declaration from those bytes. A direct caller can therefore omit or fabricate “configuration-declared” evidence independently from the configuration file.

### Impact

- Public metadata can claim absence when the immutable config contains an unsupported declaration.
- Documentation and runtime behavior disagree.
- A future crate consumer can inject declaration evidence that did not come from the source config.
- E1's cross-checks only prove that the same already-normalized value passed through its layers; they do not prove the value reflects exact config bytes.

### Required correction

Introduce a tri-state parse result internally:

```text
Absent
Recognized(ScalarType)
Unsupported
```

Recommended rules:

- both fields absent/null: `Absent`;
- modern `dtype` recognized: use it;
- modern `dtype` absent/null and legacy `torch_dtype` recognized: use legacy;
- either selected present field unsupported: reject;
- recognized modern and stale legacy disagreement may follow an explicitly documented precedence rule, but it must be tested and documented.

The Candle adapter should parse the declaration from the exact retained config bytes used to create its `Config`. The best public API removes declaration from `CandleLlamaSource` or names any caller-supplied value as a separate expected/override policy rather than source fact.

## P1 — Absent mixed-dtype declarations are resolved by an unjustified lossy inference

### Evidence

`select_primary_scalar` currently maps:

```text
{F16,F32}   -> F16
{BF16,F32}  -> BF16
```

and accepts `declaration == None`.

The CPU test `absent_declaration_uses_inferred_mixed_primary` codifies this behavior.

The original monolithic Phase 12 specification did **not** authorize this. Its accepted matrix was explicitly:

```text
declared F16 + observed {F16,F32}
declared BF16 + observed {BF16,F32}
```

and it required rejection when the observed set did not contain the declared primary. The implementation/ADR changed this from “declared primary” to “infer lower precision from the set.”

### Why the inference is not truthful

A set of categories contains no frequency or semantic-role information. From `{F16,F32}` alone, the adapter cannot know whether the repository is:

- an F16 model with a small number of F32 normalization/auxiliary tensors; or
- an F32 model with one incidental F16 tensor.

The current rule selects F16 in both cases and downcasts every F32 tensor. The loader validates required tensor names and shapes, but it does not restrict F32 to a reviewed set of auxiliary tensor roles.

This is not a memory-safety defect. It is a compatibility and numerical-policy defect: the runtime can silently choose a lossy execution policy from ambiguous evidence.

### Required correction

For the Phase 12 boundary, require a recognized matching declaration for mixed sets. Homogeneous sets may safely allow absence because their primary is unambiguous.

A future explicit operator policy could authorize a mixed primary without config metadata, but that should be a named load policy/override, not silently inferred source truth.

An alternative lossless policy would be to execute absent `{F16,F32}` or `{BF16,F32}` as F32, but that changes memory and execution semantics and deserves a separate reviewed decision. The strict declaration requirement is the cleanest Phase 12 correction.

## P1 — Phase 12 closure status is ahead of shared acceptance

The code and docs repeatedly say “Phase 12 complete.” The archive was then pushed and shared CI failed before portable acceptance.

The failure is not a product-code regression, but completion should still distinguish:

```text
implementation complete
local validation complete
shared CI accepted
CUDA workflow accepted
external checkpoint evidence accepted
```

The current documentation is generally careful about local versus remote CUDA evidence, but it does not yet reflect the failed shared portability run. Correct the workflow, rerun it, and update the status. Until then the phase is conditionally accepted rather than closed.

## P2 — The original fixed-size offending-tensor diagnostic was not implemented

The original Phase 12 specification required unsupported dtype failures to identify the offending location without arbitrary paths or unbounded names. It suggested a bounded diagnostic containing:

- shard ordinal;
- tensor ordinal;
- stable tensor-name hash;
- observed dtype classification;
- stable adapter code.

The implementation often returns bare `LoadError::UnsupportedFormat`, including unsupported Safetensors dtype and unsupported scalar-set combinations. `BackendFailure` only carries backend, kind, and code; it does not identify which tensor caused rejection.

This matters operationally. With a multi-shard checkpoint, an operator needs to know whether one auxiliary tensor is unsupported or whether the entire repository uses another format.

Add a portable fixed-size load diagnostic or backend failure detail that preserves the required bounded location information. Keep file paths and full names out of the portable domain.

## P2 — Native Candle fault injection does not cover the complete real failure path

The generic E0 fault-injection suite is excellent. Actual Candle coverage is less complete:

- a payload-mutation test drives the real materialization path and verifies successful cleanup;
- a unit test manually constructs `CandleLlamaPreparedLoad`, injects one cleanup synchronization failure through a global atomic, and verifies retry/idempotence.

What is missing is one deterministic test that:

1. uses the real Candle materialization path;
2. fails at a selected stage after a selected tensor ordinal;
3. then fails cleanup/synchronization;
4. proves the exact real owner populated by materialization remains retained;
5. succeeds on retry and restores accounting.

The original prompt requested test-only fault points after inspection, shard load, conversion, transfer, insertion, model construction, and synchronization. The implementation did not build that narrow staged fault mechanism.

Add an internal `#[cfg(test)]` fault plan keyed by stage and tensor ordinal. It must not become a public production hook. At minimum cover CPU conversion, CUDA transfer/sync, model construction, and cleanup retry with the actual prepared owner.

## P2 — Load preparation performs full payload hashing and can read the model multiple times

`inspect_weight_shards` hashes every tensor payload during inspection. Materialization then rereads every tensor and hashes it again.

Consequences:

- a normal E0 load reads the complete weight payload approximately twice;
- a caller that invokes public `inspect()` and later loads can read it approximately three times;
- `inspect()` no longer behaves like a metadata-only operation despite its public description;
- CUDA materialization also synchronizes after each transferred tensor.

For a multi-gigabyte model this may dominate startup and storage traffic. The integrity guarantee is valuable, but the cost has not been measured and no Phase 12 load-time baseline was accepted.

Required follow-up before broad real-model claims:

- benchmark preparation and load separately on representative model sizes;
- document cold-cache and warm-cache semantics;
- consider keeping public `inspect()` header-only;
- consider making strict payload binding a preparation/integrity policy;
- consider shard-level or bounded-batch transfer synchronization only after proving ownership and failure semantics remain correct.

Do not optimize this blindly. First measure it. But do not describe the current path as optimized merely because its memory plan is exact.

## P2 — The loader needs an internal module split

`candle-backend/src/loader.rs` is 1,746 lines and combines:

- config parsing and validation;
- header parsing;
- duplicate detection;
- payload integrity;
- Llama tensor-schema validation;
- dtype policy;
- footprint calculation;
- prepared ownership;
- materialization;
- cleanup;
- error translation;
- unit tests.

The transaction should remain locally auditable, but this is now a god module. Split it **within the same crate**, not into microcrates. A sensible structure is:

```text
loader/mod.rs          public loader transaction and orchestration
loader/inspection.rs   retained files, headers, tensor inventory, schema validation
loader/policy.rs       declaration/observed/primary/execution decisions
loader/footprint.rs    exact checked memory math
loader/prepared.rs     materialization owner and cleanup
```

Keep the critical state machine visible and avoid generic abstraction for its own sake.

## P2 — Architecture identity is assumed rather than explicitly verified

The loader sets `ModelArchitecture::Llama` after deserializing into Candle's `LlamaConfig` and validating Llama-style tensor names/shapes. Static review found no explicit check of immutable config fields such as `model_type` or `architectures`.

That may admit another architecture with a sufficiently similar config and tensor layout and then misclassify it as Llama. Mistral-family checkpoints are the obvious class requiring caution because their tensor vocabulary is closely related while execution semantics may differ.

The project documentation explicitly claims non-Llama architectures are unsupported. Add an explicit reviewed architecture-identity check and regression test so “unsupported” means rejected rather than merely likely to fail structural validation.

## Lower-priority findings

### P3 — Header bounds do not separately limit tensor count, name length, or rank

The 100 MB aggregate header cap prevents unbounded input, but parsed JSON can amplify substantially into strings, maps, vectors, and shapes. Add explicit inspection limits for tensor count, individual name bytes, total name bytes, and maximum rank, or reduce the aggregate scratch bound to a quantity justified by real Llama repositories.

### P3 — Benchmark preparation can warm future load measurements

The external benchmark observer calls `prepare_load` to capture plan facts before a timed E1 load. Preparation now scans and hashes the full model and may initialize the selected CUDA device. This warms page cache and driver state.

No schema-4 performance run was accepted, so existing evidence has not been falsely rewritten. Before collecting one, define whether the measurement is cold or warm and avoid an observer step that changes the state being measured.

### P3 — CUDA workflow test-count enforcement is brittle

The CUDA workflow requires exactly four ignored Candle CUDA tests. This is intentionally fail-closed, but an unrelated new ignored test will break the hardware matrix. Prefer a central reviewed hardware-test manifest or an xtask validation that enumerates approved hardware tests by name/category.

### P3 — Closure hygiene checked the working tree, not the committed phase diff

`git diff --check a28008a..HEAD` reports trailing whitespace in `docs/agent/execution/analyzer.md`. A clean working-tree `git diff --check` cannot detect already committed whitespace.

The closure gate should check the accepted base range or scan all tracked text files, not only uncommitted changes.

## Verification of the closure agent's statements

| Statement | Audit result |
|---|---|
| Commit `181a069...` and tree `310e437...` | Confirmed |
| Branch/worktree clean | Confirmed in uploaded archive |
| Nothing pushed at time of agent report | Not inferable from archive; archive's `origin/main` now points to the commit |
| Five accepted observed layouts implemented | Confirmed in current policy code |
| F16+BF16 and unsupported dtypes rejected | Confirmed statically |
| Declaration may be absent or match inferred primary | Implemented, but the absent mixed policy is a questionable deviation from the original specification |
| Unsupported present config declaration rejected | **Not true for the Hugging Face parser** |
| Prepared load consumes exact preparation without replanning | Confirmed |
| Failed load retains sole cleanup owner | Confirmed |
| E0 reserves loading peak and quarantines failed cleanup | Confirmed |
| No model published before receipt/plan validation | Confirmed |
| 20 CPU adapter tests declared | Confirmed |
| 3 ordinary native E0 tests | Confirmed by inventory: 4 declared, 1 ignored |
| 32 fault-injection tests declared | Confirmed |
| 78 normal benchmark tests | Plausible/consistent with 79 declarations and feature/target cfg selection |
| Local CPU/portable/CUDA executions passed | Cannot independently reproduce in this environment |
| Shared GitHub portability passed | False for the supplied run; runner exhausted disk |
| CUDA workflow hardened and exact | Confirmed statically; remote Phase 12 run still absent |
| No external mixed checkpoint | Confirmed and honestly documented |
| E0 backend-neutral / E1 thin / Slint thin | Confirmed |

### Later evidence note — 2026-08-10

The table above records the audit's evidence at the time it examined the uploaded archive; it is not rewritten retroactively. Official GitHub records later established that both Phase 12 workflows had run on closure commit `181a069ce81525e9c144fe8de051ced8e3c0b9d7`:

- self-hosted CUDA [run 31281013243](https://github.com/hartolit/milkdrift/actions/runs/31281013243) completed successfully on the exact RTX 5070 Ti job;
- hosted Quality [run 31281013257](https://github.com/hartolit/milkdrift/actions/runs/31281013257) failed after its canonical native work succeeded. Workspace-wide release bench artifacts left roughly 49 MiB free, and the following WASM check created a separate root target and failed with `No space left on device`; the later linker bus error was consequential.

The CUDA run is remote Phase 12 hardware evidence for `181a069`. The Quality failure is CI-infrastructure evidence, not a product/WASM failure, and neither run proves later amendment trees.

## Prompt-quality retrospective

The ownership-based three-prompt segmentation was correct. It avoided repeatedly loading the entire repository context and kept the main transactions together:

- contracts + Candle + E0;
- Hub + E1 + persistence + Slint;
- validation + evidence + project truth.

The prompts were nevertheless below the project's desired standard in several important ways. This is a prompt-design miss, not merely an execution miss.

### What the prompts failed to preserve

1. **The original strict declared-primary matrix.**  
   The core prompt asked the agent to make absent/unsupported policy explicit but did not restate that mixed layouts originally required a declared primary. This allowed ADR-0020 to authorize lossy inference from an ambiguous set.

2. **The fixed-size diagnostic acceptance criterion.**  
   The segmented core prompt did not retain the original shard/tensor/hash/dtype diagnostic requirement strongly enough, and it disappeared.

3. **Actual adapter-stage fault injection.**  
   The prompt mentioned fault injection, but its acceptance language allowed generic E0 fakes and manually constructed prepared state to substitute for failures injected into the real Candle path.

4. **Inspection cost and API semantics.**  
   The prompt rewarded exact source binding without requiring `inspect()` to remain metadata-oriented or requiring a load-cost measurement. The agent chose a very conservative full-payload double-read design without evidence of acceptable startup cost.

5. **Unsupported configuration tests.**  
   The application-integration prompt explicitly required recognized and absent declarations, but did not require unknown-present, conflicting-field, or exact-config-versus-source mismatch tests. The parser bug follows directly from that omission.

6. **Hosted-runner target lifecycle.**  
   The validation prompt required clean targets and broad gates but did not require disk diagnostics, cleanup of the canonical target, or isolation of portable targets. It therefore did not expose the workflow's resource layout before closure.

The lesson is not to return to a monolithic prompt. Keep domain-oriented segmentation, but carry a compact, immutable acceptance ledger from the source specification into every segment. Each segment should state which acceptance clauses it owns and which it must verify at handoff.

## Required corrective sequence

Do this as focused Phase 12 maintenance, not as a new product phase.

### Correction set A — source truth and scalar policy

- implement declaration tri-state parsing;
- reject unsupported present declarations;
- derive declaration from exact config bytes in the Candle adapter;
- remove or rename caller-injected declaration evidence;
- require a matching recognized declaration for mixed sets;
- add absent, unknown, legacy, modern/legacy precedence, mismatch, and direct-source truth tests;
- update ADR-0020 and canonical docs to one exact rule.

### Correction set B — diagnostics and actual fault ownership

- implement bounded offending-tensor diagnostics;
- add test-only staged Candle fault injection;
- prove real CPU conversion failure + cleanup failure/retry;
- prove real CUDA transfer/synchronization failure + cleanup failure/retry on the accepted hardware row;
- preserve E0 peak accounting throughout.

### Correction set C — shared CI acceptance

- split portable checks into a fresh job;
- isolate and remove target directories;
- disable incremental compilation in CI;
- move scheduled nursery Clippy to another job/target;
- add disk-usage diagnostics;
- rerun the complete quality workflow;
- run the updated self-hosted CUDA workflow;
- only then restore “Phase 12 complete” status.

### Correction set D — maintainability and measured performance

This can follow the release-blocking corrections but should precede broad checkpoint support:

- split `loader.rs` internally;
- add explicit Llama architecture identity validation;
- tighten header metadata limits;
- measure header inspection, payload binding, materialization, transfer, and synchronization separately;
- define cold/warm benchmark semantics;
- optimize only with retained ownership and memory evidence intact.

## Acceptance criteria for final greenlight

Phase 12 should receive an unconditional greenlight when all of the following are true:

- unknown present config declarations fail explicitly;
- Candle derives declaration evidence from exact config bytes;
- mixed sets do not silently infer a lower-precision primary from absence;
- unsupported tensor errors include bounded location diagnostics;
- real Candle partial-load + cleanup-failure tests pass;
- shared quality CI passes on fresh isolated targets;
- self-hosted Phase 12 CUDA workflow passes and has a recorded run ID;
- docs distinguish deterministic fixture support from external mixed-checkpoint evidence;
- the loader's startup cost is at least measured and documented, even if conservative behavior is retained.

## Final recommendation

The agents delivered a strong architectural implementation but an overconfident closure. The right response is neither to discard Phase 12 nor to proceed as though it is fully complete.

Keep commits `58490fe`, `1251069`, and `181a069` as the foundation. Apply the focused corrections above, update the completion status, and then return to the workflow/workspace/authority program. Phase 12 is close enough that a disciplined correction is preferable to a redesign, but the declaration and mixed-primary issues should be fixed before more real model repositories are trusted.
