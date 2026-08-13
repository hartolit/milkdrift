# Current implementation status

**Status date:** 2026-08-13

```text
Phase 12 and the artifact-loading, runtime-ownership, and application-boundary amendments are implemented.
The post-closure foundation repair rejects ambiguous mixed precision, separates failed-load typestate, and corrects sequence reservation to simultaneous lifetimes.
The exact-tree foundation closure passes its focused CPU, canonical native, portable, dependency-policy, and offline-link matrices locally; hosted Quality and current CUDA hardware acceptance remain separate pending evidence.
The orchestration-boundary repair makes task-graph generic and allocation-free and moves corrective semantics into validated reference-template data; its focused host, portable, architecture, and hygiene matrix passes locally.
The CI resource-topology repair gives each hosted verification profile an independently bounded metadata-owned plan; its local component, portability, workflow-policy, and shell/YAML validation passes, while redesigned remote runs remain pending.
No later product phase is active, and workflow/workspace direction is not yet a ratified program.
```

This page is the sole product-level support and current validation-state owner. It separates implementation, local working-tree validation, clean committed-tree evidence, compile-only coverage, hardware execution, remote Actions, and external product evidence. Procedures live in [validation](validation.md), measurements in [performance evidence](performance.md), and chronology in [execution history](../agent/execution/history.md).

## Implemented product boundary

Milkdrift remains workflow-first, but the implemented product is still the local-inference foundation and optional reference application kit:

```text
Slint or another native host
  -> application-runtime (E1 reference services)
     -> immutable Hugging Face resolution + tokenizer + redb
     -> one hosted inference worker
        -> inference-runtime (E0)
           -> candle-backend
              -> unquantized Llama Safetensors
              -> mandatory/default CPU
                 or explicit feature-gated CUDA ordinal 0
```

General workflow definitions/runs, durable context workspaces, plugin execution, provider/peer targets, browser transport, and a control center are not current product paths.

| Capability | Current implementation and evidence boundary |
|---|---|
| Local engine | Candle only. |
| Artifact source | Hugging Face revision resolved to an immutable commit. Selected LFS shards carry exact provider SHA-256/length identity; non-LFS and arbitrary local paths use explicit mutable-source fallback semantics. |
| Format/architecture | Unquantized Safetensors through the current Llama compatibility path. All selected structure is inspected; only required Llama tensors are materialized. |
| Required scalar layouts | Exactly `{F32}`, `{F16}`, `{F16,F32}`, `{BF16}`, and `{BF16,F32}` under the strict declaration policy below. Understood unused extras may broaden complete observed evidence without changing execution. |
| CPU | Mandatory in every build and the default selection. The exact-tree local closure passes focused domain/Candle/E0/E1 lifecycle and accounting suites, all six native components, the clean canonical composite, and both portable domain targets. This is local evidence, not remote Quality, CUDA, external-model, leak, or performance evidence. |
| CUDA | Non-default explicit ordinal 0 with no fallback. Implementation remains scoped to the exact RTX 5070 Ti row; Phase 12 remote hardware evidence applies only to `181a069`, and the artifact-loading amendment has later local hardware evidence. The current package's dedicated graph is registered, but local compilation stops in `cudarc` before Rust compilation because `nvcc` is absent. No current-tree CUDA compile or hardware result is accepted. |
| Resident models | One selected/resident model in E1. |
| Completion/chat | Direct completion for every loaded compatible model; built-in chat only for exact TinyLlama profile/revision `fe8a4ea1ffedaf415f4da2f062534de366a451e6`. |
| Persistence | redb preferences and model catalogue; `LAS1` writes v2/reads v1; `LAM1` writes v3 and reads exact v1/v2 without automatic rewrite. Conversation history remains memory-only. |
| Frontend | Thin Slint reference host. Its state path is now `milkdrift/state.redb`; a sole legacy `llm-app/state.redb` is moved once when no current database exists. |
| Incubating orchestration foundation | `task-graph` owns generic topology, attempt state, cancellation/blocking, deterministic readiness, and identity-only provenance. `corrective-workflow` owns a bounded data-defined corrective schema/executor and the current six-stage reference template. Neither is the general workflow/workspace product runtime. |

## Orchestration foundation truth

The portable `task-graph` no longer contains corrective task kinds, model/backend
selection, token budgets, output byte policy, artifact media/roles, or an
exactly-one-output axiom. `TaskNode<Operation>` carries caller-owned uninterpreted
metadata; topology, ready selection, attempt identity/retry/exhaustion,
cancellation/blocked propagation, and identity-only provenance use caller-owned
scratch/state and allocate nothing.

`corrective-workflow` now validates borrowed bounded definitions containing its
supported operations, typed artifact meanings, model/validator policy and token
budgets, attempt limits, artifact bindings, output limits, and terminal artifacts.
The executor preflights definition shape, graph/provenance, external bindings,
artifact capacity, and worst-case event capacity before port calls. It selects
generic ready nodes deterministically, executes through operation-specific typed
contexts, commits artifacts/events before generic success, rolls back failed or
cancelled runs without ID reuse, and requires explicit release after success.

The six-stage corrective behavior is `ReferenceCorrectiveTemplate` data passed
through the ordinary executor path. A structurally different three-node test
definition uses that same scheduler and proves definition-ordered selection when
multiple nodes are ready.

The exact focused source-tree matrix passed in one isolated target: format; all
targets for `domain-contracts`, `task-graph`, and `corrective-workflow`; all
`task-graph` tests including the harness-free allocation contract; all
`corrective-workflow` tests; strict Clippy; warning-denied rustdoc; both named
WASM/embedded portability commands; architecture; hygiene; and diff whitespace.
This is local source-tree evidence, not complete canonical workspace, remote,
CUDA, external-model, or product-workflow evidence.

## Scalar, artifact, and memory truth

The loader keeps four facts separate:

1. **Configuration declaration** — optional recognized F32/F16/BF16 producer intent from bounded `config.json`. Modern `dtype` never silently falls back; unsupported, conflicting, duplicate, malformed, or wrongly typed declarations fail explicitly.
2. **Complete observed set** — every structurally valid selected tensor header, including unused extras.
3. **Required set and primary** — adapter-private compatibility facts for tensors consumed by the supported Llama schema.
4. **Execution scalar** — selected during exact device-aware preparation and verified from the loaded backend by E0.

A genuine required F16+BF16 mixture, required unsupported dtype, empty required set, quantization, malformed structure, or contradictory declaration fails before publication. Mixed required `{F16,F32}` and `{BF16,F32}` layouts require the matching recognized declaration because the set alone does not establish a primary precision; absent declarations remain accepted only for homogeneous required sets. Structurally understood unused integer, boolean, FP8/bit-packed, wider numeric, complex, or other tensors remain observed/identity-checked evidence but are never staged, cast, transferred, retained, or counted in tensor footprints.

Source identity has two paths:

- exact Hugging Face LFS SHA-256 plus length at the resolved commit is `VerifiedImmutable` and may skip a pre-admission payload pass;
- project-established and unverified mutable sources are sequentially hashed from retained open files before admission.

Materialization re-verifies retained header/payload/EOF identity while hashing ignored ranges through a fixed buffer and allocating only required ranges. Path replacement cannot redirect the retained file.

`MemoryFootprint` contains concrete host/device weight/working bytes only. Sequence-cache bytes per token is a separate planning rate. `LoadPlan::final_footprint` is exact post-load required-tensor ownership; `loading_peak_footprint` is the separate required-only materialization peak. `SequencePlan::reservation` carries persistent logical payload, additional transient headroom, and their checked total. Persistent KV ownership scales across every layer, while Candle's sequential block loop admits only one block's source-derived transient peak plus outer embedding/norm/logit state. Caller generation workspaces remain separate. Parsed metadata, the fixed verification buffer, allocator/driver overhead, contexts/workspaces, process RSS, and whole-device memory remain separate bounded or sampled facts.

## Retained ownership truth

E0 reserves the accepted loading peak before materialization and commits only verified final ownership. It also admits the complete sequence total before native creation, keeps caller workspace accounting separate, and verifies sequence identity, capacity, and the full immutable plan after creation/execution and around destruction. An ordinary-drop-safe preparation is consumed into a distinct failed-materialization typestate when native acquisition fails. Matching failed-owner plan reports retain the exact accepted peak while cleanup is pending. A substituted or mutated failed-owner/model/sequence report, or a sequence identity/capacity contradiction, becomes `Unverified` accepted/reported/conservative evidence if cleanup fails, is excluded from exact aggregate bytes, preserves earlier larger reports monotonically, and blocks new admission. Verified unload and conforming sequence-destruction failures retain their accepted exact reservations.

Release requires correlated explicit cleanup/unload/shutdown evidence. Zero exact bytes, a bounded snapshot omission, endpoint disconnection, or worker/join-handle absence does not prove release. Cleanup rotates fairly with bounded attempts and reports terminal process-lifetime retention when native ownership cannot be released.

E1 exposes retained state as `ApplicationRetainedModel`: resource, `Exact`/`Unverified`/`Unknown` ownership, cleanup disposition, primary failure, and optional cleanup failure. One private checked coordinator owns every cleanup origin/action. `ModelCleanupPending { resource, disposition }` is a compact notification over the durable state, which never coexists with a normal `LoadedModel`; selection/load/generation remain locked until explicit release evidence.

## Repository and verification infrastructure

Every tracked non-fixture Cargo package must be a root workspace member, and every member declares one explicit role in `[package.metadata.milkdrift]`. The validator requires exact root policy schema version 1 and checks compatible location, a generic inward role DAG, the actual acyclic F0/F1 graph, outer-only observers, tooling isolation, dependency kinds, exact external/development exceptions, and fail-closed CUDA features. Ordinary legal normal/build Cargo edges are not duplicated in a package-name registry.

Maintained Cargo benchmark targets are declared by owning package metadata and matched bidirectionally against Cargo targets plus exactly one explicit `harness = false` manifest entry. The complete inventory is:

```text
runtime-benchmarks / runtime
sampling           / sampling_pipeline
```

`cargo xtask verify` compiles those exact targets only. It never runs `cargo bench --workspace --no-run`. Its six canonical plans are also exposed as `cargo xtask verify-component structure|check|test|clippy|docs|benches`; both entry points consume the same typed metadata-owned operations. The exploratory scheduled plan is `verify-component nursery`.

GitHub Quality now has six independent native matrix legs plus separate WASM, embedded, policy, nursery, and link jobs. Every leg owns a unique `RUNNER_TEMP` target, disables incremental compilation, records disk use, rejects checkout-local targets, and unconditionally removes its target/tool/shim resources through the reviewed shared script. Only the nursery lint-report step is non-blocking. CUDA preserves separate check and release-hardware targets and runs whole dedicated harness-free adapter/E0/E1 suites without parsing function names. All first-party checkout steps use immutable v7.0.1 commit `3d3c42e5aac5ba805825da76410c181273ba90b1` with read-only permission and credentials disabled.

External evidence schema 6 observes the public E1 product path without an independent adapter preparation. It retains variable provenance/timing/count/process/whole-device observations and removes shadow planning fields, derivable duplication, invariant prose, and tautological success flags. No schema-6 CPU/CUDA product report is accepted; historical reports retain their original schema and commit attribution.

## CI resource-topology source-tree validation

The redesigned command-plan and workflow tests pass locally, including composite/component parity, exact operations, fail-closed unknown roles/components, benchmark and portable ownership, CUDA owner/suite preservation, unique targets, centralized cleanup, immutable action pins, and shell parsing. Fresh local component targets completed at approximately 143 MiB (structure), 1.94 GiB (check), 7.33 GiB (tests), 1.94 GiB (Clippy), 1.95 GiB (rustdoc), and 1.20 GiB (exact benches); each portable leg retained about 149 MiB. Sequential policy-tool compilation sampled below 0.9 GiB. These measurements justify per-leg standard-runner preflights from 1–9 GiB and do not constitute product-performance or GitHub-hosted evidence.

The standard hosted class remains Ubuntu 24.04 with 14 GB total SSD; no larger runner is selected. The separate self-hosted CUDA gate retains its 20 GiB host-specific reserve: historical run 31281013243 observed 139 GiB free on that runner's 1.9 TiB root filesystem. The redesigned Quality and CUDA workflows still require exact-tree remote execution before they can be accepted.

## Foundation closure 05 local acceptance

The clean source candidate `b1f7e90b1ba67f1cf968d773052b5062ef8cbbb9`,
tree `fcb3ee6fa00243734abd74b64218aa0db2e340c1`, passed the complete
download-free local closure matrix on the UM790 Pro. The result includes fast
structure, the focused E1/persistence/Candle/E0/orchestration/tooling suites, all
six native component plans, the fresh canonical composite, both portable domain
targets, pinned dependency policy, and pinned offline Markdown links. Three
consecutive complete `application-runtime` runs were stable. No repair was
required.

The CPU lifecycle verdict is accepted for the deterministic project fixtures:
homogeneous and mixed loads, independent declaration/observed/required/execution
scalar facts, sequence reservation and execution, output backpressure and
cancellation, ordinary unload to zero deterministic ownership, retained and
exhausted cleanup states, complete-model contradictions, the unified E1 cleanup
coordinator, and bounded shutdown/disconnection behavior all passed. This proves
the checked accounting and lifecycle contracts, not physical leak freedom,
representative performance, language quality, or an external checkpoint.

The portable plans compiled only the five metadata-owned domain libraries for
both `wasm32-unknown-unknown` and `thumbv7em-none-eabihf`. Cargo-deny 0.20.2
passed advisories, bans, licenses, and sources. Lychee 0.24.2 checked 256 links
with no errors. GitHub-hosted Quality, current RTX CUDA compile/hardware suites,
and external-model evidence remain pending and cannot be inferred from this
local acceptance.

## Predecessor-tree local validation

The following results apply to the predecessor infrastructure-truth working tree. They do **not** validate the post-closure foundation repair described above; the current package's separate focused source-tree results are recorded in [validation](validation.md):

| Evidence class | Result |
|---|---|
| Architecture/hygiene/verification policy | `cargo test --locked -p xtask` passed 3 unit, 21 architecture, 8 hygiene, and 1 command-surface tests, including tracked-manifest membership, mandatory policy schema version, manifest-level benchmark harness checks, dependency-name CUDA denials, and executable harness-free CUDA fixture targets. Direct `cargo xtask architecture` and `cargo xtask hygiene` passed. |
| Canonical native gate | Passed from a previously absent `/tmp/milkdrift-native-validation-target` with `CARGO_INCREMENTAL=0` and failing CMake/Python/Hugging Face CLI shims: format, workspace all-target check, workspace tests/doctests, strict Clippy, warning-denied rustdoc, and both exact release benchmark compilations. |
| Native disk observation | The final retained isolated target was 10,517,044,748 bytes (9.9 GiB). The post-gate tmpfs observation was 16,544,532 KiB used and 15,229,456 KiB free. This is local CI-infrastructure evidence, not hosted-runner peak evidence or product performance. The pre-existing ignored root `target/` remained unchanged during the clean gate at 17,615,909,720 bytes and supplied no build artifacts. |
| Portable matrices | All five domain packages passed on both `wasm32-unknown-unknown` and `thumbv7em-none-eabihf`; the fresh targets were respectively 6,397,064 and 6,396,759 bytes and were removed. |
| Persistence migration | Two focused `desktop-slint` path tests passed for fresh Milkdrift state and one-time non-overwriting legacy migration. |
| Supply chain | Pinned cargo-deny 0.20.2 passed advisories, bans, licenses, and sources; duplicate-version reports remained warnings/audit input. |
| Offline links | Pinned Lychee 0.24.2 checked 248 links: 229 OK, 19 excluded, 0 errors. |
| Workflow static validation | Workflow review and repository searches found no floating first-party Actions, deprecated checkout pin, old target names, or CUDA test-name parsing. `actionlint`/another YAML linter is not installed locally, so no actionlint pass is claimed. |
| CUDA compile/hardware | Architecture feature topology passed. Exact isolated CUDA `cargo check` was attempted and stopped in `cudarc` because `nvcc --version` could not execute; `nvidia-smi`, `nvcc`, and NVIDIA device nodes are absent in this environment. No Rust CUDA diagnostic, dedicated-suite execution, or current-tree hardware result is claimed. |
| External checkpoint/product evidence | Not run. No network/model download occurred. No immutable license-reviewed external mixed checkpoint is established. |

Isolated validation targets and temporary tool installation directories are removed after evidence capture; the pre-existing ignored root target is preserved. Formatting, whitespace, status, and post-commit checks are also reported in the completion response and execution history.

## Historical remote evidence

- Phase 11 shared Quality [run 30942153370](https://github.com/hartolit/milkdrift/actions/runs/30942153370) and CUDA [run 30942148369](https://github.com/hartolit/milkdrift/actions/runs/30942148369) passed on commit `1a62d2ed6623500e9052b4b8386ebd058984bd89`.
- Phase 12 CUDA [run 31281013243](https://github.com/hartolit/milkdrift/actions/runs/31281013243) passed on closure commit `181a069ce81525e9c144fe8de051ced8e3c0b9d7`.
- Phase 12 Quality [run 31281013257](https://github.com/hartolit/milkdrift/actions/runs/31281013257) passed its canonical native work, then the old workspace-wide bench build left roughly 49 MiB free and the later WASM/root-target work failed with `No space left on device`. The linker bus error was consequential. This is infrastructure history, not a Rust/WASM product failure.
- Those runs do not prove `d4a1e43`, `b43d0f4`, `1f91cba`, the infrastructure-truth tree, or the post-closure foundation repair.

## Unsupported behavior and open evidence

- Quantized/GGUF loading, non-Llama architectures, arbitrary required mixtures, required F16+BF16, required unsupported dtypes, Metal, cuDNN, flash attention, NCCL, multi-GPU, generic `gpu`, automatic CPU fallback, and GPU-side sampling are unsupported.
- CUDA outside an actually observed exact row is unclaimed. The current tree still needs post-push self-hosted execution of all dedicated suites.
- The exact-tree local focused, canonical, portable, policy, and offline-link
  matrices pass. Hosted Quality and current CUDA compile/hardware results remain
  pending.
- No schema-6 external product run or immutable reviewed external mixed-checkpoint evidence exists.
- Multi-model E1 residency, generalized chat, conversation persistence, general workflows/workspaces/plugins/providers/peers, and browser/remote transport are not implemented.
- Synthetic fixtures prove deterministic compatibility/lifecycle behavior, not language quality, representative scale, production throughput, or external-checkpoint compatibility.

## Next direction

The next product direction is workflow/workspace/authority, but it is not yet ratified beyond that direction. This maintenance closure does not authorize feature implementation or promotion of the current reference application into the general workflow core.
