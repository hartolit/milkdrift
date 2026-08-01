# Current execution context

**Status date:** 2026-08-01
**Prior committed checkpoint:** `3942a19b97d347fd238c451d2b0a2fcbea287873` (tree `be069879fea9531799038c5189c9edb3007ebf72`)
**Current target:** Phase 10 implementation is next; the pre-Phase 10 closure is complete
**Phase 9 state:** complete and preserved as history
**Pre-Phase 10 state:** terminal shutdown semantics, benchmark architecture/hygiene, and fixture provenance policy are implemented; no Phase 10 benchmark suite or result exists
**Decision state:** amended [ADR-0006](../decisions/0006-explicit-bounded-shutdown.md) defines retryable versus terminal shutdown; [ADR-0018](../decisions/0018-benchmark-and-model-fixture-policy.md) defines benchmark and fixture policy
**Validation state:** the clean starting tree passed the canonical gate; focused E0/E1 lifecycle and `xtask` policy suites pass on this closure working tree; final exact-tree evidence belongs in the execution report or CI log
**Canonical plan:** [execution-plan.md](execution-plan.md)
**Current product truth:** [project implementation status](../../project/implementation-status.md)
**Historical evidence:** [execution history](history.md)

This file is the dense handoff after the pre-Phase 10 closure. Implementation status owns current support and validation summaries, accepted ADRs own decisions, the validation guide owns repeatable commands, and history owns baseline-specific evidence. CI records the tested commit and Git tree in its runtime log; a tracked document does not attempt to contain its own resulting commit SHA or tree hash.

## Current architecture

The supported local composition is exactly:

```text
Slint or another native frontend
        ↓
application-runtime (E1)
        ├── one bounded Hugging Face Hub worker
        ├── one concrete Hugging Face tokenizer/decoder path
        ├── redb application persistence
        └── one Candle E0 worker/thread
                    ↓
             inference-runtime (E0)
                    ↓
     Candle + Safetensors + CPU
```

`ModelSelection` is a normalized Hugging Face repository plus revision. Resolution pins it to an immutable Hub commit. Resolved/loaded state derives engine, source, device, format, scalar, tokenizer vocabulary, and immutable identity from the supported artifacts; unsupported cross-products are not public selection options.

Direct completion remains available for every loaded model. Chat remains limited to `TinyLlama/TinyLlama-1.1B-Chat-v1.0` at commit `fe8a4ea1ffedaf415f4da2f062534de366a451e6` with `</s>` mapped to token ID 2. GGUF, GPU execution, hosted/peer execution, browser transport, and multiple application-resident models remain deferred.

## Phase 9 closure

### 9.1 — Source, graph, and policy cleanup complete

- Candle is the sole local engine; removed llama.cpp/GGUF production and tooling paths remain absent.
- Engine, artifact source, format, scalar, and device remain separate concepts.
- Project-owned operational tooling is Rust/Cargo-native and the selected graph remains fail-closed.

### 9.2 — Narrow E1 composition complete

- `application-runtime` remains one public frontend-neutral, non-generic façade.
- Private composition owns one `HostedRuntime<CandleLlamaSource>`, one inference thread, one Hub worker, one resolved tokenizer, request-local streaming decoders, and one resident-model lifecycle.
- `corrective-workflow` remains an independent capability engine.

### 9.3 — Behavior and test preservation complete

- E0 retains exclusive resource, sequence, scheduling, cancellation, cleanup, accounting, unload, and shutdown ownership.
- Download-free deterministic loaders and the newly generated synthetic Candle fixture preserve lifecycle and generation coverage.
- E1 and Slint preserve resolution, completion, exact chat, context, cancellation, backpressure, unload, persistence, and presentation behavior.

### 9.4 — Documentation and validation reconciliation complete

- Current architecture, status, workspace, component, validation, execution, and frontend guidance describe the same Candle/Hub/Safetensors/CPU product.
- Phase 8 remains factual history rather than current support.
- Provenance no longer requires a commit to contain its own final SHA.

### 9.5 — Structural reconciliation complete

- The root manifest is a virtual workspace. Custom policy and composite verification live in `tools/xtask`; ordinary Cargo operations have no forwarding subcommands.
- The exact reviewed acyclic domain DAG contains the four current F1 → F0 edges. The coarse policy can represent a justified F1 peer, but every domain edge must be explicitly registered with a rationale and the complete graph must remain acyclic.
- `TaskId` is owned by `task-graph`; `domain-contracts` is reserved for backend/runtime crossings or stable vocabulary shared by at least two distinct domains.
- Oversized production internals were split by responsibility: E0 runtime operations, E1 generation, task graph/artifact/state/error logic, desktop presentation, and repository tooling.
- Stable selected Clippy lints remain mandatory under `-D warnings`; the nursery group is a separate scheduled, non-blocking exploratory report.

### Lifecycle hardening completed before closure

- The superseded Phase 9 implementation tracked a combined failed/retryable state; the pre-Phase 10 correction below separates retryable joins from terminal E0 cleanup failure.
- Incompatible loaded models remain in private cleanup accounting through unload-submission retry, runtime disconnection, successful unload, or observable retry exhaustion.
- Startup uses an owning rollback guard. If Hub construction/spawn fails after inference starts, E1 attempts bounded inference shutdown/join before returning the primary Hub error; a rollback timeout moves the complete owner into a private quarantine that a later startup retries.

### External-tooling cleanup completed

- Ubuntu CI retains `build-essential` and Slint development packages but no longer installs system CMake.
- The selected non-FIPS AWS-LC path is forced through its CC builder.
- Required CI runs the canonical gate from a fresh target with failing shims for CMake, Clang, Python/package-manager, and Python-distributed Hugging Face commands.
- The temporary Candle cleanup agent brief was removed; hygiene applies no filename-, directory-, history-, or ADR-status whole-file bypass.

## Current invariants

1. Candle is the sole local execution engine; Safetensors, Hugging Face Hub, and CPU are current format/source/device facts rather than engine aliases.
2. E1 owns exactly one Candle inference worker/thread plus one Hub worker and permits one resident model.
3. Frontends construct only application-owned selections and never construct Candle sources, Hub clients, tokenizers, devices, or E0 commands.
4. Direct completion is general to loaded models; chat is tied to the exact verified TinyLlama profile.
5. E0 owns local resources, scheduling, sampling, cancellation boundaries, backpressure, cleanup quarantine, accounting, unload, and terminal shutdown.
6. Terminal generation and resource release remain distinct; pending/exhausted cleanup stays observable and accounted.
7. Explicit shutdown success requires observed clean E0 completion and confirmed worker joins; terminal cleanup failure remains sticky after worker exit, while a join timeout remains retryable.
8. Ordinary tests remain download-free, and maintained operational tooling remains Rust/Cargo-native.
9. The domain dependency registry is exact, justified, and acyclic; layer membership alone does not authorize a domain edge.
10. The mandatory lint gate excludes the blanket nursery group; exploratory findings do not silently become merge blockers.
11. Component benchmarks are real crate-local targets; future cross-crate/system measurement belongs only in `benchmarks/runtime` as an outer non-production consumer.
12. Committed model fixtures require explicit provenance; real-model measurements are opt-in, immutable-revision, cache/local-path based, and never ordinary-CI downloads or repository redistribution.

## Phase 10 entry

The pre-Phase 10 lifecycle correction is complete, benchmark layout and fixture policies are established, and Phase 10 implementation is the next operation. No Phase 10 performance result has been recorded.

Phase 10 must expand the existing sampling benchmark, add one `benchmarks/runtime` system harness, record reproducible environment metadata, and measure controlled lifecycle/memory behavior. Tokenizer, context-planner, output-accumulator, and isolated Candle microbenchmarks remain conditional on a named question and an explanation of why the system harness is insufficient. Do not optimize from unmeasured assumptions, add hard wall-clock thresholds to shared CI, or treat the correctness-oriented external Hub smoke as a benchmark.

## Validation and recording rule

Follow [project validation](../../project/validation.md). The canonical command is:

```text
cargo xtask verify
```

CI prints `HEAD` and `HEAD^{tree}` immediately before that command. Local working-tree evidence records the prior committed checkpoint plus dirty-state context; CI-attached commit and run metadata are the authoritative identity for a committed result. Historical evidence from `3942a19…` or an earlier tree must not be reused as proof after source changes.
