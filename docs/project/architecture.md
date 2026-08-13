# Milkdrift project architecture

Milkdrift applies the reusable
[Layered Workspace](../architecture.md#model-b-layered-workspace) model. This page
maps that model to the current repository. Exact members and Cargo edges live in
[workspace boundaries](workspace.md); decision rationale lives in the
[ADRs](../agent/decisions/README.md).

Milkdrift remains workflow-first: operator-defined workflows, scoped context,
explicit authority, and replaceable execution targets are the project center. The
implemented Candle/E0/E1/Slint stack is a local-inference foundation and optional
reference application kit, not the final workflow plane or control surface.

## Physical layers

```text
.cargo/             workspace-local Cargo configuration
tools/xtask/        architecture, hygiene, and verification policy
benchmarks/runtime/ outer measurement observer
crates/domain/      portable contracts and algorithms
crates/platform/    process-host primitives
crates/adapters/    vendor, artifact, filesystem, network, and storage integration
crates/runtime/     stateful resource and capability owners
crates/apps/        process, transport, event-loop, and presentation boundaries
```

Every tracked non-fixture package is a root workspace member and declares one
role in `[package.metadata.milkdrift]`. Roles and locations are validated rather
than inferred from names. The current role vocabulary is `tooling`,
`benchmark-observer`, `domain-foundation`, `domain-feature`, `platform`, `adapter`,
`runtime-foundation`, `runtime-capability`, `runtime-application`, and
`application`.

## Dependency direction

```text
apps / hosts
    -> optional application-runtime (E1 reference services)
        -> capability runtimes and inference-runtime (E0)
            -> platform, adapters, and domain algorithms
                -> domain-contracts

benchmark observers -> public product APIs only
tooling              -> repository metadata only
```

Dependencies point inward. Domain code never imports adapters, runtimes, or apps.
Adapters do not depend on runtimes. E0 and capability runtimes do not depend on
E1. Same-role runtime peers are not generally legal. Cargo's actual domain graph
must be acyclic; reviewed exceptional development or external edges are explicit
policy records, not a second list of ordinary legal edges.

`benchmark-observer` packages remain outside production. They may observe public
APIs but cannot become a dependency of product, applications, tooling, tests, or
another observer.

## Domain and capability ownership

`domain-contracts` is the F0 shared foundation. `tokenization`, `context-planner`,
`sampling`, and `task-graph` are F1 algorithms with honest `no_std` boundaries.
Vocabulary stays with its narrowest coherent owner; shared foundation is not a
dumping ground.

`task-graph` owns generic directed-work mechanics: topology, stable task identity,
attempt state, deterministic readiness, cancellation/blocking, and identity-only
artifact provenance. It does not own model policy, corrective stages, artifact
semantics, or output bounds.

`corrective-workflow` is a runtime capability. It owns a bounded data-defined
corrective schema/executor and its six-stage reference template. It is not the
general workflow/workspace runtime. A new capability runtime requires an
independently coherent state, lifecycle, reuse, or deployment boundary.

## E0 local inference ownership

`inference-runtime` is E0, the backend-independent single owner of local native
model and sequence resources. It owns prepared-load admission, exact aggregate
reservation, model generations, generation workspaces, scheduling, cancellation,
cleanup quarantine, unload, and terminal shutdown.

E0 validates a concrete loader's portable plan and loaded receipt without
reimplementing its artifact policy. Public clients hold generation-safe identity,
not shared model ownership. Provider and peer endpoints remain above E0 because
remote work has different ownership, cancellation, accounting, and transport
semantics.

## Optional E1 reference services

`application-runtime` is E1 for the current reference application. It coordinates
selection, immutable Hub resolution, device choice, persistence, one resident
model, completion, exact compatible chat, context planning, bounded decoded
output, retained cleanup, events, and worker shutdown.

Its concrete local composition is private:

```text
application-runtime
    ├── Hub worker -> hf-hub-adapter -> immutable artifact identities
    ├── HfTokenizer and request-local decoder
    ├── redb-storage
    └── HostedRuntime<CandleLlamaSource>
            -> inference-runtime (E0)
                -> candle-backend
```

E1 coordinates application behavior; it must not absorb every domain it calls.
Tensor compatibility stays in Candle, token scheduling in E0, algorithms in
domain crates, vendor/storage implementation in adapters, corrective state in its
capability runtime, and presentation in apps. Reconsider a coarse extraction only
when a second consumer, deployment, or execution kind reveals an independent
lifecycle—not for symmetry. Do not expose the composition as a façade with many
public backend/storage/tokenizer type parameters. This is the accepted boundary
from [ADR-0008](../agent/decisions/0008-capability-and-execution-boundaries.md) and
[ADR-0013](../agent/decisions/0013-candle-only-local-execution.md).

## Adapter and host boundaries

`candle-backend` owns Safetensors structure, required-tensor compatibility,
source verification, target-specific preparation, materialization, sequence
resources, and native cleanup. `hf-hub-adapter`, `hf-tokenizer`, and
`redb-storage` own their external integrations. Vendor types do not cross into
portable contracts or presentation.

`host-runtime` owns bounded channels, named threads, monotonic time, and typed
pull-oriented output storage. It owns no model, application, conversation, or
workflow state.

`desktop-slint` is a replaceable native host. It owns paths, the event loop,
callbacks, labels, and presentation. It submits coarse E1 operations and projects
structured E1 state; it does not reconstruct E0/E1 state machines or import
adapter internals. A headless, terminal, transported, or workflow host may choose
the coarse boundary appropriate to its semantics.

## Current execution composition

```text
desktop-slint (optional host)
    -> application-runtime (E1)
        -> inference-runtime (E0)
            -> candle-backend
                -> Safetensors + CPU or feature-gated CUDA
```

The opt-in CUDA chain is
`desktop-slint/cuda -> application-runtime/cuda -> candle-backend/cuda`. Default
features do not reach CUDA. GGUF, quantization, Metal, generic GPU aliases, and
automatic CPU fallback are absent. Exact support and evidence belong only in
[implementation status](implementation-status.md).

## Enforcement and future placement

`cargo xtask architecture` validates roles, locations, the generic inward DAG,
actual domain acyclicity, observer/tooling isolation, reviewed exceptions, CUDA
features, and maintained benchmark registration. `cargo xtask hygiene` validates
tracked repository policy, including documentation authority. See
[dependency policy](dependency-policy.md).

A future portable workflow or plugin SDK belongs in a domain role; a stateful
workflow engine in `runtime-capability`; provider/peer implementations in
adapters; a headless process in apps; and a control center as another host over
public schemas. The [execution plan](../agent/execution/execution-plan.md) must
ratify those packages before implementation.
