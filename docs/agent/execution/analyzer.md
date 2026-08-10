# Milkdrift strategic vision and architecture analysis

**Analysis date:** 2026-08-07
**Repository basis:** the previously reviewed `hartolit/milkdrift` snapshot at commit `a28008a369214e26522cf027977b67292962d058`, combined with the operator-provided workflow and context vision recorded on 2026-08-07
**Purpose:** correct the earlier engine-only framing, define Milkdrift's durable product identity, and establish an architecture that keeps workflows operator-programmable rather than hardcoded
**Scope:** project vision, core concepts, execution layers, extensibility, workspaces, authority, local and remote models, trusted networking, repository structure, crate publication, frontend role, documentation ownership, and roadmap. The earlier model-loading and technical-cleanup findings remain relevant where explicitly retained.
**Validation note:** this is a conceptual and static architecture review. No fresh repository fetch, build, benchmark, or CI run was performed for this revision.

---

## Executive correction

The previous strategic analysis was directionally right about thin frontends, direct Rust integration, explicit resource ownership, and the value Milkdrift adds above Candle. It was nevertheless centered one layer too low.

It described Milkdrift primarily as an AI execution runtime and treated workflows, tools, memory, and cooperating agents as later optional capabilities. The newly clarified vision shows that this is incomplete.

Milkdrift is intended to become:

> **A Rust-native runtime for operator-defined AI systems. Milkdrift lets an operator compose local and remote model executors, tools, context workspaces, validators, external sources, and effectful integrations into versioned, recursive workflows whose lifecycle, permissions, provenance, authority, and resource use remain explicit.**

The existing inference runtime remains essential. It is the local model-execution kernel and one of the strongest parts of the project. It should be publishable and usable independently. It is not, however, the complete product center.

The complete center is the combination of:

```text
workflow definition
+ workflow runtime
+ context workspace
+ artifact/provenance model
+ authority and commit bindings
+ extensible node/plugin model
+ execution-target abstraction
+ the existing local inference kernel
```

This changes several conclusions from the prior report:

- `corrective-workflow` and `task-graph` are not unrelated experiments to discard. They are early prototypes of a central future product area. Their current hardcoded shape should not become the final architecture, but their behavior and tests should be preserved as input to a general workflow runtime.
- A workflow runtime is justified. The earlier recommendation to avoid creating a third runtime is no longer correct.
- Workflows should not arrive after a long sequence of server, provider, or frontend work. The general workflow and workspace model should become the next major architectural program after Phase 12 stabilizes the local execution target.
- `application-runtime` should not become the workflow engine. Its current conversation, Hugging Face, redb, preference, and desktop-oriented composition remains a useful application kit and vertical slice, but the general workflow substrate must be lower, more neutral, and not shaped around a chat application.
- The visual control center is not the core runtime, but it is an important first-party projection of the core. It should edit and observe the same versioned workflow definitions that the Rust API, CLI, and headless host use.
- The previous `milkdrift-vision-draft.md` should **not** replace the existing `docs/vision.md`. It flattened the distinctive ambition into generic language about integration, tools, memory, and cooperating agents. Some paragraphs may be reusable in the README, but the existing authentic vision should be preserved and expanded.

The key project idea can be stated more simply:

> **Most people cannot create a new AI model, but they should be able to design the system around a model. Milkdrift should give them control over that system.**

---

# 1. Restated vision

Milkdrift should let an operator design an AI system rather than merely select a model and submit a prompt.

A workflow may, for example:

1. accept a task;
2. ask smaller agents to search books, the web, local documents, or other workspaces;
3. assemble only the relevant evidence into a bounded context view;
4. invoke a main agent on a local GPU, a remote provider, or a trusted peer;
5. validate the result against one or more authorities;
6. repeat correction up to an operator-configured limit such as three iterations;
7. pass the accepted result through sanitizing, formatting, security, or policy nodes;
8. execute approved effects;
9. spawn focused child workflows with their own context workspaces;
10. publish results to an editor, repository, document, filesystem, database, or another workflow;
11. keep specialized reactive agents subscribed to selected outputs and maintain their own assigned artifacts over time.

None of those steps is universally mandatory. The runtime must not assume that there is one main agent, one validator, one linear flow, one canonical memory, one final output sink, or one local model.

The operator defines:

- which nodes exist;
- how information flows between them;
- which execution targets they may use;
- which workspaces they may inspect;
- which sources carry authority for a given artifact or decision;
- where proposals are committed;
- which plugins and tools are available;
- how recursion, retries, branching, and child work are bounded;
- how long the workflow remains active;
- what must be approved by a human or another system;
- what data may leave a machine or trusted network.

Milkdrift supplies the runtime semantics that make this composition reliable, observable, portable, and difficult to misuse.

## 1.1 The project is not a hardcoded agent pipeline

The project should never encode the following as its immutable product flow:

```text
retrieve -> main model -> correct three times -> sanitize -> execute
```

That is a valuable **workflow template**, not the engine.

A default template may offer useful behavior immediately, but it must be represented using the same public workflow definition that operators use. A user should be able to inspect it, clone it, remove stages, replace nodes, alter authorities, change recursion limits, bind different models, and publish a modified template without patching Milkdrift's runtime.

The rule should be:

> **Defaults are data, not hidden control flow.**

## 1.2 The model is a component, not the system

A model supplies a form of computation. It does not automatically own:

- the workflow;
- durable context;
- task state;
- truth;
- tool permissions;
- external side effects;
- the final destination of its output;
- the authority to create or modify other workflows.

An agent is therefore not synonymous with a model. In Milkdrift, an agent should be understood as a configured workflow role or node that combines an execution target with instructions, a context view, tools, policies, and output bindings.

The same agent role may run on different execution targets. The same model may serve several roles. A workflow may contain no distinguished "main" agent at all.

## 1.3 Workflows are tied to context workspaces

A reusable workflow template describes structure and policy. A deployed workflow binds that template to a specific context workspace, target registry, authority set, plugin set, and resource policy.

This distinction preserves both goals:

- workflows can be shared and reproduced;
- each deployment can remain uniquely tied to its own context, data, permissions, and external systems.

A workflow run then executes within that deployment. Child workflows may receive a new workspace, a forked workspace, a restricted view of the parent, or an explicitly linked workspace according to policy.

---

# 2. Durable project identity

## 2.1 One-sentence public identity

> **Milkdrift is an embeddable Rust runtime for composing operator-defined AI workflows across local models, remote providers, tools, context workspaces, and trusted execution endpoints.**

## 2.2 Expanded mission

Milkdrift exists to make AI systems programmable at the systems level.

It should provide a coherent Rust API for defining workflows, binding them to context and authorities, registering execution targets and plugins, starting durable runs, observing events and provenance, controlling cancellation and recursion, and committing results through explicit effect boundaries.

It should preserve the existing runtime values:

- explicit ownership;
- bounded queues and work;
- truthful failure and cleanup reporting;
- no silent device or target fallback;
- backend and provider isolation;
- portable contracts;
- thin frontends;
- direct integration without mandatory HTTP, JSON, Python, JavaScript, or a daemon.

The project is differentiated not merely by running a model, but by allowing the operator to determine how intelligence is assembled around models.

## 2.3 What Milkdrift is

Milkdrift is intended to be:

- an embeddable AI workflow runtime;
- a context-workspace and artifact-coordination substrate;
- a local inference runtime with explicit ownership and cleanup;
- an execution-target abstraction spanning local, provider, process, and peer targets without erasing their differences;
- a plugin and connector platform;
- a durable, observable control plane for recursive and reactive AI work;
- a foundation for a visual workflow control center;
- a set of publishable Rust crates, with portable `no_std` or `no_std + alloc` foundations where truthful;
- suitable for applications, services, editors, games, custom operating systems, local nodes, and distributed systems.

## 2.4 What Milkdrift is not

Milkdrift is not primarily:

- a replacement tensor framework for Candle;
- a desktop chat application;
- a fixed multi-agent pattern;
- a universal business-process workflow engine;
- a mandatory OpenAI-compatible server;
- a bundled VPN or overlay network;
- a provider-account scraping layer;
- a single global memory shared implicitly by every agent;
- an assumption that one agent's answer is truth;
- an unbounded autonomous loop with hidden authority;
- a claim that the full native model stack currently runs under `no_std`.

The workflow runtime should stay specialized around AI-system concerns rather than attempting to replace general durable-workflow systems, dataflow engines, editors, or network overlays.

## 2.5 Durable differentiation

Milkdrift's strongest intended distinction is the combination of:

1. **Operator-defined composition.** The flow is a versioned graph, not framework-owned procedure.
2. **Context workspaces.** Durable context and artifacts exist independently of fleeting model windows.
3. **Explicit authority.** Sources, validators, proposal targets, and commit destinations are configured rather than implied.
4. **Target independence without false equivalence.** Local Candle, a provider API, a coding CLI, and a trusted peer can all execute work while retaining distinct capabilities and guarantees.
5. **Recursive and reactive workflows.** Workflows can spawn focused child work, subscribe to changes, and continue over time under explicit budgets.
6. **Extensible node types.** Retrieval, models, validators, tools, stores, editors, and custom capabilities can be supplied as plugins or connectors.
7. **Systems-level lifecycle.** Ownership, capacity, cancellation, backpressure, cleanup, persistence, and shutdown remain first-class facts.
8. **Thin visual and programmatic hosts.** A ComfyUI- or Blueprint-like control center edits and observes the runtime but does not define its semantics.

The existing inference runtime provides item 7 for local models. The next architectural program must build the other items above that kernel without weakening it.

---

# 3. The core conceptual model

Milkdrift needs a small set of durable concepts. These concepts should appear consistently in the README, vision, architecture, Rust APIs, persisted schemas, logs, and control center.

## 3.1 Workflow definition

A `WorkflowDefinition` is a versioned, serializable description of:

- node instances;
- typed ports;
- data edges;
- control edges;
- subworkflow references;
- triggers;
- policies;
- resource and recursion bounds;
- required plugin and target capabilities;
- configurable defaults.

It contains no live threads, model objects, provider clients, database handles, UI widgets, or implicit global state.

A definition should be inspectable and transportable. A missing plugin may prevent execution, but it should not make the graph unreadable or destroy unknown configuration.

## 3.2 Workflow deployment

A `WorkflowDeployment` binds a definition to one operational environment:

- a primary context workspace;
- authority and commit bindings;
- available execution targets;
- installed plugin versions;
- secrets and credential references;
- network and data-egress policy;
- persistent storage;
- resource quotas;
- schedules and event subscriptions.

This is the correct location for environment-specific facts. A shared workflow template should not contain another user's filesystem path, Google Drive identity, provider key, NetBird peer, or editor session.

## 3.3 Workflow run

A `WorkflowRun` is one live or durable execution of a deployment. It owns:

- run identity and lineage;
- current node states;
- pending inputs and outputs;
- child-run references;
- retries and recursion counters;
- consumed budgets;
- cancellation state;
- checkpoints;
- ordered events;
- terminal outcome.

A long-lived workflow should be represented as durable state plus triggers and subscriptions, not as one immortal hidden thread.

## 3.4 Workflow node

A workflow node is one configured operation in the graph. Examples include:

- model or agent invocation;
- web, book, file, or workspace retrieval;
- context assembly;
- validation and correction;
- routing, branching, joining, and selection;
- sanitization;
- tool or command execution;
- artifact transformation;
- external commit;
- human approval;
- child-workflow spawning;
- schedule or event trigger;
- subscription-driven maintenance of a derived artifact.

The runtime should ship only a small set of universal control and artifact primitives. Domain-specific behavior belongs in plugins and templates.

## 3.5 Execution endpoint

The term **node** is overloaded and should not also mean a physical machine.

Use separate terminology:

- **workflow node:** a logical operation in a workflow graph;
- **execution endpoint:** a local runtime, provider connector, process, or peer capable of executing model or tool work;
- **peer host:** another Milkdrift-capable machine or process reached through operator-provided connectivity.

This distinction will prevent major confusion once visual graphs and distributed execution coexist.

## 3.6 Context workspace

A `ContextWorkspace` is the durable information environment associated with a workflow deployment. It may contain or reference:

- source documents;
- prompts and rendered context views;
- model outputs;
- task descriptions;
- plans;
- tool results;
- code patches;
- tests and validation evidence;
- external-resource references;
- child-workspace links;
- indexes and search providers;
- provenance and authority metadata.

A model's context window is only a temporary projection from this workspace. It is not the workspace itself.

## 3.7 Artifact

An artifact is a versioned unit of information or work. It may be stored locally, embedded, content-addressed, or referenced through an external connector.

Examples:

- text output;
- prompt plan;
- search result set;
- code patch;
- document revision;
- test report;
- workflow definition;
- source-of-truth reference;
- execution proposal;
- approval decision.

Artifact versions should retain producer, inputs, target identity, configuration, timestamps or sequence, and validation history where available.

## 3.8 Context view

A `ContextView` is a bounded, policy-controlled projection of workspace artifacts for one node invocation.

It may be assembled by:

- explicit artifact references;
- search queries;
- recent events;
- role-specific filters;
- provenance constraints;
- token budgets;
- summaries;
- spatial, vector, keyword, graph, or custom indexes.

Agents should not automatically receive every other agent's raw context. Cross-workspace or cross-agent context access happens through explicit queries and grants.

## 3.9 Authority binding

The phrase "source of truth" is useful but too singular for the desired system. A workflow may use several kinds of authority at once.

Milkdrift should model **authority bindings** with at least these conceptual roles:

| Authority role | Purpose |
|---|---|
| Evidence authority | A source considered authoritative input, such as a specification, repository, book, test suite, or external document |
| Working authority | The current provisional artifact state inside the workspace |
| Validation authority | A source or system used to accept, reject, or correct a proposal |
| Commit authority | The destination whose accepted revision becomes canonical for a configured scope |
| Derived authority | An artifact maintained from other authorities, such as a tracker document or generated index |

One artifact scope may bind several authorities. Conflicts, precedence, merging, and approval must be workflow policy rather than hidden behavior.

## 3.10 Connector

A connector integrates an external system:

- filesystem;
- Git repository or editor;
- Google Drive;
- Word document;
- web search;
- provider API;
- local CLI agent;
- database;
- peer host;
- message bus;
- custom storage.

Connectors expose stable capabilities and consistency guarantees. They do not silently gain global read, write, network, or credential access.

## 3.11 Plugin

A plugin extends Milkdrift with one or more registered types:

- workflow node type;
- execution target;
- connector;
- artifact store;
- context index or search provider;
- trigger;
- policy or validator;
- visual inspector metadata.

Plugins must register versioned identities, configuration schemas, port schemas, required capabilities, and state migration behavior where state persists.

---

# 4. Concrete example: the described corrective coding workflow

The operator's example can be represented as a workflow rather than special runtime code:

```text
Task input
   │
   ▼
Research planner
   ├───────────────┬────────────────┬──────────────────┐
   ▼               ▼                ▼                  ▼
Book search     Web search     Workspace search   Specialist query
   └───────────────┴────────────────┴──────────────────┘
                           │
                           ▼
                    Context assembler
                           │
                           ▼
                       Main agent
                           │
                           ▼
              Corrective loop (maximum = 3)
                 ├─ validate against main output
                 ├─ validate against correction specification
                 └─ revise while inconsistencies remain
                           │
                           ▼
                   Sanitize / cleanup
                           │
                           ▼
                    Execution planner
                    ├─ apply approved effect
                    └─ spawn focused child workflow
                           │
                           ▼
                 Commit through Zed/repository
```

A parallel reactive path may subscribe to the main artifact stream:

```text
Main-agent artifact revisions
             │
             ▼
Niche tracking workflow
             │
             ▼
Maintain assigned document or code area
```

This example demonstrates several architectural requirements:

- research is fan-out/fan-in rather than forced linear dialogue;
- correction is a configurable loop with an operator-defined bound;
- validation consumes multiple authorities;
- the main agent is not automatically canonical;
- execution is separated from proposal generation;
- child work receives its own workspace and policy;
- a specialized workflow may continuously maintain a separate artifact;
- the final commit target may be Zed, Git, a document, or another external system;
- every stage can be replaced or omitted.

The default workflow may resemble this, but the engine must not privilege it.

---

# 5. Project constitution

These principles should become durable, normative project values.

## 5.1 Operator control before framework opinion

Milkdrift supplies composable primitives and safe execution semantics. The operator determines the flow, roles, targets, authorities, limits, and destinations.

## 5.2 Defaults are inspectable workflow data

No important default pipeline is hidden in application code. Official templates use the public graph format and can be copied, modified, disabled, or replaced.

## 5.3 No implicit main agent

A workflow may designate a coordinating role, several peers, a human, an external store, or no central agent. The runtime must not equate "main model output" with workflow truth.

## 5.4 Context is explicit and scoped

Nodes receive bounded context views. They do not receive ambient global memory. Cross-workspace search is a capability subject to policy and provenance.

## 5.5 Authority is explicit

Reading evidence, proposing an update, validating it, and committing it are distinct operations. A node may have one capability without the others.

## 5.6 Effects require capability grants

Filesystem mutation, command execution, network access, provider invocation, workflow creation, external commit, and secret use are explicit capabilities. Plugins and agents receive no ambient authority.

## 5.7 Recursive autonomy is bounded and observable

Retries, correction loops, child workflows, schedules, subscriptions, concurrency, token use, monetary cost, storage growth, and external effects have operator-defined policies. A persistent workflow may live indefinitely under quotas; an individual run may not expand invisibly without bounds.

## 5.8 Local and remote targets are not false equivalents

Local model ownership, provider requests, coding agents, and peer execution expose a shared coarse request model but retain distinct capability, cancellation, privacy, accounting, and cleanup facts.

## 5.9 Connectivity is operator-owned

Milkdrift does not force WireGuard, NetBird, Tailscale, ZeroTier, a SaaS control plane, or a custom overlay. It consumes operator-provided connectivity and applies its own endpoint identity and authorization policy above it.

## 5.10 Trusted network does not mean ambient trust

A LAN or VPN connection establishes reachability, not permission to read every workspace, use every model, or execute every tool. Peer identity and capability grants remain explicit.

## 5.11 Thin hosts, rich runtime

A control center, desktop, CLI, TUI, editor plugin, or web frontend may edit definitions and observe runs. It does not own scheduler semantics, authority resolution, context lineage, or target execution.

## 5.12 Hot execution remains specialized

The workflow control plane may use dynamic dispatch, allocation, and plugins at coarse node boundaries. The current local token/tensor path remains statically dispatched, preallocated where justified, and isolated from plugin complexity.

## 5.13 Portable foundations have honest boundaries

Definitions, identifiers, capabilities, graph validation, artifact references, and selected algorithms may support `no_std` or `no_std + alloc`. Native storage, threads, providers, plugins, networking, Candle, CUDA, and desktop hosts remain separate.

## 5.14 Failure preserves ownership and provenance truth

A timeout is not success. A proposal is not a commit. A generated answer is not validated truth. A terminal model result is not backend cleanup. A disconnected peer is not proof that an external effect did not occur.

## 5.15 Extensibility is designed, not simulated

Adding a capability should normally mean registering a node, connector, target, index, policy, or template—not editing the scheduler's central match statement or patching the desktop application.

---

# 6. Target logical architecture

```text
┌────────────────────────────────────────────────────────────────────────────┐
│ Hosts and projections                                                      │
│ Rust SDK · headless host · CLI/TUI · editor integration · control center  │
└────────────────────────────────┬───────────────────────────────────────────┘
                                 │ definitions, commands, events, views
┌────────────────────────────────▼───────────────────────────────────────────┐
│ Workflow control plane                                                     │
│ definition registry · deployment · run lifecycle · scheduler · triggers   │
│ branching · joins · loops · child runs · budgets · checkpoints            │
└──────────────┬─────────────────┬───────────────────────┬───────────────────┘
               │                 │                       │
               ▼                 ▼                       ▼
┌──────────────────────┐ ┌──────────────────────┐ ┌─────────────────────────┐
│ Workspace plane      │ │ Node/plugin runtime  │ │ Execution-target plane  │
│ artifacts            │ │ type registry        │ │ capability discovery    │
│ context views        │ │ typed ports          │ │ target selection         │
│ authority bindings   │ │ capability handles   │ │ request/cancel/output     │
│ provenance           │ │ plugin state         │ │ privacy/cost/locality     │
│ cross-workspace ACLs │ │ effect mediation     │ │ target-specific truth     │
└──────────┬───────────┘ └──────────┬───────────┘ └───────────┬─────────────┘
           │                        │                         │
           ▼                        ▼                         ├──────────────────┐
┌──────────────────────┐ ┌──────────────────────┐            ▼                  ▼
│ Storage/index        │ │ Tool/connectors      │  ┌──────────────────┐ ┌──────────────┐
│ redb or alternatives │ │ files, web, editor   │  │ Local model       │ │ Provider/peer│
│ spatial/vector/etc.  │ │ docs, commands       │  │ target adapter    │ │ target       │
└──────────────────────┘ └──────────────────────┘  └─────────┬────────┘ └──────────────┘
                                                             ▼
                                                   ┌────────────────────┐
                                                   │ Inference runtime   │
                                                   │ ownership/scheduler │
                                                   │ backpressure/cleanup│
                                                   └─────────┬──────────┘
                                                             ▼
                                                   ┌────────────────────┐
                                                   │ Candle adapter      │
                                                   └─────────┬──────────┘
                                                             ▼
                                                   ┌────────────────────┐
                                                   │ Candle/tensor stack │
                                                   └────────────────────┘
```

## 6.1 Foundation layer

The foundation owns portable vocabulary and validation:

- stable IDs;
- artifact references;
- workflow definitions;
- node and port schemas;
- capabilities and grants;
- authority descriptions;
- resource budgets;
- target capability descriptions;
- run events and outcomes;
- graph validation;
- selected context, sampling, and tokenization algorithms.

It owns no threads, filesystems, provider clients, model tensors, UI state, or database implementation.

## 6.2 Local inference kernel

The current `inference-runtime` remains the exclusive owner of local loaded models, sequences, request admission, token scheduling, bounded output, cancellation safe points, cleanup quarantine, resource accounting, unload, and shutdown.

It should remain independently usable by projects that want precise local model control without the workflow layer.

The workflow runtime invokes it through a coarse local execution-target adapter. Workflow plugins never receive Candle tensors, mutable model references, or access to the token scheduler.

## 6.3 Execution-target plane

The execution-target plane presents the common semantics that a workflow node needs:

- target identity;
- advertised capabilities;
- model or agent selection;
- request admission;
- context/input submission;
- streamed or batched output;
- cancellation intent and reported result;
- usage/cost facts;
- terminal outcome;
- locality, privacy, and data-egress facts.

It does not pretend every target owns the same resources.

Local targets may expose deterministic resource planning and explicit unload. Provider targets may expose quotas, billing, session behavior, and best-effort cancellation. Peer targets may expose connection loss and remote ownership. The shared API must preserve those distinctions.

## 6.4 Workflow runtime

A new general workflow runtime is justified. It should own:

- definition validation and compilation;
- deployment binding;
- run scheduling;
- node state;
- data and control dependencies;
- bounded fan-out and fan-in;
- explicit loops and retries;
- child-workflow spawning;
- triggers and subscriptions;
- checkpoint and resume;
- event publication;
- workflow-level cancellation;
- workflow budgets;
- coordination with workspace, target, and plugin runtimes.

It should not own model tensors, provider implementations, a visual editor, a specific correction algorithm, or a specific persistence database.

## 6.5 Workspace runtime

The workspace runtime should own:

- artifact identity and versions;
- immutable lineage;
- working references or named heads;
- context queries and views;
- authority bindings;
- workspace access policy;
- child-workspace creation and linking;
- subscriptions to artifact changes;
- index provider registration;
- transactional or best-effort external commit mediation;
- retention and storage budgets.

It may begin as a cohesive module inside the workflow runtime. It should become a separate crate only when portability, independent consumers, storage replacement, or lifecycle pressure justifies the split.

## 6.6 Plugin/node runtime

The plugin runtime should own registration and invocation at coarse operation boundaries.

A conceptual node contract needs phases similar to:

```text
validate configuration
prepare invocation
start or resume
poll/yield events
commit outputs or proposals
cancel
checkpoint
release
```

The exact Rust API should be designed from vertical slices rather than copied from this list. The important requirements are:

- typed configuration and ports;
- stable node-type identity and version;
- explicit capabilities;
- bounded output;
- cancellation;
- persistent state migration where needed;
- no ambient access to runtime internals.

## 6.7 Application services

The current application-oriented features remain useful above the workflow runtime:

- Hugging Face acquisition;
- tokenizer and chat-template support;
- model catalogue;
- conversation conveniences;
- preferences;
- user-facing summaries;
- default workflow templates.

These should become optional services, nodes, connectors, or SDK helpers. They should not define the lowest workflow or inference contracts.

## 6.8 Hosts

Hosts own only presentation and process policy:

- visual graph editing;
- control-center views;
- application launch;
- platform paths;
- user input;
- rendering events and artifacts;
- local credential setup;
- deployment management.

They consume schemas and runtime APIs. They must not hide private workflow semantics in frontend code.

---

# 7. Workflow graph semantics

The graph model is where extensibility can either be enabled or accidentally constrained.

## 7.1 Separate data flow from control flow

A single undifferentiated edge type will become ambiguous. Milkdrift should distinguish at least:

- **data edges:** artifact or value dependencies;
- **control edges:** scheduling, branch, loop, trigger, approval, or lifecycle dependencies;
- **authority edges or bindings:** where evidence is read, validation is performed, or results are committed;
- **workspace links:** which workspace views a node or child workflow may query.

The control center may render these differently, but the distinction belongs in the canonical workflow model.

## 7.2 Typed ports

Node ports should declare stable type or schema identities. The runtime should reject incompatible connections before a run begins where possible.

Not every value should be forced through JSON internally. In-process Rust nodes may exchange typed or opaque artifact handles. Serialization is required at persistence, plugin, process, or network boundaries and should be explicit there.

## 7.3 Explicit recurrence instead of ambiguous graph cycles

Operator freedom does not require unstructured cycles.

Milkdrift should support recurrence through explicit semantics:

- retry node;
- loop-until predicate;
- bounded corrective iteration;
- schedule next run;
- event subscription;
- spawn child workflow;
- map/fan-out over a collection;
- recursive template invocation under a depth or budget policy.

The static data-dependency graph may remain acyclic for one execution epoch while control nodes create new epochs or child runs. This makes recursion inspectable, checkpointable, and budgetable without preventing non-linear systems.

Raw cycles may later be supported if their operational semantics are fully defined. They should not be accepted merely because a visual editor can draw them.

## 7.4 Dynamic work creation

An execution or planning agent may determine that new work is needed. It should not mutate the running graph through hidden memory.

It should emit one of:

- a child-run request based on an existing workflow template;
- a proposed workflow definition;
- a proposed patch to a definition;
- a task artifact consumed by a generic dispatcher.

The workflow runtime then validates required plugins, permissions, recursion and cost budgets, workspace policy, and target availability before activation.

Self-modifying workflows remain possible, but modification is a versioned, reviewable operation rather than an unlogged side effect.

## 7.5 Reactive and persistent workflows

A workflow that "continues to live" should be represented as a deployed state machine with persistent subscriptions and triggers.

Possible triggers include:

- artifact revision;
- external document update;
- repository change;
- schedule;
- peer event;
- child completion;
- operator command;
- model or target availability;
- threshold or policy event.

Each trigger creates or resumes bounded work. The deployment can remain active indefinitely while individual runs remain observable and controlled.

## 7.6 Reusable subgraphs

Workflows should support versioned reusable subgraphs or templates. A corrective loop, research fan-out, code validation sequence, or publication pipeline can then be shared without being hardcoded into the engine.

Templates should declare parameters, required capabilities, authority slots, workspace expectations, and target requirements.

---

# 8. Context workspace architecture

The context workspace is central to the vision and should not be reduced to conversation history.

## 8.1 Workspace versus context window

A context window is ephemeral, model-specific, bounded, and often discarded after execution.

A workspace is durable, model-independent, searchable, versioned, and shared according to policy.

The prompt sent to a model is produced from a workspace view. It should retain references to the artifacts and transformations that produced it.

## 8.2 Workspace hierarchy and links

A child workflow may receive one of several explicit workspace relationships:

| Relationship | Meaning |
|---|---|
| Empty | Child begins with no inherited artifacts |
| Snapshot | Child receives a fixed view of selected parent artifacts |
| Fork | Child receives mutable working heads derived from the parent |
| Linked read | Child may query selected live parent scopes |
| Linked bidirectional | Parent and child may publish to configured shared scopes |
| External binding | Workspace projects an external system such as a repository or document store |

The default should be least privilege, not automatic global visibility.

## 8.3 Cross-workspace search

Agents may search artifacts from other active workflows when granted access. The result should be a bounded set of artifact references with provenance, not uncontrolled copying of another model's full prompt buffer.

Search providers may include:

- exact and keyword indexes;
- vector indexes;
- graph indexes;
- spatial or toroidal memory experiments;
- recency/event indexes;
- external search connectors;
- custom plugin indexes.

The core should define query/result contracts and provenance. It should not force one storage or embedding technology.

## 8.4 Context assembly as a workflow operation

Context selection, reduction, summarization, provenance filtering, and token budgeting should be node-configurable behavior.

The existing `context-planner` can remain an important algorithmic primitive, but the workflow decides:

- which artifacts are candidates;
- which are pinned;
- which index is queried;
- whether a small model filters them;
- which target's token limits apply;
- which authorities must be represented;
- how overflow is corrected.

## 8.5 Artifact lineage

Every meaningful derived artifact should be able to answer:

```text
Which workflow run produced me?
Which node and plugin version produced me?
Which input artifact versions were used?
Which execution target/model/settings were used?
Which validators accepted or rejected me?
Which external authority was eventually updated?
```

This does not require storing every private provider thought or hidden reasoning. It requires preserving observable inputs, outputs, decisions, and provenance.

---

# 9. Authority, proposals, validation, and commit

## 9.1 Do not give model nodes implicit write authority

A model node normally produces a proposal or artifact revision. A separate commit operation applies it to an external authority.

This distinction permits:

- validation before mutation;
- human approval;
- comparison with current external revision;
- conflict detection;
- rollback or rejection;
- separate read and write permissions;
- audit of what was proposed versus what was committed.

## 9.2 Multiple authorities

An error-correction workflow may consume:

- the main agent output;
- a correction specification;
- a test suite;
- a repository state;
- a human instruction.

None has to be universally dominant. The workflow defines their roles and conflict policy for the artifact being produced.

## 9.3 External commit targets

Connectors for Zed, Git, files, Google Drive, Word documents, databases, or other systems should expose:

- resource identity;
- readable version or revision token where available;
- proposed changes;
- validation or preview;
- commit/apply;
- conflict result;
- resulting revision;
- consistency and idempotency guarantees.

Exactly-once effects cannot be honestly promised for every external system. The connector must report its guarantees, and workflows should use idempotency keys or reconciliation where supported.

## 9.4 Continuously maintained derived artifacts

A niche agent that continuously tracks main-agent output can be modeled as a reactive workflow subscribed to selected artifact changes.

It owns a distinct derived artifact or external destination. It receives only the scopes it needs. Its updates do not need to pass back through a privileged main agent unless the workflow defines that relationship.

This allows many non-linear reporting and authority topologies without inventing special agent classes.

---

# 10. Extensibility and plugin architecture

Extensibility is not an afterthought. It is one of the primary product requirements.

## 10.1 Extension categories

The architecture should support separate extension interfaces for:

1. workflow node types;
2. execution targets;
3. external connectors;
4. artifact stores;
5. context indexes and search providers;
6. triggers and event sources;
7. validation and policy providers;
8. visual metadata/inspectors.

One universal `Plugin` trait will likely become too vague. A plugin package may register several narrowly defined extension types.

## 10.2 Plugin descriptors

A registered extension should declare:

- stable type ID;
- semantic version or schema version;
- human-readable name and documentation;
- configuration schema;
- input and output port schemas;
- required capabilities;
- whether it performs external effects;
- persistence/checkpoint support;
- cancellation behavior;
- portability and host requirements;
- migration support for saved configuration or state.

This metadata allows the same plugin registry to power validation, documentation, CLI generation, and a visual control center.

## 10.3 Trust tiers

Milkdrift should eventually support several extension trust models:

| Tier | Intended use |
|---|---|
| Compile-time Rust integration | Trusted, high-performance, direct embedding |
| In-process registered extension | Trusted host-controlled plugin set |
| Sandboxed portable component | Restricted third-party logic where a suitable component boundary is proven |
| Out-of-process connector | Provider, tool, or untrusted integration over an explicit protocol |
| Remote peer capability | Capability executed by another authorized Milkdrift host |

Do not begin with arbitrary Rust dynamic libraries as a stable plugin ABI. Rust ABI and third-party safety make this an unattractive first boundary. Begin with compile-time registration and explicit out-of-process connectors; add a portable sandbox boundary only after a real plugin use case proves the contract.

## 10.4 Capability-based access

A node executor should receive scoped handles rather than global services.

Examples:

- read artifacts in workspace scope A;
- write proposals to scope B;
- query external search connector C;
- invoke execution target tagged `local-private`;
- commit only to repository path D;
- spawn at most two child workflows from template E;
- use secret F only through connector G.

This enables powerful plugins without giving every plugin ambient filesystem, network, workspace, or secret access.

## 10.5 Coarse dynamic boundaries, static hot paths

Workflow extensibility does not require dynamic dispatch inside every generated token step.

The intended split is:

```text
workflow/node boundary       -> dynamic, extensible, coarse-grained
local model execution target -> stable adapter boundary
inference token loop         -> static, specialized, preallocated where measured
```

This preserves both extensibility and performance.

---

# 11. Local models, providers, coding agents, and peers

## 11.1 Execution targets are capability providers

A workflow model node binds to an execution-target requirement or exact target.

Target capabilities may include:

- supported input and output modalities;
- message versus token interface;
- context limit;
- streaming;
- structured output;
- tool invocation;
- session continuity;
- cancellation semantics;
- concurrency;
- cost/accounting;
- privacy and data retention;
- local/remote status;
- model identity and revision;
- determinism/seed support;
- availability.

Target selection must obey operator policy. Milkdrift should never silently route private local work to a provider because a local model is unavailable.

## 11.2 Local Candle target

The existing E0/Candle path is the first local execution target. It retains its stronger local guarantees:

- exact resource ownership;
- memory admission;
- device selection;
- request scheduling;
- backpressure;
- explicit sequence cleanup;
- model unload;
- bounded shutdown.

The workflow layer should not weaken or duplicate these semantics.

## 11.3 External provider target

An OpenAI, Anthropic, datacenter, or other provider integration should normally run through a local connector/translator that implements the execution-target contract.

This matches the operator's idea of treating an external service as an independent logical endpoint while keeping workflow state and credentials under local control.

The connector must use a provider-supported authorization surface. Milkdrift must not assume that a consumer chat subscription automatically grants programmatic API access, and it should not rely on scraping private web interfaces.

## 11.4 Coding-agent and CLI target

Codex, Claude Code, or another local coding tool may be exposed through a process connector when its supported interface and terms permit automation.

Such a target may differ from a stateless model API:

- it may maintain its own session;
- it may read and edit files;
- it may execute commands;
- it may have an independent permission system;
- its output may be patches and events rather than tokens.

The execution-target model should therefore be capability-based rather than reduced to `prompt -> text`.

## 11.5 Peer target

A trusted peer host may expose:

- model execution;
- tool execution;
- selected workflow-node types;
- workspace search or artifact access;
- child-workflow hosting.

Start with remote execution as the narrowest peer capability. Distributed workflow placement and shared workspaces should follow only after identity, authorization, provenance, failure, and partition semantics are proven.

---

# 12. Connectivity, trust, and network scope

## 12.1 Milkdrift should not implement the overlay network

The operator chooses connectivity:

- LAN or VLAN;
- WireGuard;
- NetBird;
- Tailscale;
- ZeroTier;
- another private network;
- a custom transport.

Milkdrift should accept configured addresses, discovery providers, or transport implementations. It should not force a network vendor or become responsible for NAT traversal and VPN lifecycle in the core project.

## 12.2 Application-level identity still matters

Reachability does not establish Milkdrift authorization.

A peer protocol should eventually provide:

- stable endpoint identity;
- explicit trust enrollment or allowlisting;
- capability advertisement;
- workspace and artifact access policy;
- request authentication;
- revocation;
- audit events;
- protocol/version negotiation.

## 12.3 Data placement and egress

Workflow policy should be able to state:

- artifact must remain local;
- artifact may remain inside selected trusted peers;
- artifact may be sent to provider class X;
- only a summary may leave the workspace;
- secrets may be used only by a named connector;
- execution must occur on a host with a capability or trust label.

This is essential once local and external models coexist in one graph.

## 12.4 Control plane versus transport

Milkdrift owns workflow intent, endpoint capability, authorization, and execution facts. The selected network supplies connectivity.

This separation keeps the project self-hostable without duplicating NetBird, Tailscale, WireGuard, or ZeroTier.

---

# 13. Public API direction

The prestige API should eventually expose the complete system without forcing every consumer to use every layer.

## 13.1 Two valid integration levels

### Low-level local inference API

For systems that want precise control over local models:

- start local runtime;
- inspect/load model;
- submit token generation;
- pull output;
- cancel;
- unload;
- shut down.

This is the current `inference-runtime` direction and remains valuable independently.

### High-level workflow API

For systems that want the Milkdrift vision:

- register targets, plugins, connectors, and stores;
- load or build workflow definitions;
- bind a deployment to a workspace and authorities;
- start, pause, resume, inspect, or cancel runs;
- subscribe to events and artifacts;
- manage child runs;
- checkpoint and recover;
- publish or commit through explicit connectors.

## 13.2 Conceptual public types

The exact names are not mandates, but the public design will likely need concepts equivalent to:

```text
WorkflowDefinition
WorkflowTemplate
WorkflowDeployment
WorkflowRunId
WorkspaceId
ArtifactId / ArtifactRef / ArtifactVersion
AuthorityBinding
NodeTypeId / NodeInstanceId
ExecutionTargetId / TargetCapabilities
CapabilityGrant
ResourceBudget
RunEvent / NodeEvent
PluginRegistry
TargetRegistry
ConnectorRegistry
```

The API should not expose Candle, Slint, redb, Hugging Face, NetBird, provider SDK, or UI graph-widget types in these contracts.

## 13.3 Builder and serialized definitions

Rust builders should create the same canonical workflow definition that can be loaded from a file or edited visually.

There must not be separate semantic models for:

- Rust-authored workflows;
- visual workflows;
- CLI workflows;
- shared templates.

## 13.4 API stability layers

A publication strategy should distinguish:

- stable foundational schemas and IDs;
- evolving workflow runtime;
- target and plugin SDKs;
- first-party integrations;
- unstable experimental nodes/templates.

Do not publish every internal crate merely because Cargo permits it.

---

# 14. `std`, `no_std`, and portability

The wider vision strengthens rather than weakens the need for precise portability tiers.

## 14.1 Proposed tiers

| Tier | Candidate scope |
|---|---|
| `no_std` | IDs, compact capabilities, lifecycle enums, borrowed descriptors, basic graph validation without owned strings where practical |
| `no_std + alloc` | Workflow definitions, artifact references, authority descriptions, owned graph schemas, selected context/sampling algorithms |
| `std` runtime | Workflow scheduling, checkpoints, plugins, channels, native storage, provider/process connectors |
| Native execution | Candle, CUDA, filesystem, Hugging Face, redb, desktop hosts |
| Custom host | Alternative platform implementation for a custom OS, embedded system, or unusual scheduler |

## 14.2 Do not force full-runtime `no_std` prematurely

The project should first make foundational schemas and algorithms portable. A full `no_std` workflow interpreter or model executor should be built only when a real host supplies timing, allocation, storage, scheduling, and communication requirements.

## 14.3 Portable workflow definitions are valuable even when execution is native

A custom OS or constrained controller may still inspect, validate, route, or generate workflow definitions while delegating model work to another endpoint. Portability is therefore useful even before full local inference is portable.

---

# 15. Repository scope and physical structure

## 15.1 Remain an engine-centered monorepo

The repository should not become strictly engine-only.

It should contain:

- publishable core and runtime crates;
- first-party execution targets and connectors;
- default workflow templates;
- a headless reference host;
- a thin visual/control-center host when developed;
- focused examples;
- benchmarks and tooling outside the product graph;
- incubating experiments clearly separated from supported APIs.

This permits atomic API changes and proves that the engine is genuinely consumable.

## 15.2 The frontend should remain optional and thin

The current Slint chat frontend is not the project identity. It should receive no major product investment unless it evolves into a useful control-center projection.

Keep it temporarily for regression and lifecycle integration. Add a headless host as the canonical minimal consumer. Once the headless path covers current behavior, decide whether the Slint application should:

- evolve into the control center;
- remain a small example;
- move to an examples area;
- be retired.

Do not remove all first-party hosts. An engine with no real consumer can develop a beautiful but impractical API.

## 15.3 `crates/core/` can make sense under a strict rule

The current `crates/domain/` root already approximates a portable core. Renaming it to `crates/core/` may improve discoverability for strangers, but only if "core" has a precise admission rule:

A core crate must be:

- dependency-light;
- vendor-neutral;
- UI-neutral;
- storage-implementation-neutral;
- free of process-host ownership;
- portable to its documented tier;
- part of the stable workflow, workspace, execution, or algorithm vocabulary.

Without this rule, `core` becomes a dumping ground.

A path migration should happen once as part of a ratified repository taxonomy, not through repeated aesthetic moves. Package/API naming matters more than folder names.

## 15.4 Suggested target shape

This is a target taxonomy, not an instruction to create every crate immediately:

```text
Cargo.toml
README.md

crates/
  core/
    milkdrift-core/            # IDs, capabilities, resources, portable shared contracts
    workflow-model/            # workflow definitions, ports, graph validation
    workspace-model/           # artifacts, authority, provenance, workspace schemas
    tokenization/
    context-planner/
    sampling/

  runtime/
    inference-runtime/         # existing local model kernel
    workflow-runtime/          # general workflow scheduler and durable run lifecycle
    host-runtime/              # threads, channels, clocks, bounded output

  adapters/
    candle-backend/
    hf-hub/
    hf-tokenizer/
    redb-storage/
    provider-*/                # only after real integrations
    peer-*/

  sdk/
    milkdrift/                  # curated facade after lower APIs stabilize

apps/
  headless/
  control-center/               # later visual projection
  desktop-slint/                # current temporary reference host

templates/
  direct-completion/
  chat/
  corrective-research/

examples/
experiments/
benchmarks/
tools/
docs/
```

Important cautions:

- `workspace-model` may begin inside `workflow-model`.
- `workflow-runtime` should evolve from proven behavior rather than a blank rewrite.
- provider and peer subtrees should not be created before a real implementation.
- the facade should not be published until it has a coherent vertical slice.
- moving paths before semantic ownership is ratified would create churn without clarity.

---

# 16. Current crate disposition under the corrected vision

| Current area | Revised role |
|---|---|
| `domain-contracts` | Seed of `milkdrift-core`; retain local backend contracts but separate truly general workflow/artifact vocabulary from backend-specific semantics |
| `tokenization` | Portable algorithm/service usable by context and model nodes |
| `context-planner` | Core context-view primitive; should be invoked by configurable context-assembly behavior rather than define one chat policy |
| `sampling` | Local inference primitive; remains below workflow layer |
| `task-graph` | Incubating workflow-model primitive; audit against typed ports, control/data separation, persistence, recurrence, and dynamic child runs before promotion |
| `host-runtime` | Retain as process-host infrastructure; likely supports both inference and workflow runtimes |
| `candle-backend` | First-party local execution adapter; Phase 12 remains valid |
| `hf-hub` | Optional acquisition connector/service, not core workflow semantics |
| `hf-tokenizer` | Optional tokenizer adapter used by model/context nodes |
| `redb-storage` | Current application storage adapter; may later implement workspace/checkpoint stores but should not define their contracts |
| `inference-runtime` | Publishable local execution kernel and major durable asset |
| `corrective-workflow` | Incubating vertical slice; extract general attempts, validation, retry, bounded output, artifact, and release semantics, then express correction as a template |
| `application-runtime` | Existing application kit/reference composition; not the future workflow core |
| `desktop-slint` | Thin temporary reference host; no longer roadmap center |
| `runtime-benchmarks` | Evidence observer; keep outside default product development path |
| `xtask` | Enforce durable layer rules, not every incidental dependency edge |

## 16.1 `corrective-workflow` migration direction

Do not simply rename the crate to `workflow-runtime`.

First identify which semantics are general:

- workflow artifact ownership;
- attempts;
- validator invocation;
- retry state;
- bounded outputs;
- event ordering;
- cancellation;
- artifact release;
- terminal outcomes.

Then identify which semantics are corrective-template-specific:

- one model/validator relationship;
- correction prompt construction;
- fixed retry meaning;
- a specific output sink;
- assumptions about linear progression.

Only the first group belongs in the general runtime. The second becomes a versioned template or plugin set.

## 16.2 `task-graph` audit

The current task graph should be evaluated against the target workflow model:

- typed ports and artifacts;
- separate control and data dependencies;
- static validation;
- versioned serialization;
- subgraphs/templates;
- explicit recurrence;
- child-run lineage;
- persistent node state;
- plugin type identity;
- missing-plugin preservation;
- authority bindings;
- workspace links.

It may evolve into the workflow definition core, remain a lower scheduling utility, or be replaced. Its existence should not dictate the final model.

## 16.3 `application-runtime` decomposition trigger

Do not immediately shatter E1 into micro-crates. Stop adding unrelated workflow behavior to it.

As the general workflow vertical slice emerges, migrate concerns deliberately:

- direct completion becomes a minimal workflow template over the local target;
- chat becomes an optional template/service;
- model acquisition becomes a connector or service;
- conversation history becomes one workspace projection, not universal state;
- application preferences remain host/application policy;
- device selection remains local-target configuration;
- frontend summaries remain host-facing adapters.

---

# 17. Documentation architecture

## 17.1 Status of `milkdrift-vision-draft.md`

It should not replace `docs/vision.md`.

Its useful material is limited to concise explanations of:

- direct Rust integration;
- owned execution;
- thin frontends;
- portability boundaries;
- Candle versus Milkdrift.

Those passages are better candidates for the root README or an architecture introduction.

The draft underrepresented or omitted:

- operator-defined graph composition;
- context workspaces;
- multiple authorities and commit targets;
- recursive child workflows;
- reactive long-lived workflows;
- plugin-defined capabilities;
- external model and coding-agent targets;
- user-owned connectivity;
- a Blueprint/ComfyUI-like control center;
- the central idea that users design the system around the model.

Delete it as a proposed replacement or rename it to positioning notes. Preserve the existing authentic vision and extend it.

## 17.2 `docs/vision.md` remains canonical

The existing vision should remain the deep, authentic, long-form source of purpose. Add sections covering:

1. operator-programmable intelligence;
2. workflows as versioned graphs;
3. context workspaces and cross-workspace search;
4. authority, proposals, validation, and commit destinations;
5. recursive and reactive workflows;
6. extensible plugins and connectors;
7. local, provider, coding-agent, and peer execution;
8. operator-owned networking;
9. the visual control center;
10. the principle that models are components and workflow design creates capability.

Do not rewrite the document into corporate product copy. Preserve first-person motivations, specific examples, uncertainty, and the unusual long-term ideas that make the project authentic.

## 17.3 Root README

The README should be the public compression of the vision, not a substitute for it.

Recommended order:

1. identity and one-sentence promise;
2. the problem: models are available, operator-controlled systems around them are not;
3. a concrete workflow example;
4. what Milkdrift owns versus Candle and providers;
5. current implementation status, clearly separated from the long-term vision;
6. architecture diagram;
7. current supported path;
8. roadmap;
9. contribution map;
10. links to vision, concepts, operation, status, and ADRs.

## 17.4 New cohesive concept documents

The project needs a small public concept set:

```text
docs/
  vision.md
  roadmap.md
  concepts/
    workflow.md
    workspace.md
    authority-and-provenance.md
    execution-targets.md
    plugins-and-connectors.md
    security-and-capabilities.md
  operation/
    local-inference.md
    workflow-runtime.md             # once implemented
  project/
    architecture.md
    implementation-status.md
    validation.md
```

These should explain concepts and operation cohesively. Phase history and agent execution records remain subordinate.

## 17.5 Current versus intended architecture

Every public document must clearly label:

- implemented now;
- incubating in the repository;
- architecturally committed but not implemented;
- exploratory only.

The README must not describe the future workflow runtime as already available.

## 17.6 Example workflows as documentation

A contributor will understand the vision faster from versioned examples than from abstract prose alone.

Document at least:

- direct local completion;
- research enrichment before a main model;
- corrective loop with `maximum_iterations = 3`;
- coding workflow committing through an editor/repository connector;
- reactive tracker maintaining a derived document;
- child workflow with a focused workspace;
- local/private target plus external provider target under data-egress policy.

---

# 18. Roadmap aligned to the clarified vision

## Stage 0 — ratify identity and terminology

Before Phase 12 expands code:

- replace the previous analyzer with this strategic owner;
- preserve and extend `docs/vision.md`;
- rewrite the README around operator-defined workflows and current honest status;
- add canonical terminology;
- mark `task-graph` and `corrective-workflow` as incubating workflow foundations rather than supported product or disposable experiments;
- update architecture and roadmap documents so future phases are reviewed against this identity.

This stage is documentation and decision work, not a large rewrite.

## Stage 1 — Phase 12: strengthen the local execution target

Proceed with the mixed-dtype Safetensors and load-transaction work already identified:

- observed per-tensor dtype inventory;
- exact conversion-aware planning;
- prepared load transaction;
- transactional partial-load cleanup;
- CPU and CUDA fixtures/evidence;
- removal of duplicated scalar assumptions.

This work remains valuable because the local execution target is one foundational executor in the future graph.

Do not mix workflow architecture into the Phase 12 loader changes.

## Stage 2 — define workflow, workspace, and authority schemas

Produce reviewed domain models and fixtures before building a large runtime:

- workflow definition and versioning;
- node/port identities;
- control versus data edges;
- workflow deployment;
- run identity and lineage;
- artifacts and versions;
- context workspace and links;
- authority bindings;
- capabilities and budgets;
- target requirements;
- plugin descriptors;
- explicit recurrence semantics.

Keep schemas portable where practical.

## Stage 3 — minimal general workflow vertical slice

Implement the smallest end-to-end runtime that can express current behavior without hardcoding it:

```text
input artifact
-> context assembly
-> local execution target
-> output artifact
-> explicit terminal/release
```

Add a headless host and a versioned direct-completion template. Prove that the same definition can be built through Rust and loaded from serialized data.

## Stage 4 — migrate corrective behavior into a template

Generalize the reusable semantics from `corrective-workflow` and express:

```text
produce -> validate -> conditionally revise -> bounded repeat -> commit
```

as public workflow configuration.

Acceptance requires changing the correction count, model target, validator, and sink without modifying scheduler code.

## Stage 5 — durable workspace and reactive execution

Add:

- persistent artifacts and run checkpoints;
- workspace search provider interface;
- child workspaces;
- cross-workspace grants;
- triggers/subscriptions;
- derived-artifact maintenance;
- external commit connectors with conflict reporting;
- resume after process restart.

## Stage 6 — plugin SDK and second execution-target category

Prove extensibility with one real non-Candle target and one external connector.

A provider or supported local coding-agent process connector is preferable to inventing a second tensor backend solely for abstraction proof.

Acceptance requires one workflow to switch or route between local and external execution based on explicit configuration and policy.

## Stage 7 — peer execution over operator-provided connectivity

Add:

- peer identity;
- capability advertisement;
- target enrollment;
- request authentication;
- remote execution events;
- disconnection and remote ownership semantics;
- data placement policy.

Do not implement a VPN. Test over ordinary operator-provided connectivity.

## Stage 8 — control center

Build the visual control center over stable schemas and runtime APIs:

- graph editing;
- typed ports;
- template/subgraph composition;
- target and peer overview;
- workspace and artifact lineage;
- authority and access-policy views;
- live run state;
- recursion and budget visibility;
- plugin discovery;
- validation before deployment.

The control center should be replaceable. Workflow files and APIs remain canonical.

## Stage 9 — advanced placement, memory, and orchestration

Only after the foundations are proven:

- distributed workflow-node placement;
- spatial/toroidal memory providers;
- richer ECS-backed scheduling experiments;
- multi-host workspace replication;
- advanced policy engines;
- cooperative long-lived agents.

These should plug into established interfaces rather than redefine them.

---

# 19. Architecture governance

Every proposed feature or phase should answer the following.

## 19.1 Core-or-extension test

1. Is this behavior required for every safe workflow?
2. Can it instead be expressed as a workflow template or node type?
3. Is it provider-, UI-, storage-, model-, or network-specific?
4. Does it require new ambient authority?
5. Can the operator replace or disable it?
6. Does it preserve workspace and authority semantics?
7. Does it introduce unbounded recursion, output, cost, storage, or effects?
8. Does it force remote and local targets into false equivalence?
9. Does it contaminate the local token hot path with plugin or workflow complexity?
10. Which canonical document and test prove the behavior?

A behavior specific to one desirable flow belongs in a template or plugin. A behavior needed to execute all flows safely belongs in the runtime.

## 19.2 Required architecture invariants

- no hidden main agent;
- no implicit global context;
- no implicit truth source;
- no implicit external write;
- no implicit provider or peer routing;
- no silent target fallback;
- no unbounded recursion by default;
- no plugin ambient authority;
- no provider/vendor types in portable contracts;
- no workflow semantics in frontend code;
- no dynamic plugin dispatch inside local token/tensor hot loops;
- no support claim without implementation and evidence.

## 19.3 Phase alignment record

Each major phase should include a short checked-in record:

```text
Which vision capability does this advance?
Which layer owns it?
Why is it core rather than a template/plugin/adapter/host feature?
Which operator choices remain configurable?
Which new capability or authority is introduced?
How is work bounded?
What is implemented, and what remains intentionally unsupported?
```

This is more valuable than expanding the architecture validator with every exact dependency edge.

---

# 20. Immediate decisions recommended before Phase 12

## Decision A — reject the generic vision draft as a replacement

Retain `docs/vision.md` and expand it with the operator's detailed workflow vision. Reuse only concise integration/runtime passages from the generic draft where helpful.

## Decision B — adopt the operator-programmable identity

Ratify workflows, workspaces, authority, plugins, and execution targets as central—not speculative late-stage extras.

## Decision C — retain the local inference runtime as an independent kernel

Do not bury or rewrite E0. It remains publishable value and the first local execution target.

## Decision D — incubate, do not discard, workflow prototypes

Keep `corrective-workflow` and `task-graph` accessible and tested, but label their current APIs non-final. Use them to derive the general workflow model.

## Decision E — prevent E1 from becoming the workflow core

Freeze its role as the current application/reference composition. Do not add general workflow, plugin, or workspace contracts directly to the chat-oriented facade.

## Decision F — commit to defaults as public templates

The future default workflow, direct completion, chat, and corrective behavior should be versioned template data using the public graph model.

## Decision G — keep an engine-centered monorepo

Retain thin hosts and examples, move executable applications to a clearer top-level `apps/` area when performing the ratified structure migration, and keep publication package-by-package.

## Decision H — define a strict `core` admission rule before renaming folders

A `crates/core/` root is reasonable only for portable, vendor-neutral definitions and algorithms. Do not perform path churn until the package taxonomy is accepted.

## Decision I — separate workflow node from physical endpoint terminology

Adopt `workflow node`, `execution endpoint`, and `peer host` consistently.

## Decision J — keep networking transport-agnostic

Milkdrift owns peer capabilities and authorization, not the VPN or overlay.

## Decision K — preserve explicit budgets as autonomy expands

Add workflow-level cost, recursion, concurrency, effect, storage, and egress policies before autonomous persistent workflows are enabled.

---

# 21. Technical cleanup retained from the earlier report

The clarified vision changes the disposition of workflow-related crates, but it does not invalidate the following cleanup findings:

1. **Documentation truth ownership still needs consolidation.** The new public concept spine makes this more urgent.
2. **Benchmark/evidence tooling should remain outside the default product gate.** It must not dominate workflow development.
3. **The obsolete generic decoded-output path should still be removed if it remains unused.** Token and text output types should have one clear owner at each layer.
4. **The architecture checker should enforce durable layer direction rather than mirror every incidental dependency edge.** The new plugin and workflow ecosystem would otherwise make policy maintenance excessive.
5. **`application-runtime` still needs internal cohesion cleanup.** Its model, chat/context, generation, and cross-cutting support areas should be clearer while its scope is frozen.
6. **Phase 12 still requires a model metadata and load-transaction reset.** Workflow ambitions do not excuse inaccurate local model planning.
7. **Live `llm-app` naming and data-path migration still need correction.** Public identity should consistently use Milkdrift.
8. **Large transaction-heavy tests and production functions should still be organized by invariant.** The workflow runtime will add more state-machine pressure, making this discipline important.
9. **CI should distinguish core, integration, evidence, experiment, and hardware gates.** Plugins and templates should not force every contributor to build every host or external connector.

One earlier finding is explicitly reversed:

> Do **not** move `corrective-workflow` and `task-graph` away as irrelevant experiments merely because they are disconnected from the current Slint product. Reclassify them as incubating workflow foundations and redesign their role under the new architecture.

---

# 22. Explicit non-recommendations

Do not:

- rewrite the entire repository before Phase 12;
- merge every crate into one `milkdrift-core` package;
- expose the current chat facade as the final workflow API;
- turn `corrective-workflow` directly into the universal scheduler by renaming it;
- hardcode research, correction, sanitization, execution, or main-agent roles;
- use arbitrary graph cycles without defined persistence and budget semantics;
- allow agents to mutate workflow definitions silently;
- make all workspace data globally visible to every node;
- equate an agent output with authority;
- let model nodes directly execute filesystem or network effects by default;
- give plugins ambient access to secrets, files, network, or runtime internals;
- require JSON for every in-process value;
- use Rust dynamic libraries as the first stable plugin ABI;
- represent provider APIs as local `LoadedModel` implementations;
- assume a ChatGPT, Claude, or coding-tool subscription grants a supported automation API;
- build a WireGuard/Tailscale/NetBird replacement inside Milkdrift;
- make the control center's graph format canonical instead of the engine schema;
- force a visual frontend before the graph and runtime model are stable;
- promise full `no_std` inference without a concrete host;
- let policy/documentation machinery outgrow implemented capability.

---

# 23. Success criteria

## 23.1 Identity test

A newcomer can answer, from the first README screen:

```text
Milkdrift lets operators build configurable AI systems around models.
The workflow, workspace, authority, and target runtime is the product.
The local inference runtime is the first execution kernel.
The UI is a replaceable host.
```

## 23.2 Configurability test

The research-and-correct workflow can be changed from three correction iterations to one, use another validator, remove sanitization, add a child workflow, or change the commit target without modifying runtime code.

## 23.3 Context test

A child agent can query an authorized parent or peer workspace through a bounded context view with provenance, while an unauthorized node cannot see it.

## 23.4 Authority test

A workflow can distinguish model proposal, specification evidence, test validation, and repository commit. The runtime records which artifact version was proposed, validated, and committed.

## 23.5 Target test

The same logical model node can bind to the local Candle target or a real external target through configuration, while capability and privacy differences remain visible.

## 23.6 Plugin test

A third-party Rust crate or process connector can register a node type and appear in validation, documentation, CLI introspection, and the control-center schema without editing the workflow scheduler.

## 23.7 Recursion test

A workflow can spawn a focused child workflow with its own workspace and return artifacts through an explicit binding. Depth, concurrency, cost, and effect permissions are enforced.

## 23.8 Persistence test

A reactive workflow can survive process restart, retain run and artifact lineage, resume valid work, and avoid silently repeating an external effect without reporting uncertainty.

## 23.9 Frontend test

A headless host and a visual host execute the same serialized workflow definition and observe the same run semantics.

## 23.10 Candle-value test

A project that only needs local inference can depend on the lower Milkdrift runtime without importing workflow UI, provider, redb, Hugging Face acquisition, or application-conversation concerns.

## 23.11 Portability test

The repository states and tests which workflow/artifact schemas build under `no_std`, `no_std + alloc`, and `std`, without implying that Candle or native plugins share those targets.

---

# 24. Proposed canonical project statement

The following is suitable as the basis of README positioning, but not as a replacement for the deeper authentic vision:

> Milkdrift is a Rust-native runtime for building operator-defined AI systems. It connects local models, remote providers, tools, context workspaces, validators, and external data sources through versioned workflows rather than a fixed agent pipeline. Operators decide how work is researched, routed, corrected, executed, and committed. Milkdrift supplies the lifecycle, permissions, provenance, resource bounds, execution targets, and extensibility needed to make those systems reliable and deeply embeddable.
>
> The current implementation provides the local inference kernel: exclusive model ownership, bounded scheduling and output, cancellation, cleanup, unload, and explicit CPU/CUDA execution through Candle. The broader workflow, workspace, plugin, provider, peer, and visual-control layers are the project's ratified direction and remain under staged development.

---

# 25. Final verdict

The operator's clarified workflow vision is not an optional feature to append to the previous architecture. It changes the project's product boundary.

Milkdrift should not be reduced to:

```text
Candle + a safer generation loop
```

Nor should it become:

```text
one fixed multi-agent pipeline + a graphical editor
```

Its strongest coherent future is:

```text
portable workflow/workspace/authority contracts
        +
extensible workflow runtime
        +
explicit context and artifact lineage
        +
local/provider/peer execution targets
        +
the existing owned inference kernel
        +
replaceable headless and visual hosts
```

The inference kernel remains a substantial independent product and should be tightened and published. The broader Milkdrift project, however, exists to let operators design the intelligence system around that kernel.

That is the distinction the README, vision, architecture, project structure, public API, and roadmap must preserve.
