# Milkdrift

> **Design the AI system around the model.**

Milkdrift is a Rust-native runtime for building operator-defined AI systems. It connects local models, remote providers, tools, context workspaces, validators, and external data sources through versioned workflows rather than a fixed agent pipeline.

Operators decide how work is researched, routed, corrected, executed, and committed. Milkdrift supplies the lifecycle, permissions, provenance, resource bounds, execution targets, and extensibility needed to make those systems reliable and deeply embeddable.

> [!IMPORTANT]
> Milkdrift is pre-release. The current implementation provides the local inference foundation. The general workflow, workspace, plugin, provider, peer, and visual-control layers described here are the project's architectural direction and are not all implemented yet.

## The idea

Most people cannot create a new AI model. They should still be able to design the system that makes a model useful.

A Milkdrift workflow might:

```text
user task
  -> small research agents search books, the web, and local workspaces
  -> context assembler selects bounded evidence
  -> main agent produces a proposal
  -> validators compare it with specifications and tests
  -> correction repeats up to an operator-defined limit
  -> sanitizer and execution nodes prepare approved effects
  -> child workflows handle focused subtasks in their own workspaces
  -> results are committed to an editor, repository, document, or another system
```

That is only one template. Milkdrift does not require a main agent, a linear flow, a correction stage, a local model, or one final output destination.

**Defaults are workflow data, not hidden framework procedure.**

## What makes Milkdrift different

### Workflows belong to the operator

The canonical workflow is a versioned graph with typed nodes, ports, policies, targets, authority bindings, and resource limits. A visual editor, Rust builder, CLI, or shared template uses the same underlying definition.

### Context outlives a model window

Each deployed workflow is bound to a context workspace containing versioned artifacts, provenance, indexes, external references, and links to other authorized workspaces.

A model prompt is a temporary bounded view over that workspace—not the workspace itself.

### Truth and commit authority are explicit

A model normally produces a proposal. Specifications, tests, repositories, documents, humans, or other agents may validate it. A separate authorized node commits the accepted revision.

Milkdrift does not assume that the largest model, the first answer, or a designated "main agent" is automatically the source of truth.

### Local and remote intelligence can coexist

A workflow node may use:

- the local Milkdrift/Candle execution runtime;
- an authorized provider connector;
- a supported coding-agent process;
- a model in a datacenter;
- a trusted peer host.

Targets share coarse workflow semantics while retaining honest differences in ownership, privacy, cost, cancellation, and cleanup.

### Extensibility is a first-class requirement

Plugins and connectors can add:

- workflow node types;
- model execution targets;
- tools and effect handlers;
- artifact stores;
- search and memory indexes;
- triggers;
- validators and policies;
- external systems such as editors, files, repositories, documents, or databases.

Plugins receive explicit capabilities rather than ambient access to every file, secret, workspace, and network.

### Recursive work is explicit and bounded

Workflows may retry, loop, schedule future work, subscribe to changes, or spawn child workflows with focused context workspaces.

Depth, concurrency, cost, token use, storage, network egress, and external effects remain operator-controlled and observable.

### The local inference kernel is lifecycle-safe

Milkdrift's current local runtime adds real systems behavior above Candle:

- exclusive model and sequence ownership;
- transactional load and request admission;
- bounded scheduling and output backpressure;
- cancellation safe points;
- cleanup retry and quarantine;
- retained resource accounting;
- unload and explicit shutdown;
- truthful CPU/CUDA target reporting with no silent fallback.

Candle executes tensors and models. Milkdrift controls how that execution lives inside a long-running system.

## Architecture

```text
Hosts and projections
Rust SDK · headless host · CLI/TUI · editor integration · control center
                              │
                              ▼
Workflow control plane
versioned definitions · runs · scheduling · triggers · recursion · budgets
              ┌───────────────┼─────────────────┐
              ▼               ▼                 ▼
Context workspace       Plugin/node       Execution targets
artifacts                runtime           local · provider · peer
provenance               typed ports               │
authority bindings       capabilities              ▼
search/indexes            effects          Local inference runtime
              │                                    │
              ▼                                    ▼
Storage/connectors                         Candle adapter and Candle
```

The visual control center is a replaceable host over these schemas and APIs. It does not own workflow semantics.

## Core concepts

| Concept | Meaning |
|---|---|
| Workflow definition | Versioned graph of nodes, ports, edges, policies, target requirements, and bounds |
| Workflow deployment | Definition bound to a workspace, authorities, plugins, targets, credentials, and quotas |
| Workflow run | One live or durable execution with lineage, checkpoints, events, and terminal state |
| Workflow node | A logical operation such as retrieval, model invocation, validation, routing, execution, or commit |
| Execution endpoint | A local runtime, provider connector, process, or trusted peer that performs work |
| Context workspace | Durable artifact and context environment associated with a deployment |
| Context view | Bounded projection of workspace artifacts supplied to one node invocation |
| Artifact | Versioned information or work product with provenance |
| Authority binding | Configured evidence, validation, working, derived, or commit authority for an artifact scope |
| Connector | Integration with an external system |
| Plugin | Package registering node, target, connector, index, trigger, or policy types |

## Current implementation

At the current reviewed foundation, Milkdrift already provides:

- portable domain contracts and algorithms;
- a backend-independent local inference runtime;
- exclusive model ownership and request scheduling;
- bounded token and text output paths;
- cancellation, cleanup quarantine, unload, and explicit shutdown;
- a first-party Candle adapter;
- mandatory/default CPU execution;
- explicitly selected CUDA execution for the exact validated matrix;
- Hugging Face model acquisition and tokenizer integration;
- a frontend-neutral application/reference composition;
- redb-backed preferences and model catalogue state;
- a thin Slint reference host;
- an incubating task graph and corrective workflow vertical slice.

Important limitations currently include:

- Candle is the sole local model backend;
- the supported model path is narrow unquantized Llama Safetensors;
- mixed-dtype repositories are not generally supported yet;
- workflow definitions, workspaces, plugins, external targets, peer execution, and the control center are not yet general product paths;
- current chat and conversation behavior belongs to the reference application layer, not the final workflow API.

The implementation-status document is authoritative for the exact support matrix.

## Why not just use Candle?

Use Candle directly when a short-lived program only needs to load a model, run one generation loop, and exit.

Use Milkdrift when a system needs to remain alive and control:

- model and request ownership;
- multiple or repeated work items;
- bounded output and backpressure;
- cancellation;
- load/unload transactions;
- cleanup failures;
- target capabilities;
- workflow composition;
- durable context and artifact lineage;
- external effects and commit authority;
- local, provider, or peer execution;
- thin replaceable hosts.

## Repository direction

Milkdrift is an **engine-centered monorepo**.

- publishable core and runtime crates form the center;
- local/provider/peer implementations remain adapters or execution targets;
- default behavior is shipped as editable workflow templates;
- a headless host proves direct embedding;
- the current Slint host remains temporary and thin;
- a future control center edits and observes the public graph model;
- experiments, benchmarks, and evidence tooling remain outside the default product graph.

A future `crates/core/` root is appropriate only for portable, vendor-neutral schemas and algorithms. It must not become a generic dumping ground.

## Roadmap

### Now — identity and local-execution correctness

- ratify the operator-programmable workflow identity;
- extend the authentic vision and rewrite the public documentation spine;
- keep task-graph and corrective-workflow as incubating foundations;
- complete Phase 12 mixed-dtype inspection, planning, loading, and cleanup work;
- tighten the local inference API and repository boundaries.

### Next — workflow and workspace foundation

- define versioned workflow, node, port, artifact, workspace, authority, capability, budget, and target schemas;
- implement a minimal general workflow runtime;
- add a headless host;
- express direct completion through a public workflow template.

### Then — configurable correction and durable context

- migrate corrective behavior into a general template;
- add persistent runs, workspace artifacts, context search, child workspaces, triggers, subscriptions, and external commit connectors;
- prove correction count, validator, target, and sink can change without scheduler code changes.

### Later — plugins, external targets, peers, and control center

- publish a plugin/connector SDK;
- add one real external execution target;
- add trusted peer execution over operator-provided connectivity;
- build a Blueprint/ComfyUI-like control center over stable schemas;
- experiment with spatial memory, advanced placement, and long-lived cooperating agents.

## Project values

- **Operator control:** framework policy never replaces workflow configuration.
- **Explicit context:** nodes receive scoped views, not ambient global memory.
- **Explicit authority:** proposal, validation, and commit are separate.
- **Explicit effects:** tools and external mutation require capabilities.
- **Bounded autonomy:** recursion and long-lived work remain observable and governed.
- **Native embedding:** Rust is the canonical API; transports are optional.
- **Truthful targets:** local, provider, process, and peer guarantees stay distinct.
- **Thin hosts:** UI semantics never become runtime semantics.
- **Replaceable backends:** Candle is the first local backend, not the project identity.
- **Scoped portability:** each crate states its real `no_std`, `alloc`, `std`, or native requirements.
- **Useful rigor:** architecture exists to enable more powerful systems, not to become an end in itself.

## Contributing

Before adding a feature, determine whether it belongs in:

| Change | Owner |
|---|---|
| Workflow, artifact, workspace, authority, capability, or target schema | portable core |
| Run scheduling, recurrence, triggers, checkpoints, child workflows | workflow runtime |
| Local model ownership, token scheduling, cleanup, unload | inference runtime |
| Candle model/device support | Candle adapter |
| Provider, peer, editor, document, filesystem, or tool integration | target/connector plugin |
| Context search or memory algorithm | context/index plugin or portable algorithm |
| Chat, conversation, acquisition, preferences | optional application services/template |
| Visual graph editing and live inspection | host/control center |
| Measurements and run evidence | evidence tooling |

A flow-specific behavior should normally be a node or template. A behavior required for every workflow to execute safely belongs in the runtime.

## Status and licensing

Milkdrift is under active architectural development. Current capability is intentionally narrower than the vision, and support claims require named implementation and validation evidence.

Licensed under **MIT OR Apache-2.0**.
