# Application Runtime Architecture Warning

## Purpose

`application-runtime` is a useful boundary, but it is also the part of the workspace most likely to become a catch-all as the product grows.

The risk is not that E1 exists. The risk is that every capability above inference is eventually implemented inside it because it already sits between the frontend and the rest of the system.

This document records that risk and the architectural direction to prefer if it materializes. It is not an execution plan and does not require a refactor now.

## What `application-runtime` should be

`application-runtime` should remain the frontend-neutral application façade.

Its core responsibilities are application-level behavior shared by different hosts:

- model lifecycle state;
- generation use cases;
- cancellation and unload policy;
- frontend-neutral application state and events;
- bounded output exposed to frontends;
- explicit application shutdown;
- coordination of lower capabilities required to perform those operations.

Frontends should continue to use this boundary instead of independently composing `inference-runtime`, model backends, tokenizers, persistence, and host workers. Removing E1 would move shared application behavior into every app and create duplicated state machines.

The boundary should instead stay narrow:

> `application-runtime` coordinates application behavior. It should not absorb every implementation or domain it coordinates.

## Main risk

E1 already sits where many concerns meet:

- inference runtime ownership;
- Candle composition;
- model resolution;
- Hugging Face tokenization;
- persistence;
- host workers;
- generation state and decoded output;
- unload and shutdown.

Some of these belong to the application façade. Others are infrastructure or separate domains.

The corrective workflow has now been extracted into its own capability engine because it already had independent state, lifecycle, ports, and tests. That is the kind of ownership split this warning is intended to encourage: not more crates by default, but separation after the boundary is proven.

If future work adds conversation history, prompt rendering, context planning, execution-target routing, memory, tools, permissions, provider transports, peer discovery, and diagnostics directly into the same crate, the result could become a monolith even though the workspace still contains many crates.

A warning shape would look like this:

```text
application-runtime
├── model lifecycle
├── backend composition
├── acquisition
├── tokenization
├── persistence
├── generation
├── conversations
├── prompt rendering
├── context planning
├── workflows
├── memory
├── tools
└── permissions
```

The concern is therefore ownership, not crate size alone.

## Ownership rule

A subsystem should live in `application-runtime` when E1 genuinely owns its application semantics. Needing to call a subsystem is not enough.

| Concern | Preferred ownership |
|---|---|
| Model lifecycle state | `application-runtime` |
| Generation use case | `application-runtime` |
| Cancellation and unload policy | `application-runtime` |
| Application state/events | `application-runtime` |
| Explicit shutdown | `application-runtime` |
| Candle/GGUF construction | native composition or adapter |
| Hugging Face resolution implementation | adapter/native composition |
| redb implementation | adapter/native composition |
| Context selection algorithm | `context-planner` |
| Prompt rendering logic | separate/internal renderer boundary when justified |
| Corrective workflow engine | `corrective-workflow` capability engine |
| Hosted-provider client/protocol | adapter/composition below application semantics |
| Peer discovery/transport | adapter or capability engine according to state/lifecycle |
| UI presentation state | app/frontend |

This distinction should also guide later memory, tool, and agent systems.

E1 may coordinate them. It should not automatically implement them.

## Concrete composition is the likely pressure point

The current runtime contains concrete Candle, Hugging Face, redb, and host-runtime types. That is acceptable while there is one real production composition.

Once Candle and GGUF both reach the application layer, or when a remote execution target becomes real, the project will have enough evidence to decide where concrete local wiring belongs.

If E1 is still dominated by local model infrastructure at that point, prefer a coarse split similar to:

```text
application-runtime
    frontend-neutral use cases
    application state
    commands and events
    lifecycle policy

local-model capability
    Candle/GGUF selection and E0 composition
    Hugging Face integration
    host worker construction
```

Persistence can receive its own coarse replacement boundary if another store/deployment proves that need. Do not move unrelated infrastructure into one new catch-all merely to make E1 look smaller.

Do not create this layer only for symmetry. A second working backend should reveal the actual common boundary.

Remote providers and peer nodes create a different seam. They should satisfy a coarse model-execution boundary above E0; they should not be inserted below E0 as if a hosted API owned local tensors and sequences.

## Do not genericize the entire façade

Moving composition out of E1 should not produce a public type such as:

```rust
ApplicationRuntime<I, T, S, H, C, ...>
```

That would expose implementation structure through the application API.

Replacement points should be coarse and justified by real substitution. Cold-path infrastructure can use closed enums, small service boundaries, or an internal composition object. Token-sensitive inference paths should remain statically dispatched where ownership or measurement supports it.

The public E1 API should describe what the application can do, not how its dependency graph is assembled.

## Corrective workflow extraction

The corrective workflow was the clearest subsystem with a reason to change independent from E1. It owns artifacts, attempts, diagnostics, validation, review, revision, retries, and workflow events without sharing the hosted model lifecycle.

It now lives as a capability engine:

```text
engines/
├── inference-runtime
├── corrective-workflow
├── application-runtime
```

This is not a template to create an engine for every feature. The extraction is justified by existing state, lifecycle, ports, tests, and reuse pressure.

## Conversation and context

Conversation support will put more pressure on E1, but related concerns should remain distinct:

- conversation operations and application conversation state may belong in E1;
- context selection remains `context-planner` behavior;
- prompt rendering is model-compatibility logic;
- tokenizer implementation remains infrastructure;
- persistence remains storage infrastructure;
- message presentation remains in the frontend.

A chat feature should connect these capabilities rather than collapse them into one runtime module.

Conversation state must also remain independent from where generation happens. A message should not carry a Candle source, local `ModelHandle`, provider request object, socket, or peer connection as its semantic identity. Local E0, a peer node, and a hosted provider may have different capabilities, but switching execution target should not require a second conversation implementation.

The common model-execution seam should be coarse: complete request admission, target capabilities, cancellation intent, bounded output, usage, and terminal state. E0 remains the local token/resource engine underneath the local implementation.

The same rule applies later to memory, tools, permissions, and specialist agents.

## Warning signs

Revisit the structure when several of these appear:

- one feature requires changes across many unrelated E1 modules;
- E1 directly imports multiple concrete backend implementations;
- backend-specific types reach frontend-facing APIs;
- workflows, memory, tools, or context internals dominate the public surface;
- provider SDK or peer-transport types enter conversation/application-domain state;
- a hosted service is forced into E0's backend contract despite not owning local model resources;
- tests for one application concern require constructing many unrelated systems;
- `runtime.rs`, `state.rs`, or `lib.rs` become routing points for unrelated domains;
- broad re-exports turn internal subsystem types into application API;
- duplicated application state machines appear for different backends or frontends;
- temporary adapters and compatibility paths survive after a migration.

These are reasons to inspect ownership, not automatic reasons to create another crate.

## Refactoring standard

When a structural change is justified, it should be subtractive.

A migration is not complete when the new structure works while the old structure remains behind compatibility layers. Superseded code should be removed.

Review and delete obsolete:

- types and constructors;
- re-exports;
- state fields;
- compatibility adapters;
- error variants;
- conversion helpers;
- tests and fixtures tied only to the old design;
- dependencies;
- documentation and terminology;
- dead modules and feature flags.

The workspace is still internal and pre-1.0. Backwards compatibility should not preserve a known-wrong architecture.

## Likely long-term shape

If future integration justifies it, a clean structure could resemble:

```text
apps/
    desktop-slint
    TUI / headless host / transported frontend
        │
        ▼
application-runtime
    frontend-neutral application behavior
        │
        ├── corrective-workflow
        ├── context-planner
        ├── other proven capability engines
        └── model execution
              ├── local -> inference-runtime -> Candle/GGUF
              ├── peer  -> peer transport
              └── hosted -> provider adapter
```

This is a direction, not a required crate map.

The important properties are:

1. frontends do not rebuild application orchestration;
2. `application-runtime` remains a coherent façade rather than the entire AI system;
3. algorithms remain owned by their feature domains;
4. vendor and platform implementation stays outside application semantics where practical;
5. E0 remains a local inference engine rather than a universal remote-provider abstraction;
6. new boundaries are created only when ownership, lifecycle, replacement, testing, or reuse evidence justifies them.

## Guidance

Do not redesign `application-runtime` solely because it may become bloated later.

Continue using E1 while real product integrations expose its pressure points. When new capabilities are added, check whether E1 is **coordinating** them or **absorbing** them.

The intended result is a small application façade surrounded by independently owned capabilities: not independent apps that each rebuild the runtime, and not one runtime crate that eventually owns everything.
