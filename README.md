# Milkdrift

> **Design the AI system around the model.**

Milkdrift is a Rust-native runtime for operator-defined AI systems. It is aimed at
systems where local models, remote providers, tools, context workspaces,
validators, and external data sources are composed through versioned workflows
instead of one fixed agent pipeline.

Operators decide how work is prepared, routed, checked, executed, and committed.
Milkdrift supplies explicit lifecycle, permissions, provenance, resource bounds,
execution targets, and extension boundaries.

> [!IMPORTANT]
> Milkdrift is pre-release. The implemented product is currently a local
> inference foundation and optional reference application, not the general
> workflow/workspace system described by the long-term vision.

## Why Milkdrift

- **Workflows belong to the operator.** Defaults should be versioned graph data,
  not hidden framework procedure.
- **Context outlives a prompt.** A prompt is a bounded view over durable,
  provenance-bearing workspace artifacts.
- **Proposal and authority are separate.** Models may propose; specifications,
  tests, people, or other systems validate and commit.
- **Execution targets stay honest.** Local, provider, process, and peer targets
  may share coarse workflow semantics without pretending their ownership,
  privacy, cancellation, and cleanup guarantees are identical.
- **Extensibility is capability-scoped.** Plugins and connectors should receive
  only the data and effects their declared role requires.
- **Autonomy remains bounded.** Retry, recursion, concurrency, cost, storage,
  network egress, and external effects remain visible and operator-controlled.

The full motivation—including clean model context, navigable long-term memory,
multi-machine execution, trust, and deeper system integration—is preserved in
[the project vision](docs/vision.md).

## Current system

```text
native host (the Slint app is the reference)
    -> application-runtime (optional E1 application services)
        -> immutable Hugging Face resolution, tokenizer, redb
        -> hosted inference worker
            -> inference-runtime (E0 ownership and scheduling)
                -> candle-backend
                    -> unquantized Llama Safetensors
                    -> CPU by default, or explicit supported CUDA selection
```

The current foundation provides:

- portable domain contracts and algorithms;
- a backend-independent local inference runtime with transactional loading,
  bounded scheduling/output, cancellation, cleanup quarantine, unload, and
  explicit shutdown;
- a Candle adapter that verifies immutable content, materializes only required
  tensors, plans loading and retained ownership separately, and reports actual
  execution facts;
- mandatory/default CPU execution and an explicit no-fallback CUDA path limited
  to the exact hardware evidence in the support matrix;
- immutable Hugging Face artifact resolution, tokenizer integration, redb state,
  frontend-neutral application services, and a thin Slint host; and
- generic task-graph mechanics plus an incubating data-defined corrective
  workflow engine.

General workflow definitions and runs, durable context workspaces, plugins,
provider/peer targets, browser transport, and a visual control center are not
implemented product paths. The canonical capability and evidence boundary is the
[implementation status](docs/project/implementation-status.md).

## Ownership model

Milkdrift uses a layered workspace:

```text
apps / hosts
    -> application and capability runtimes
        -> E0 local inference
            -> adapters / platform / domain algorithms
                -> domain contracts
```

Features own portable meaning, adapters quarantine vendors and I/O, runtimes own
stateful lifecycle, and applications own process/presentation concerns. The
[project architecture](docs/project/architecture.md) maps that model to the
current crates; the [operation guide](docs/project/operation.md) follows one
request through resolution, loading, generation, cleanup, unload, and shutdown.

## Repository

```text
crates/domain/      portable contracts and algorithms
crates/platform/    process-host primitives
crates/adapters/    Candle, Hub, tokenizer, and storage integrations
crates/runtime/     inference, corrective, and application runtimes
crates/apps/        process and presentation boundaries
benchmarks/         non-production measurement observers
tools/xtask/        architecture, hygiene, and verification policy
docs/               vision, current reference, decisions, and execution handoff
```

The canonical local repository gate is:

```sh
cargo xtask verify
```

See [validation](docs/project/validation.md) before making an evidence claim;
CUDA, external-model, portability, and remote results are distinct evidence
classes.

## Roadmap

The local-execution foundation is in maintenance and exact-tree acceptance. The
next product direction is workflow/workspace/authority, but it is not yet a
ratified implementation phase. A future program must first define versioned
workflow, artifact, workspace, authority, capability, budget, and execution-target
contracts plus a minimal headless workflow host.

Later tracks include durable context and provenance, capability-scoped plugins,
provider and trusted-peer targets, a replaceable control center, and research into
advanced context placement and long-lived cooperating systems. Current ordering
and activation state live only in the [execution plan](docs/agent/execution/execution-plan.md).

## Documentation route

Start with:

1. [Vision](docs/vision.md) for motivation and research direction.
2. [Project architecture](docs/project/architecture.md) for current ownership.
3. [Operation](docs/project/operation.md) for the end-to-end execution flow.
4. [Implementation status](docs/project/implementation-status.md) for support and evidence.
5. [Project documentation map](docs/project/README.md) for component detail.
6. [Validation](docs/project/validation.md) and [performance evidence](docs/project/performance.md) when producing or interpreting evidence.
7. The relevant [ADR](docs/agent/decisions/README.md) for durable rationale.

## Licensing

Milkdrift is licensed under **MIT OR Apache-2.0**, at your option.
