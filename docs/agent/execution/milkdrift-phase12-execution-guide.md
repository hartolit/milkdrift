# Milkdrift Phase 12 execution guide

## Decision

Execute Phase 12, but do **not** run the original monolithic prompt as one agent task.

The technical objective remains aligned with Milkdrift's values:

- it replaces declared configuration metadata with observed artifact facts where correctness requires observation;
- it makes memory admission conversion-aware rather than approximate;
- it strengthens prepare, validate, commit, cleanup, and ownership semantics;
- it increases the usefulness of the local Candle execution endpoint without making Candle the definition of Milkdrift;
- it improves model compatibility needed by future operator-defined workflows.

Phase 12 is not the workflow-runtime phase. It hardens one execution endpoint that future workflows can select. Keep workflow definitions, workspaces, plugins, provider targets, peer execution, and the control center out of this implementation.

## Preconditions

Before starting the implementation prompts, commit the current strategic documentation direction so it cannot be flattened by a loader-focused agent:

- `docs/vision.md` remains the canonical, authentic vision document;
- `docs/agent/execution/analyzer.md` records the operator-programmable workflow direction;
- the README and architecture direction clearly state that local inference is one execution kernel inside the broader intended system;
- `task-graph` and `corrective-workflow` are treated as incubating workflow foundations, not removed as irrelevant experiments;
- `application-runtime` is treated as the current application kit, not the future workflow core.

This is a small governance checkpoint, not another implementation phase.

## Corrected Phase 12 boundary

The original Phase 12 objective is retained:

```text
configuration-declared scalar metadata
+ observed per-tensor Safetensors layout
+ selected execution scalar
+ exact conversion-aware resource plan
+ prepared load transaction
+ explicit partial-load cleanup
```

The following refinements are mandatory:

1. **Safetensors details stay in the adapter.** Tensor names, offsets, shard DTOs, and format-specific metadata must not enter portable workflow or generic runtime contracts.
2. **Generic layers receive only facts they need.** E0 may require a plan, actual execution facts, aggregate footprint, and a transaction/cleanup contract. It does not need a public list of every Safetensors tensor.
3. **E1 does not reproduce Candle policy.** `application-runtime` may expose stable source/execution facts, but it must not select per-tensor conversions or reject a model because configuration metadata does not equal every observed tensor dtype.
4. **The frontend remains thin.** Do not add a tensor-inspection interface or expand the Slint product. Make only compatibility changes required by the public API.
5. **Configuration metadata is not observed truth.** A declared scalar may remain useful metadata or an execution-policy input, but it cannot prove tensor homogeneity.
6. **Partial loading must have an ownership answer.** Phase 12 is not complete if a failed CPU or CUDA load can leave resources whose ownership and cleanup state are merely assumed.
7. **Evidence does not become a product dependency.** Project-authored mixed-dtype fixtures are deterministic gates. A pinned external checkpoint is optional/manual evidence and must not become a mandatory network download or committed weight fixture.
8. **Do not implement future workflow abstractions here.** This phase should make the local target more truthful and usable, then stop.

## Why the work is split this way

The split follows stable ownership boundaries rather than dividing each requirement into tiny chronological tasks.

### Segment 1 — core model loading and runtime ownership

Owns:

- portable backend/load contracts;
- E0 admission, validation, accounting, and retained cleanup;
- the Candle Safetensors inspection and load path;
- deterministic backend and adapter fixtures.

These areas must be designed together because the adapter cannot truthfully promise a load transaction that E0 has no way to own or validate.

### Segment 2 — artifact and application integration

Owns:

- immutable Hugging Face artifact resolution;
- E1 source and execution fact translation;
- model selection/load validation;
- persistence compatibility;
- thin frontend adaptation only where compilation requires it.

This segment starts only after the core transaction and public facts are stable.

### Segment 3 — validation, evidence, CI, and project truth

Owns:

- benchmark/evidence observers;
- CPU and CUDA gates;
- optional external-model procedure;
- support matrices and documentation closure;
- the canonical full verification run.

This keeps evidence machinery from distorting production APIs while still requiring honest closure.

## Execution order

Run the prompts sequentially:

1. `milkdrift-phase12-core-loader-runtime.md`
2. `milkdrift-phase12-application-artifact-integration.md`
3. `milkdrift-phase12-validation-project-truth.md`

Do not run them concurrently. The second segment consumes the first segment's public contracts, and the third validates the complete result.

Each segment should begin from a clean, reviewed working tree and end at one coherent commit boundary. Do not squash away a failed design experiment until its replacement is understood; inspect the final diff before committing.

## Required handoff between segments

After Segment 1, record:

- the final meanings of declared scalar, observed layout, and execution scalar;
- whether `ModelLoader` or its preparation/load contract changed;
- the exact CPU and CUDA peak-memory formulas used by the implementation;
- how partial load failure is owned, synchronized, retried, retained, or reported;
- which facts are adapter-private and which cross E0.

After Segment 2, record:

- the final E1 public vocabulary;
- whether persistence changed and how old records remain readable;
- which compatibility checks were removed, retained, or relocated;
- confirmation that Slint gained no new product responsibility.

Segment 3 must use those handoffs as evidence, not infer semantics from names.

## Stop conditions

Do not declare Phase 12 complete when any of these remain unresolved:

- the plan is still derived by scaling complete file sizes rather than inspecting tensor payloads;
- unsupported tensor dtypes are discovered only after device allocation begins;
- planning and loading may use materially different inspected facts without detection;
- failed partial CUDA loading can lose the only cleanup owner;
- E1 still treats configuration-declared scalar metadata as proof that every tensor has that dtype;
- a mixed-dtype fixture passes only because validation was removed rather than replaced with a reviewed conversion policy;
- documentation claims generic mixed-dtype, CUDA, or model compatibility beyond executed evidence;
- full CPU verification is not green;
- required CUDA evidence is claimed without running on the accepted hardware matrix.

If external checkpoint access or CUDA hardware is unavailable, leave that evidence explicitly pending. Never replace missing execution evidence with a claim.

## What should follow Phase 12

Once Phase 12 is closed, stop broadening the Candle loader for its own sake. Return to the strategic roadmap:

1. ratify workflow, workspace, artifact, authority, node, endpoint, and plugin schemas;
2. build a minimal general workflow vertical slice;
3. express corrective behavior as a configurable template;
4. add durable workspace and reactive execution semantics;
5. add a second execution-target category without pretending it is a local E0 backend.

Phase 12 earns its place by making the first local endpoint trustworthy. It should not postpone the operator-programmable architecture indefinitely.
