# Current execution context

**Status date:** 2026-08-01
**Prior committed checkpoint:** `f0fe9c6623f1e2afd569767d903f3978e00560da` (tree `db8a9ae77f41e0e769c7434ce21a940ae33784ae`)
**Current target:** Phase 10 — meaningful performance work may begin from the closed Phase 9 baseline
**Phase 9 state:** complete; Candle-only correction, structural reconciliation, lifecycle hardening, tooling cleanup, and documentation closure are implemented
**Decision state:** [ADR-0013](../decisions/0013-candle-only-local-execution.md) through [ADR-0017](../decisions/0017-stable-clippy-gate-exploratory-nursery.md) define the current composition, tooling, domain DAG, workspace, and lint policy
**Validation state:** focused changed-package tests and the canonical locked gate pass; clean forbidden-tool, portability, policy, and link evidence is summarized in [implementation status](../../project/implementation-status.md)
**Canonical plan:** [execution-plan.md](execution-plan.md)
**Current product truth:** [project implementation status](../../project/implementation-status.md)
**Historical evidence:** [execution history](history.md)

This file is the dense handoff after Phase 9 closure. Implementation status owns current support and validation summaries, accepted ADRs own decisions, the validation guide owns repeatable commands, and history owns baseline-specific evidence. CI records the tested commit and Git tree in its runtime log; a tracked document does not attempt to contain its own resulting commit SHA or tree hash.

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
- Download-free deterministic and real Candle fixtures preserve lifecycle and generation coverage.
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

- Shutdown tracks `Running`, `Stopping`, `Stopped`, and `FailedOrRetryable`; worker join handles stay owned across timeouts, and a later call retries unresolved joins instead of reporting false success.
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
7. Explicit shutdown success means both workers were confirmed stopped; starting shutdown alone is not idempotent success.
8. Ordinary tests remain download-free, and maintained operational tooling remains Rust/Cargo-native.
9. The domain dependency registry is exact, justified, and acyclic; layer membership alone does not authorize a domain edge.
10. The mandatory lint gate excludes the blanket nursery group; exploratory findings do not silently become merge blockers.

## Phase 10 entry

Phase 9 is explicitly closed. Phase 10 may now add measurement coverage without reopening removed product paths or weakening lifecycle guarantees. Begin with the existing sampling benchmark gaps and controlled product-level TTFT, throughput, memory, cancellation, and unload measurements. Do not optimize from unmeasured assumptions, add hard wall-clock thresholds to shared CI, or treat the correctness-oriented external Hub smoke as a benchmark.

## Validation and recording rule

Follow [project validation](../../project/validation.md). The canonical command is:

```text
cargo xtask verify
```

CI prints `HEAD` and `HEAD^{tree}` immediately before that command. Local working-tree evidence records the prior committed checkpoint plus dirty-state context; CI-attached commit and run metadata are the authoritative identity for a committed result. Historical evidence from `f0fe9c6…` or an earlier tree must not be reused as proof after source changes.
