# Milkdrift project documentation

This directory owns current Milkdrift-specific reference. Reusable principles live
one level above; durable rationale and immediate execution state live under
`../agent/`.

## Current authority

| Question | Canonical document |
|---|---|
| What architecture and ownership model apply now? | [Project architecture](architecture.md) |
| How does a request execute end to end? | [Operation](operation.md) |
| What works and what evidence supports it? | [Implementation status](implementation-status.md) |
| How are claims reproduced? | [Validation](validation.md) |
| What benchmark methods and curated results exist? | [Performance evidence](performance.md) |
| Which crates and Cargo edges exist? | [Workspace boundaries](workspace.md) |
| Which dependencies and repository rules are enforced? | [Dependency policy](dependency-policy.md) |
| Which portable targets are claimed? | [Portability](portability.md) |

[Architecture principles](../architecture.md) are reusable beyond Milkdrift;
[project architecture](architecture.md) applies them to the actual repository.
Accepted and superseded rationale is indexed in the
[ADRs](../agent/decisions/README.md).

## Component guides

| Component | Unique detail owned here |
|---|---|
| [Candle backend](candle-backend.md) | Safetensors inspection, scalar compatibility, materialization, footprints, sequence reservation, and adapter cleanup |
| [Inference runtime](inference-runtime.md) | E0 admission, exclusive ownership, scheduling, backpressure, quarantine, unload, and shutdown |
| [Application runtime](application-runtime.md) | E1 resolution/load correlation, application state, chat/completion, persistence, retained cleanup, and worker coordination |
| [Desktop runtime](desktop-runtime.md) | Slint projection, event cadence, paths, and presentation boundary |
| [Corrective orchestration](orchestration.md) | Generic graph versus corrective capability semantics |
| [Model lifecycle](lifecycle.md) | Cross-component cancellation, cleanup, retention, and reclamation guarantees |

Component pages describe behavior only. The sole product support/evidence matrix
is [implementation status](implementation-status.md); exact procedures and run
results are not copied into component guides.

## Current boundary

The current product is the Candle/E0 path with optional E1 reference services and
a thin Slint host. CPU is mandatory/default. CUDA is explicit, non-default, and
has no CPU fallback; exact supported hardware and evidence are named only in the
status page. General workflows/workspaces/plugins/providers/peers and the control
center remain future direction.

Immediate execution state is in [current context](../agent/execution/current.md),
ordered program state in the [execution plan](../agent/execution/execution-plan.md),
and older exact-tree chronology in [history](../agent/execution/history.md).
