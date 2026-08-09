# Milkdrift orchestration foundations: remove fixed-flow assumptions before workspace design

## Objective

Clean the existing `task-graph` and `corrective-workflow` foundations so they do not hardcode one six-stage corrective procedure into the architecture that will later support operator-defined workflows, context workspaces, external execution targets, plugins, recursion, and configurable authorities.

This is not the implementation of the full agentic workspace program. It is the correction of existing foundational abstractions so they either become genuinely reusable primitives or are clearly narrowed to a template/experiment.

## Read first

Read:

- `docs/vision.md`
- the operator-programmable workflow and workspace direction in `README.md` and `docs/agent/execution/analyzer.md`
- `docs/project/architecture.md`
- `crates/domain/task-graph`
- `crates/runtime/corrective-workflow`
- their tests, READMEs, architecture-policy registration, and current consumers

## Owned area

Primary ownership:

- `crates/domain/task-graph`
- `crates/runtime/corrective-workflow`
- closely related portable orchestration vocabulary
- architecture-policy entries specific to these packages
- focused documentation and examples for their actual status

Do not change local model loading, E0, E1, providers, networking, or frontends in this prompt.

## Problems to correct

The current generic-looking task graph contains workflow-specific concepts such as:

- `TaskKind::{Draft, Review, CompileCheck, Validate, NormalizeDiagnostics, Revise, ...}`;
- `ModelPolicy` containing local `ModelId` and `BackendId`;
- artifact kinds/roles shaped around the corrective example;
- a success-only dependency model presented as general orchestration.

`corrective-workflow` then describes its six-stage flow as canonical and implements a large fixed executor.

These choices conflict with the durable vision that workflows are operator-defined data and that local models, provider models, deterministic tools, editors, documents, peers, and plugins are all possible execution capabilities.

## Required outcome for `task-graph`

Make `task-graph` a small portable compiled-run graph primitive rather than a hidden AI workflow schema.

It should own only concepts that are truly generic, such as:

- stable node and edge identity;
- acyclic dependency validation for one compiled run plan;
- caller-owned validation/runtime scratch;
- attempt-safe state transitions;
- bounded ready-node enumeration;
- generic input/output port or artifact-reference identities where justified;
- generic retry/attempt limits where they belong to execution rather than a model.

Move model selection, corrective stages, diagnostics, Rust compile checks, and other template-specific semantics out of the generic graph.

Do not force future recursive workflows to be represented as graph cycles. Document the intended distinction:

```text
versioned workflow definition / loops / child runs
    -> compiles or expands into one or more bounded run DAGs
    -> task-graph validates and tracks one run plan
```

Keep `task-graph` `no_std`, allocation-free in its validated/runtime operations, and independent from local inference identifiers.

Use opaque stable type IDs or caller-defined metadata only when needed; do not invent the final plugin ABI prematurely.

## Required outcome for corrective behavior

The six-stage corrective flow must no longer masquerade as a general workflow engine.

Choose the cleanest of these based on the resulting code:

- represent it as a configurable template assembled from generic orchestration primitives;
- rename/reclassify it as a corrective template package;
- move it under an explicit experimental/incubating area if it cannot yet satisfy the general boundary.

Defaults must be data, not scheduler branches.

At minimum prove that the reusable execution mechanism can express two materially different flows without changing executor internals, for example:

```text
Draft -> Validate -> Finish
```

and:

```text
Draft -> Review -> Revise -> Validate
```

The correction count, model target, validator, artifact inputs, and terminal sink must not be compiled into one fixed call order.

Do not implement provider connectors, persistent workspaces, plugins, or the visual control center here. Leave clean extension points and honest status instead of speculative APIs.

## Code quality

The current corrective executor and library modules are very large and use multiple `too_many_lines` / `too_many_arguments` allowances.

Refactor by responsibility:

- graph/template compilation;
- execution state;
- artifact transaction;
- bounded output sinks;
- retry policy;
- events/outcomes;
- corrective-specific node handlers.

Use typed context structs instead of long parameter lists. Remove lint suppressions that structural refactoring makes unnecessary.

Avoid dynamic dispatch in low-level portable graph operations. Coarse workflow ports may use a deliberate dynamic registry later, but do not force that decision in this cleanup.

## Tests

Add or adapt tests for:

- graph core has no model/backend/corrective semantic dependency;
- duplicate/cyclic/unknown-edge validation remains bounded and deterministic;
- attempt tokens reject stale completions;
- two different workflow templates execute through the same generic mechanism;
- correction/retry count is configuration data;
- a template cannot access undeclared artifacts;
- failed artifact/event transactions roll back atomically;
- output bounds remain atomic and non-truncating;
- no fixed six-stage order is required by the core;
- portable WASM and embedded checks still pass.

## Validation

At minimum:

```text
cargo test --locked -p task-graph -p corrective-workflow
cargo clippy --locked -p task-graph -p corrective-workflow --all-targets -- -D warnings
cargo check --locked --target wasm32-unknown-unknown --lib -p task-graph
cargo check --locked --target thumbv7em-none-eabihf --lib -p task-graph
cargo fmt --all -- --check
git diff --check
```

Use the actual package name if corrective behavior is renamed.

## Finish

Commit one coherent orchestration-foundation cleanup. The final state must be honest: generic primitives are generic; corrective behavior is a template or experiment. Do not push.
