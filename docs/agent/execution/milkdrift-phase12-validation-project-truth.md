# Milkdrift Phase 12 — validation and project truth

This is the final ownership segment of Phase 12, not a new project phase. Work directly in the local codebase in Zed and implement the changes. Do not return a patch or code block.

Start from the reviewed commits produced by the core loader/runtime and artifact/application integration segments.

## Purpose

Validate the complete Phase 12 behavior, update non-production evidence infrastructure, and reconcile canonical documentation without changing the project's workflow-first identity or inflating the frontend.

This segment owns observation and project truth. It must not redesign production APIs to make reports easier.

## Read before editing

Read:

- `docs/vision.md`
- `README.md`
- `docs/rules.md`
- `docs/project/architecture.md`
- `docs/project/implementation-status.md`
- `docs/project/candle-backend.md`
- `docs/project/inference-runtime.md`
- `docs/project/application-runtime.md`
- `docs/project/validation.md`
- `docs/project/performance.md`
- `docs/agent/execution/analyzer.md`
- `docs/agent/execution/current.md`
- `docs/agent/execution/execution-plan.md`
- `docs/agent/execution/history.md`
- `benchmarks/runtime/README.md`
- `docs/agent/execution/milkdrift-phase12-per-tensor-safetensors-compatibility.md`
- both preceding segment completion reports and their diffs

Inspect:

- `benchmarks/runtime`
- `.github/workflows`
- relevant xtask policy and hygiene rules
- repository-local model fixtures and their provenance

## Scope

You may change:

- the benchmark/evidence observer and its schemas;
- download-free CPU/CUDA integration tests and workflow path filters;
- opt-in external validation commands and reports;
- canonical implementation, validation, performance, backend, runtime, roadmap, and execution-history documentation;
- root README wording only where Phase 12 current-status truth changed;
- small production fixes only when a failing integration test demonstrates a concrete defect that cannot be corrected in the observer.

Do not change:

- the authentic project vision or operator-programmable identity;
- workflow, workspace, plugin, provider, peer, or control-center design;
- production contracts merely to preserve an obsolete benchmark schema;
- the Slint feature surface;
- supported hardware claims beyond executed evidence;
- downloaded model weights into committed fixtures.

## Evidence model

Preserve the distinction among:

- configuration-declared scalar metadata;
- observed source tensor layout;
- selected/actual execution scalar;
- deterministic accounted footprint;
- E0 reserved ownership;
- process RSS;
- whole-device memory observation;
- supported behavior versus compilation-only coverage.

The benchmark/evidence observer may record a compact source-layout summary when useful, but it must not force adapter-private tensor names or shard DTOs into production public APIs. Prefer deriving detailed evidence through test-only or observer-facing APIs that do not become stable product contracts.

Do not rewrite historical measurements as if they were produced by the Phase 12 tree. Introduce a new schema version only when fields or meanings materially changed, retain old-schema parsing when still required, and keep provenance explicit.

## Validation requirements

### Deterministic CPU gate

Ensure ordinary shared CPU validation covers:

- homogeneous source regressions;
- mixed F16/F32 end-to-end loading and generation;
- mixed BF16/F32 CPU execution policy;
- exact conversion-aware plan and receipt facts;
- budget rejection before materialization;
- release, unload, and empty final accounting;
- malformed/unsupported layout rejection;
- retained partial-load cleanup fault paths through deterministic fakes.

This gate must be download-free.

### CUDA hardware gate

Update the existing self-hosted CUDA workflow so relevant changes in contracts, adapter code, runtime code, application integration, fixtures, benchmarks, or workflow configuration trigger the hardware job.

The hardware gate should validate, on the already accepted exact matrix:

- explicit CUDA selection with no fallback;
- at least one deterministic mixed-dtype fixture;
- planned versus actual execution scalar and device;
- conversion-aware device/host accounting;
- successful generation, release, unload, and final empty E0 ownership;
- selected failure/cleanup paths that can be exercised deterministically without risking the runner.

Do not add elapsed-time thresholds or infer generic NVIDIA support. Do not treat CUDA compilation as hardware evidence.

### External checkpoint evidence

Provide an explicit, opt-in procedure for one pinned external mixed-dtype Llama-compatible checkpoint only when a suitable immutable repository and license/provenance can be reviewed.

Requirements:

- pin an immutable commit/revision;
- record repository, artifact layout, configuration-declared metadata, observed dtype summary, selected device, execution scalar, and limitations;
- require explicit network authorization;
- do not run it in the canonical offline gate;
- do not commit downloaded weights or cache contents;
- do not make successful access to a gated model a Phase 12 correctness dependency;
- distinguish a missing credential/network failure from product incompatibility.

If no suitable external checkpoint can be responsibly pinned, document the evidence gap and close only the deterministic compatibility claim. Do not fabricate or substitute a homogeneous model.

## Documentation reconciliation

Update canonical owners so a contributor can answer:

- what Phase 12 changed;
- what “declared,” “observed,” and “execution” scalar mean;
- which mixed layouts are supported;
- how exact memory planning is derived;
- how partial load ownership and cleanup work;
- which CPU and CUDA environments were executed;
- which layouts, devices, and model families remain unsupported;
- why this work strengthens one local execution endpoint rather than redefining Milkdrift as a Candle application.

At minimum reconcile:

- `docs/project/candle-backend.md`
- `docs/project/inference-runtime.md`
- `docs/project/application-runtime.md`
- `docs/project/implementation-status.md`
- `docs/project/validation.md`
- `docs/project/performance.md`
- `docs/agent/execution/current.md`
- `docs/agent/execution/execution-plan.md`
- `docs/agent/execution/history.md`
- relevant ADR status or a new ADR when the load-transaction contract is a durable architectural decision
- `benchmarks/runtime/README.md`

Keep the root README concise. It should mention broader local model compatibility only if now supported, while preserving the workflow-first project identity and the thin-frontend position.

Do not replace `docs/vision.md` with generic inference-runtime prose.

## Final architecture review

Before closure, explicitly verify:

- Safetensors-specific structures do not cross into workflow-oriented portable foundations;
- E0 remains backend-neutral and validates the adapter contract;
- E1 does not select per-tensor conversion policy;
- Slint remains a disposable reference host;
- no default feature graph reaches CUDA;
- explicit CUDA failure does not fall back to CPU;
- no project-authored unsafe code was introduced;
- domain portability checks remain valid;
- benchmark/evidence code remains an inward observer and no production package depends on it;
- Phase 12 introduced no hidden workflow procedure or future execution-target assumption.

## Canonical validation

Run the complete project gate from a clean target as required by the repository:

```text
cargo xtask verify
```

Also run the documented portable target checks, dependency/license policy, offline Markdown link check, and the complete opt-in CUDA compile chain where they are not already part of the canonical gate.

Run the self-hosted CUDA hardware workflow when the accepted runner is available. Record exact commit/tree and workflow-run provenance only after the run succeeds.

Inspect:

```text
git status --short
git diff --check
git diff --stat
```

Do not close on formatting-only success when tests or evidence remain pending.

## Completion report

Finish with a concise closure report containing:

- final supported and unsupported source layouts;
- final load-transaction and partial-cleanup semantics;
- CPU verification results;
- CUDA compilation and hardware execution results as separate facts;
- external checkpoint evidence or the explicit reason it remains absent;
- benchmark/evidence schema changes;
- documentation owners updated;
- exact remaining limitations;
- confirmation that the next major program returns to workflow/workspace/authority architecture rather than continuing unbounded loader expansion.

Create one coherent closure commit only after the canonical CPU gates pass. Do not mark CUDA evidence complete until the hardware run succeeds.
