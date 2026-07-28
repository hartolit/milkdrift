# llm-app project documentation

This directory owns current llm-app-specific reference material. Reusable engineering doctrine lives one level above; agent-facing decisions, current execution context, plans, and historical closure evidence live under `../agent/`.

## Canonical ownership

| Question | Canonical document |
|---|---|
| What architecture does llm-app apply now? | [Project architecture](architecture.md) |
| What crates and dependency edges exist? | [Workspace boundaries](workspace.md) |
| What works in the current product tree? | [Implementation status](implementation-status.md) |
| How is the repository validated? | [Validation](validation.md) |
| How does E0 inference behave? | [Inference runtime](inference-runtime.md) |
| How does E1 application orchestration behave? | [Application runtime](application-runtime.md) |
| What does the native frontend own? | [Desktop runtime](desktop-runtime.md) |
| What are lifecycle and cleanup guarantees? | [Model lifecycle](lifecycle.md) |
| How does the Candle adapter behave? | [Candle backend](candle-backend.md) |
| How does the GGUF adapter behave? | [GGUF backend](gguf-backend.md) |
| How does corrective workflow execution behave? | [Corrective workflow](orchestration.md) |
| What performance evidence exists? | [Performance evidence](performance.md) |
| What dependency and supply-chain policy is enforced? | [Dependency policy](dependency-policy.md) |
| What portability targets are claimed? | [Portability](portability.md) |

A document may restate a small boundary when needed locally, but changing facts should be maintained in the owner above and linked elsewhere.

## Architecture and state

- [Project architecture](architecture.md) applies the reusable [architecture principles](../architecture.md) to llm-app and records the current F0/F1, E0/capability/E1, execution-target, backend, and frontend model.
- [Workspace boundaries](workspace.md) is the concrete crate inventory and dependency graph.
- [Implementation status](implementation-status.md) is the only product-level support matrix and validation-state page.
- Accepted project decisions are indexed in [architecture decisions](../agent/decisions/README.md).

## Runtime and frontend

- [Inference runtime](inference-runtime.md)
- [Application runtime](application-runtime.md)
- [Corrective workflow](orchestration.md)
- [Desktop runtime](desktop-runtime.md)
- [Model lifecycle](lifecycle.md)

These guides describe current behavior, ownership, failure semantics, and public boundaries. Roadmap sequencing belongs in the execution plan rather than these guides.

## Backends

- [Candle backend](candle-backend.md)
- [GGUF backend](gguf-backend.md)

Backend documents own backend-specific capabilities, limitations, native-resource behavior, and compatibility semantics. Product availability belongs in implementation status.

## Engineering and operations

- [Validation](validation.md)
- [Dependency and repository policy](dependency-policy.md)
- [Portable feature targets](portability.md)
- [Performance evidence](performance.md)

Procedures and measurements stay close to the domain that owns them. The status page records whether the current source baseline has satisfied required gates; it does not duplicate every command.

## Historical material

The [recovered implementation plan](implementation-plan.md) is retained as historical source material. It is not the active roadmap. The dense working set is [current execution context](../agent/execution/current.md), closed execution-phase evidence is consolidated in [execution history](../agent/execution/history.md), and the active program remains the [execution plan](../agent/execution/execution-plan.md).
