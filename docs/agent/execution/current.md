# Current execution context

**Reviewed baseline:** `15d9e87cdaee77fd0d49247712d3c12dfb3adea2` plus the current uncommitted Candle-only cleanup tree
**Current target:** Phase 9 — continue structural review from the validated Candle-only baseline
**Historical entry state:** Phase 8 completed and validated its then-current dual-product tree; the Candle-only correction later superseded that composition
**Decision state:** [ADR-0013](../decisions/0013-candle-only-local-execution.md) supersedes ADR-0012; [ADR-0014](../decisions/0014-rust-cargo-native-operational-tooling.md) defines maintained tooling
**Final gate state:** the canonical full locked gate, policy/portability audits, clean shimmed build, and Rust-native external Hub smoke pass on the current cleanup tree
**Canonical plan:** [execution-plan.md](execution-plan.md)
**Current product truth:** [project implementation status](../../project/implementation-status.md)
**Historical Phase 8 evidence:** [execution history](history.md#phase-8--gguf-parity-and-native-composition-evidence)

This file is the derived dense handoff for active Phase 9 work. Implementation status owns current support and validation provenance, accepted ADRs own decisions, the validation guide owns commands, and history owns closed-phase evidence. Phase 8 remains factual history, but its former product composition is not a current invariant.

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

`ModelSelection` is a normalized Hugging Face repository plus revision. Resolution pins it to an immutable Hub commit. Resolved/loaded state derives engine, source, device, format, scalar, tokenizer vocabulary, and repository/commit identity from the actual artifacts; unsupported combinations are not public selection options.

Direct completion remains available for every loaded model. Chat remains limited to `TinyLlama/TinyLlama-1.1B-Chat-v1.0` at commit `fe8a4ea1ffedaf415f4da2f062534de366a451e6` with `</s>` mapped to token ID 2. Context planning, regeneration/supersession, bounded transcript/output behavior, cancellation, cleanup retry/exhaustion, unload policy, persistence, and explicit bounded shutdown remain required.

GGUF is unsupported. Possible Candle-native GGUF or other quantized-format work is deferred and requires a separate reviewed implementation. GPU execution is also deferred. There is no current llama.cpp product or secondary operational toolchain.

## Immediate objective

The Candle-only correction is closed on the current validated working tree. Continue the remaining Phase 9 reviews only from concrete evidence:

```text
validated Candle-only baseline
→ review the F1 dependency DAG
→ split oversized internals only by invariant and ownership
→ evaluate the planned xtask migration as one coherent command change
→ review mandatory lint policy
```

Do not revive former product vocabulary, dormant variants, a second worker, backend routing, local-file selection, or compatibility scaffolding merely to make hypothetical expansion look easy.

## Active Phase 9 work packages

### 9.1 — Reconcile source, graph, and policy (complete)

- Keep Candle as the sole local engine and keep engine, artifact source, format, and device conceptually distinct.
- Ensure the removed native engine, adapter, fixture path, public variants, and graph entries do not remain in current source, manifests, CI, or maintained guidance.
- Keep project-owned maintenance Rust/Cargo-native and enforce the selected graph and tracked operational surfaces through the root hygiene command.
- Retain only native CI prerequisites with a current selected owner: `build-essential` and `cmake` for the Hub TLS dependency path, plus Slint system packages.

### 9.2 — Preserve the narrowed E1 composition (complete baseline)

- Keep `application-runtime` as one public frontend-neutral, non-generic façade.
- Keep one private `HostedRuntime<CandleLlamaSource>`, one inference thread, one Hub worker, one `HfTokenizer`, and request-local `HfOwnedStreamingDecoder` values.
- Keep redb in E1 while it owns application preferences/catalogue semantics.
- Do not create another runtime, public plugin registry, generic application façade, or `application-api` without a real lifecycle/consumer.

### 9.3 — Preserve behavior and test substitution (complete baseline)

- E0 remains backend-neutral at its contracts and exclusively owns model resources, sequences, request admission, scheduling/sampling, cancellation boundaries, cleanup quarantine, accounting, unload, and shutdown.
- Keep deterministic E0 loaders/fault injection for backend-independent behavior and the committed Candle real fixture for production-adapter coverage.
- Preserve E1 resolution, direct completion, exact chat, context, cancellation, backpressure, unload, persistence, worker-disconnection, and shutdown coverage.
- Keep ordinary tests download-free; external Hub validation stays explicit and opt-in.

### 9.4 — Reconcile documentation and evidence (complete)

- Current architecture/status/component guides describe only Candle/Hub/Safetensors/CPU support.
- The analyzer receives a supersession banner; its dated body remains evidence.
- The completed Phase 8 plan/history and recovered implementation plan remain clearly historical.
- Current validation documents use the Rust-native E1 Hub smoke and root hygiene command.
- The canonical gate, supplemental policy/portability audits, clean shimmed build, and external Hub smoke are recorded in implementation status and execution history.

### 9.5 — Continue later structural review only from evidence

After this correction is stable, Phase 9 may still review the F1 dependency DAG, oversized internal modules, root-runner/xtask shape, and mandatory lint set. Those reviews must not block removal of a known-wrong production path or weaken the preserved lifecycle contracts.

## Current invariants

1. Candle is the sole local execution engine; Safetensors, Hugging Face Hub, and CPU are current format/source/device facts rather than engine aliases.
2. E1 owns exactly one Candle inference worker/thread plus one Hub worker and permits one resident model.
3. `ModelSelection` contains repository and revision only; immutable resolution retains the Hub commit.
4. Public resolved/loaded facts are engine, source, device, format, scalar, tokenizer vocabulary, and immutable Hub identity derived from the supported composition.
5. Frontends construct only application-owned selections and never construct Candle sources, Hub clients, tokenizers, devices, or E0 commands.
6. Direct completion remains general to loaded models; chat remains tied to the exact verified TinyLlama profile.
7. E0 owns local resources, scheduling, sampling, cancellation boundaries, backpressure, cleanup quarantine, accounting, unload, and terminal shutdown.
8. Terminal generation and resource release remain distinct; pending/exhausted cleanup stays observable and accounted.
9. Conversation provenance, turn-atomic planning, bounded exact correction, pinned-content rules, regeneration/supersession, and in-memory-only history remain unchanged.
10. `corrective-workflow` remains an independent capability runtime with bounded service-port output and explicit artifact lifecycle.
11. Explicit shutdown cooperatively stops and bounded-joins the sole E0 worker and the Hub worker; blocking `Drop` is not the normal protocol.
12. Ordinary tests remain download-free, and maintained operational tooling is Rust/Cargo-native.

## Explicit non-goals

- no Candle-native GGUF or other quantized-format implementation in this correction;
- no GPU, hosted-provider, peer, browser/network transport, or multiple-resident-model path;
- no new local execution engine;
- no public plugin registry, dynamic dispatch in token-sensitive execution, or generic public E1 façade;
- no speculative `application-api` or local-runtime extraction;
- no broad E0 lifecycle rewrite, useful-test removal, or weakened architecture/capacity/cleanup policy;
- no erasure of the recovered implementation plan, superseded ADR rationale, completed Phase 8 plan, or Phase 8 history;
- no reuse of this validation claim after the source tree changes; rerun the exact applicable commands.

## Phase 9 correction acceptance

- Current source, API, UI, manifests, lockfile, CI, and current documentation agree on Candle/Hub/Safetensors/CPU.
- E1 starts and shuts down one inference worker and one Hub worker.
- No dead second-product public vocabulary, runtime routing, fixture, or selected dependency remains.
- Direct completion, exact TinyLlama chat, lifecycle, backpressure, cancellation, cleanup, unload, persistence, and bounded shutdown remain covered.
- The Rust-owned hygiene check passes and selected-graph audits show no prohibited package family.
- Local Markdown links and dependency policy checks pass.
- The canonical full gate and opt-in external smoke pass and are recorded with exact baseline and pinned-model provenance.

## Validation and recording rule

Follow [project validation](../../project/validation.md). Validate focused packages and policy first. The canonical command remains `cargo run --locked --bin llm-app -- verify`; the planned xtask migration has not occurred and must not be documented as current. The cleanup checkpoint is recorded in [execution history](history.md#phase-9-checkpoint--candle-only-architecture-and-rust-native-tooling).
