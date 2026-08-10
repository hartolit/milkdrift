# Milkdrift execution plan

**Plan status:** infrastructure-truth maintenance implementation complete; CUDA/remote evidence parked; no product phase active
**Status date:** 2026-08-10

This document owns execution ordering and activation state. Current support is canonical in [implementation status](../../project/implementation-status.md), repeatable commands in [validation](../../project/validation.md), exact measurements in [performance evidence](../../project/performance.md), and closed chronology in [history](history.md).

## Completed product program

| Program | Durable outcome |
|---|---|
| Phases 0–9 | Portable/domain/runtime/application foundations, Candle-only local composition, bounded lifecycle ownership, the workspace taxonomy, and Rust-native policy tooling. |
| Phase 10 | Maintained sampling and hosted-E0 Criterion targets, synthetic lifecycle/process evidence, and the historical external CPU product baseline. |
| Phase 11 | Mandatory CPU plus explicit no-fallback CUDA ordinal 0 for the exact executed RTX 5070 Ti row, with E1/Slint selection and historical CPU/CUDA product evidence. |
| Phase 12 | Transaction-bound per-tensor Safetensors loading, required-versus-observed scalar truth, exact final/loading peaks, mixed F16/F32 and BF16/F32 required layouts, and retained failed-load ownership. |
| Pristine ownership amendments | Whole-shard identity/selective materialization (`d4a1e43`), exact/unverified runtime ownership (`b43d0f4`), and frontend-neutral retained application state plus `LAM1` v3 (`1f91cba`). |

No later product capability is implied by these closures.

## Completed maintenance closure: infrastructure truth

The non-product implementation completed in this order:

1. declare every workspace package's project role in its manifest;
2. enforce the generic layer DAG, actual acyclic domain graph, dependency-kind distinctions, explicit exceptions, and exact CUDA topology;
3. register every maintained Cargo benchmark target and compile only those exact targets in `cargo xtask verify`;
4. isolate native, portable, policy, nursery, and CUDA job targets with disk preflight/observation and unconditional cleanup;
5. replace CUDA test-name enumeration with dedicated whole-suite targets;
6. simplify external evidence to public E1 observation and advance only the current schema contract;
7. reconcile ADRs, current reference, support state, remote Phase 12 evidence, and execution owners;
8. run every locally available isolated validation gate, record disk observations and unavailable CUDA/static-lint prerequisites, and create one commit without pushing.

Remote current-tree evidence is necessarily post-push work and must not be claimed by this local closure.

## Closure outcome and parked evidence

Available local closure established:

- architecture, hygiene, verification-plan, and executable harness-free CUDA-boundary fixture tests;
- `cargo xtask verify` from a fresh isolated native target, including only `runtime-benchmarks/runtime` and `sampling/sampling_pipeline` as release benchmarks;
- both named portable matrices in separate fresh targets;
- locked dependency policy, offline Markdown links, formatting, and whitespace checks;
- recorded target/filesystem observations with the pre-existing root target unchanged; and
- available workflow review without claiming an unavailable `actionlint` result.

The exact CUDA feature graph was attempted and stopped in `cudarc` because `nvcc` and NVIDIA devices are absent from this agent environment. The three current-tree hardware suites therefore remain unexecuted here. This is an explicit acceptance gap, not a substituted earlier result or a Rust/CUDA product failure. The resulting coherent local commit is reported externally and is not pushed by this package.

Post-push acceptance must observe hosted native/portable/policy jobs, the redesigned self-hosted CUDA whole-suite job, and hosted disk low-water evidence on the exact commit.

## Planned successor — not ratified

The next product direction remains workflow/workspace/authority, but no implementation phase is active. A future ratification must define at least:

1. versioned workflow, node, port, artifact, workspace, authority, capability, budget, and endpoint schemas;
2. the role declaration for any new portable SDK, runtime capability, provider adapter, headless application, plugin package, or tool;
3. a minimal general workflow runtime and headless host boundary;
4. durable run/workspace provenance and explicit commit authority; and
5. evidence that a second execution-target category belongs at a coarser endpoint boundary rather than inside local E0.

Plugins/connectors, provider/peer targets, durable context search, long-lived nodes, and a visual control center remain inactive later tracks.

## Operating rules

1. Start acceptance from a reviewed tree and use one Cargo process at a time.
2. Keep generated artifacts in one named isolated target or outside the repository; do not create root `target/` accidentally.
3. Preserve public APIs unless a ratified work package requires a real product consumer.
4. Keep correctness, synthetic performance, external product evidence, process sampling, whole-device sampling, and CI infrastructure observations distinct.
5. Record architecture changes in accepted ADRs and current behavior in project reference, not execution duplicates.
6. Do not claim remote success until the exact run exists.
7. Keep the workflow/workspace successor inactive until a separate decision ratifies it.
