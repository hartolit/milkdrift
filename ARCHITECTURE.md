# Architecture constitution

This document owns Milkdrift's durable product boundary, terminology, semantic ownership, dependency direction, and compatibility rules. Current implementation facts belong in `docs/STATUS.md`; ordered work belongs in `docs/ROADMAP.md`.

## 1. Product boundary and non-goals

Milkdrift is a local-first, durable, live-editable blueprint runtime for composing AI providers, tools, processes, humans, and peer machines as interchangeable, explicitly constrained capabilities. The semantic core defines workflows, capability needs, authority-relevant intent, and durable contracts without knowing how work is hosted.

The repository does not own tensor loading or execution, model architectures, tokenization, sampling, KV caches, inference memory, Safetensors/GGUF internals, or CUDA, ROCm, Vulkan, and similar kernels. It does not erase genuine provider differences, implement a VPN/overlay/NAT traversal service, hide mutable workflow definitions, use arbitrary graph cycles as execution, or couple semantic meaning to Iced, HTTP, a provider, a database, an operating system, or an async runtime.

## 2. Canonical terminology

- A **blueprint** is a reusable declarative workflow or subworkflow package.
- A **workflow** is a top-level blueprint identity with revision lineage.
- A **revision** is an immutable semantic definition snapshot.
- A **run** is a durable execution pinned to a revision.
- A **node** is a definition-time graph unit; a **node execution** is a runtime attempt and never mutable state on the node.
- An **edge** is an explicit control and/or typed data dependency.
- A **capability requirement** states what a task needs, not which provider wins selection.
- A **capability descriptor** is an immutable, honest advertisement; an **observation** is mutable health, availability, load, or lease state.
- **Layout** is presentation state, stored separately and excluded from semantic identity.
- An **author** is bounded revision/mutation provenance, never an authority grant.
- An **actor** is a human, service, controller, peer, or delegated principal authenticated for an action.
- An **artifact** is a bounded durable reference to content, not unbounded content embedded in events or invocation contracts.

## 3. Revisions and run history

Blueprint/workflow revisions are immutable snapshots with exact parent references, a user-facing lineage sequence, and a deterministic semantic content digest. The digest excludes timestamps, random map order, JSON object order, and layout. A revision is created only by applying one complete, versioned mutation batch to genesis or one exact optimistic base and validating the final private candidate. Multiple parents denote a deliberate resolved merge; the kernel never invents semantic conflict resolution.

Runs pin one revision and record append-only events. Events say what was accepted, scheduled, observed, produced, failed, cancelled, or left uncertain. A projection may be rebuilt from events, but neither a projection nor a later revision alters recorded history.

## 4. Commands, events, projections, effects, and ownership

A **command** asks an owning domain to change state and carries actor, authority, idempotency, and optimistic expectations. The blueprint crate owns the closed semantic mutation command model; the runtime owns closed versioned run-control commands. Commands are validated intent, not evidence that an action happened.

An **event** is an immutable accepted fact appended by the durable runtime. **Projections** are disposable, versioned read models derived from ordered events. **Effects** are explicit interactions with capabilities or infrastructure. The scheduler decides desired transitions; an outbox/effect executor performs effects; adapters report observations; the runtime alone decides workflow state. Effects and their acknowledgements must be idempotently correlated so crash recovery cannot quietly duplicate non-idempotent work.

The headless runtime owns transition decisions, projections, leases, scheduling, recovery, and reconciliation through narrow persistence/execution ports. A future daemon owns their lifecycle plus secrets mediation and real effect dispatch. Clients request commands and render projections; they do not own truth.

## 5. Prospective live reconciliation

Editing during a run produces a new immutable revision. Reconciliation compares the run's pinned/reconciled revision with an explicitly selected prospective revision. Completed, started, effect-dispatched, or otherwise historically committed node executions remain attached to their original definition and provenance. Only work that has not crossed its durable commitment boundary may be redirected. Ambiguity, incompatible interfaces, removed required work, changed authority, or uncertain effects produce a typed conflict/remediation state, never silent rewriting. The invariant is absolute: history never changes.

## 6. Capability contracts and resolution

Capability descriptors contain provider-neutral facts: identity and descriptor revision, stable category, namespaced operations, exact input/output schema contracts, streaming shapes, cancellation and idempotency behavior, side-effect class, admission limits, locality, trust zones, honest optional resource observations, and bounded extensions. Mutable health, availability, load, leases, credentials, and executor handles are separate.

Blueprint tasks carry capability requirements. A future registry resolves exact pins and constraint matches against current descriptors and policy, then records the chosen descriptor revision. Namespaced features advertise tools, structured output, reasoning controls, embeddings, images, seeds, token accounting, cancellation, or other features only when actually supported. There is no permanent enum of every provider operation and no fabricated common denominator.

Invocation requests, progress/output events, cancellation exchange, terminal outcome, usage, retryability, side-effect status, and uncertainty are versioned provider-neutral contracts. Values and artifacts use bounded references. Adapters translate and report; they never decide run state.

## 7. Actors, authority, proposals, and AI control

Actor identity and scoped authority are distinct from revision authorship. Authority grants constrain commands by workflow, capability, operation, budget, path, network destination, secret reference, side-effect class, and duration. Policy evaluation records the actor, grant/version, decision, and relevant constraints.

An actor may inspect or propose without authority to apply. Approval is an explicit command/event linking proposal, exact revision or effect, approver, and policy. An AI controller is an ordinary task using a workflow-control capability under a scoped grant. It uses the same closed mutations, optimistic checks, proposals, approvals, and audit trail as a human; it has no hidden privileged node kind or mutable backdoor.

## 8. Execution semantics

Sequential execution is ordinary acyclic control/data edges. A typed **branch** selects one declared arm using a safe condition AST. A **fork** creates named isolated child branches under structured concurrency. A **join** is owned by one fork and waits for all, any successful, or a satisfiable quorum; cancellation/failure policy is explicit. A **reducer/compositor** is a separate node with a declared input shape and does not masquerade as synchronization.

A **repeat** explicitly invokes a pinned acyclic body, evaluates a safe condition, and has a hard maximum iteration count plus optional tighter time/cost budgets and a terminal limit policy. A **wait** is a durable timer definition. A **signal wait** resumes only from an authenticated, correlated external signal. A **subworkflow** pins an immutable revision and interface; reuse is reference/instantiation, not an untracked copy. Explicit success, failure, and cancellation terminals make workflow results inspectable. Arbitrary cycles are rejected; ongoing control uses repeat or future prospective revisions.

Structured concurrency means a parent scope owns its children, branch-local resources, join, cancellation, and cleanup. A run cannot report a scope complete while owned work remains unaccounted for.

## 9. Workspaces, artifacts, context, provenance, and retention

Each concurrent branch receives an isolated logical workspace. Cross-branch values move only through declared data edges, artifacts, joins, reducers, or explicit merge operations. An artifact has identity, digest, media contract, size, producer, causal inputs, and retention class; large bytes stay outside semantic documents and ordinary events.

Context is selected from graph causality, declared inputs, chosen artifacts, scoped memory, and explicit byte/token/item budgets. Chronological whole-history dumping is not a fallback. Provenance connects every node execution to revision/node, actor, resolved capability/descriptor, invocation, inputs and selectors, outputs, artifacts, approvals, signals, effects, errors, and parent causal events. Retention policy may expire payloads while preserving safe metadata and integrity evidence; deletion is explicit and auditable.

## 10. Durable persistence and crash recovery

The persistence boundary appends checksummed, versioned events transactionally with command idempotency, workspace/accounting changes, and recovery indexes. Durable timers, signals, leases, cancellation requests, eligible work, and uncertain invocations survive process restarts. `RuntimeService::open_closed` performs only physical schema compatibility checks and returns with admission closed. `RuntimeService::new` then runs synchronous startup: it discovers nonterminal runs through bounded pages, projects and validates each active aggregate against its authoritative revision, journal head, workspace scopes/values/accounting, and frozen invocation references, classifies recoverable lease uncertainty, and opens admission only after active recovery reaches a progress-checked fixed point. Startup is bounded by active state; it does not traverse terminal history or rehash every artifact byte. A crash between external effect and acknowledgement becomes an explicit uncertain/reconciliation case governed by idempotency and side-effect facts, never guessed success.

Complete historical verification is a separate caller-owned administrative operation through resumable `StorageAdmin::scan_integrity` pages. Artifact-content verification is opt-in and bound into the continuation cursor. `health()` may report a small metadata/index sample, but a clean sample is not proof that all history or artifact content is healthy. Corruption reached while recovering a nonterminal run fails startup closed before admission; corruption isolated to unrelated terminal history is reported when that object is read or an explicit scrub reaches it.

Storage engines are adapters. No database type enters blueprint or capability semantics. Compatibility fixtures cover old on-disk data, recovery tests inject truncation/reordering/duplicate delivery/failed fsync boundaries where the selected backend permits, and migrations are restartable.

## 11. Peer execution

A peer exposes remote capability advertisements and invocation/cancellation/event contracts over a versioned protocol. Connectivity is a pluggable, user-provided transport; core does not own peer discovery, VPNs, overlay routing, or NAT traversal. Mutual identity, trust-zone mapping, authorization, replay protection, bounded messages, artifact transfer integrity, disconnect/uncertainty behavior, and capability descriptor provenance remain explicit. A peer is remote capability access, not a second semantic truth owner.

## 12. Daemon and clients

One daemon process will own authoritative durable state, scheduling, registries, effects, reconciliation, secrets mediation, and peer sessions. The CLI and Iced desktop app are thin clients over versioned commands and projections. They may optimistically render a pending command but cannot create hidden state or directly mutate journals. UI layout is stored separately from semantic revisions and can change without changing a content digest.

## 13. Dependency direction and forbidden coupling

The implemented production dependency direction is shown below. Arrows point
from a stable contract to a crate that consumes it:

```text
milkdrift-capability  -> {blueprint, workspace, persistence, runtime}
milkdrift-blueprint   -> {persistence, runtime, redb-store}
milkdrift-workspace   -> {persistence, runtime, redb-store}
milkdrift-persistence -> {runtime, redb-store}
```

The redb adapter also consumes blueprint documents directly for immutable
revision storage. Capability contracts know nothing about blueprints.
Blueprint uses only pure capability requirements and schema identities.
Persistence documents depend on immutable semantic/workspace contracts but own
no runtime decisions. Runtime and the redb/filesystem adapter are sibling
consumers of persistence and the immutable domain contracts; runtime uses the
adapter only as a development dependency in adapter-backed integration tests.
Future registries implement runtime-facing ports, and apps depend on daemon
protocols. Dependencies may point toward stable semantics, never from semantics
toward a host.

Forbidden in the semantic crates are Tokio or another executor, HTTP clients/servers, databases, Iced, provider SDKs, subprocess/OS APIs, transport types, secret values, tensor/inference types, live handles, clocks, randomness that affects identity, and mutable singleton registries. Project-authored code is safe Rust unless an independently proven requirement has a focused safety contract and tests.

## 14. Security, secrets, and untrusted input

Every disk, wire, provider, tool, peer, imported blueprint, artifact, path, signal, and AI-produced proposal is untrusted input. Readers enforce schema version, byte/count/depth/string/path bounds before expensive work; reject unknown core semantics; and preserve only bounded namespaced extensions. Conditions are data ASTs, not scripts.

Credentials and secret values never appear in blueprints, descriptors, requirements, events, diagnostics, logs, or peer advertisements. Later adapters receive opaque secret/profile references through a policy-mediated host and minimize exposure duration. Filesystem/network/process effects require normalized allowlists and resist traversal, symlink, redirect, shell-injection, and confused-deputy attacks. Side effects, authority decisions, approvals, hostile provider output, and uncertain outcomes are provenance facts. Budget and termination controls are enforced by the owning runtime, not trusted to an AI prompt.

## 15. Disk/wire schemas and compatibility

Portable capability and blueprint documents use canonical schema-v1 JSON envelopes with an explicit numeric `schema_version`. Digest inputs use recursively key-sorted deterministic JSON and deterministic collections. Unknown core variants, malformed typed identities, invalid derived fields, and unsupported future versions fail clearly. Explicit bounded DNS-namespaced extension maps are the only forward-compatible unknown field mechanism.

Readers support only versions they can interpret without guessing. A writer emits one current canonical version. Adding optional meaning still requires a schema review; changing existing meaning or canonical bytes requires a new version and fixtures. Old golden fixtures remain read tests for every supported version. Disk events, projections, daemon commands, peer messages, and artifacts will each declare independent version ownership rather than sharing one global version.

## 16. Logical ownership and crate extraction

The following map is the long-lived ownership reference:

```text
milkdrift/
├── crates/
│   ├── blueprint/
│   │   ├── model
│   │   ├── validation
│   │   ├── revision
│   │   └── mutation
│   ├── runtime/
│   │   ├── scheduler
│   │   ├── execution
│   │   ├── structured-concurrency
│   │   ├── reconciliation
│   │   └── recovery
│   ├── capability/
│   │   ├── contracts
│   │   ├── registry
│   │   └── resolution
│   ├── workspace/
│   │   ├── context
│   │   ├── artifacts
│   │   └── branches
│   ├── persistence/
│   │   ├── events
│   │   ├── journal
│   │   └── projections
│   └── peer/
│       ├── protocol
│       └── capability-advertisement
├── adapters/
│   ├── model/
│   ├── process/
│   ├── filesystem/
│   └── peer-transport/
├── apps/
│   ├── desktop-iced/
│   └── daemon/
└── docs/
```

The logical map is the long-lived ownership reference. Its exact current physical mapping is:

| Logical responsibility | Current physical crate/module |
| --- | --- |
| `blueprint/model` | `milkdrift-blueprint::model` (public types re-exported at crate root) |
| `blueprint/validation` | `milkdrift-blueprint::validation` |
| `blueprint/revision` | `milkdrift-blueprint::revision` |
| `blueprint/mutation` | `milkdrift-blueprint::mutation` |
| `runtime/scheduler` | `milkdrift-runtime::scheduler` and runtime controller admission/index decisions |
| `runtime/execution` | `milkdrift-runtime::{command,executor}` plus the `engine::{command_planning,completion,dispatch,scheduling,state,support,workspace}` and `projection::{apply_core,helpers,node,run}` modules |
| `runtime/structured-concurrency` | Blueprint definitions, `milkdrift-runtime::engine::structured`, `milkdrift-runtime::projection::{apply_structured,structured}`, and `milkdrift-workspace::scope` |
| `runtime/reconciliation` | `milkdrift-runtime::reconciliation`, `engine::reconciliation`, `projection::{apply_reconciliation,reconciliation}`, and persistence-owned plan event documents |
| `runtime/recovery` | `milkdrift-runtime::{query,engine::recovery,projection::replay}` and recovery indexes in the redb adapter |
| `capability/contracts` | `milkdrift-capability::{descriptor,invocation,document,identity,bounded}` |
| `capability/registry` | Deterministic test boundary only; live Pass 3 registry remains unimplemented |
| `capability/resolution` | Pure requirement matching and exact immutable snapshots in `milkdrift-capability`; live policy selection remains Pass 3 |
| `workspace/context` | Scoped immutable values/budgets in `milkdrift-workspace`; causal context construction remains Pass 3 |
| `workspace/artifacts` | Metadata/contracts in `milkdrift-workspace` and durable bytes in `milkdrift-redb-store` |
| `workspace/branches` | `milkdrift-workspace::scope` plus runtime branch/iteration/subworkflow projections |
| `persistence/events` | `milkdrift-persistence::{event,document}` with schema-v1 golden fixtures |
| `persistence/journal` | Narrow `milkdrift-persistence` ports implemented by `milkdrift-redb-store::journal::{append,discovery,queries,workspace}` |
| `persistence/projections` | Pure `milkdrift-runtime::projection`; optional checked snapshots are persistence documents |
| `peer/protocol` | Not implemented |
| `peer/capability-advertisement` | Generic descriptor contract exists; peer protocol/advertisement is not implemented |
| `adapters/model` | Not implemented |
| `adapters/process` | Not implemented |
| `adapters/filesystem` | Content-addressed artifact ownership in `milkdrift-redb-store::artifact::{accounting,cleanup,path,publication}` |
| `adapters/redb` | The transactional local adapter, split across `milkdrift-redb-store::{admin,journal,store}` facades and their private child modules |
| `adapters/peer-transport` | Not implemented |
| `apps/desktop-iced` | Not implemented |
| `apps/daemon` | Not implemented |

Physical crates are extracted only for a real dependency, lifecycle, host, publication, or multiple-consumer boundary. No empty crate or placeholder directory may be created merely to resemble the diagram. A later pass may merge or split physical packages when it preserves logical ownership and reduces coupling. Within a cohesive crate, private modules are preferred until extraction creates a measurable boundary; conversely, a growing module must split when unrelated invariants, dependencies, lifecycle, or test ownership become entangled.

## 17. Testing philosophy

Tests establish independent invariants rather than restating algorithms. Small hand-reviewed examples cover each semantic node and capability variance. Golden JSON fixtures own compatibility and exact canonical re-encoding. Property/model tests generate mutation batches and require every published revision to validate. Compile-fail examples prove private API boundaries.

The runtime and persistence layers must add deterministic state-machine tests, crash/restart and fault injection, duplicate/out-of-order delivery, uncertain-effect recovery, reconciliation histories, compatibility fixtures, and cross-adapter contract suites. Security-critical policy, graph validation, reconciliation, idempotency, and recovery logic should receive mutation testing once implemented; surviving mutants are missing assertions or deliberately justified equivalents. Coverage volume is not a substitute for testing the invariant from an independent observation.
