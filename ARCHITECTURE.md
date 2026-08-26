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

Runtime history has four deliberately different owners:

```text
journal = complete immutable history
active projection = bounded operational state
snapshot = optional bounded recovery checkpoint
historical read model = paged reconstruction/query
```

The active projection compacts deterministically while folding each durable event. Full execution occurrences remain only while eligible, dispatched, retrying, uncertain, cancelling, awaiting successor scanning, owning active structured work, or carrying an unconsumed import/reconciliation obligation. Once those obligations close, the current scope/node frontier keeps at most one compact terminal summary with its immutable revision, creation boundary, attempt count/latest identity, conservative side-effect class, selected route, and retained output/provenance references. Completed branch/join ownership and scopes, repeat iterations, and child-workflow links retire when their exact consumers close. Settled progress reports, old attempts and leases, fired timers, consumed signals, superseded occurrences/repeat frontiers/revision pins, recovery passes, and reconciliation plans are journal-only history. Active query methods return current operational occurrences and summaries, never an audit timeline; exact historical detail is read from stable-cursor journal pages. The runtime-owned projection payload schema changes when those payload semantics change; its persistence envelope has an independent schema. Unsupported or invalid optional snapshots are discarded in favor of authoritative replay.

Compaction means removal from active operational state only. This pass never deletes run events, artifact content, artifact metadata, or output records. Retained summaries keep the stable execution/attempt/revision identities, sequence boundary, history digest, and live artifact/output references required to locate older evidence. A future Iced timeline/inspector and causal-context builder can reconstruct exact historical context on demand by paging the journal and resolving those durable references.

"Bounded" is relative to semantic liveness, not a universal constant-memory claim. Projection and checkpoint size may grow with workflow shape, intentionally active branches, unresolved safety obligations, retained outputs/artifacts/context, bounded worker ownership evidence, and configured workspace limits. For fixed workflow shape and bounded active concurrency, settled event count and elapsed iterations do not by themselves increase active state.

The headless runtime owns transition decisions, projections, leases, scheduling, recovery, and reconciliation through narrow persistence/execution ports. `milkdrift-capability-host::EffectWorkerHost` is the explicit embeddable owner of bounded caller-created effect and cancellation workers; it has no singleton or async runtime. The daemon owns that object together with registry, secret, redb, artifact, recovery, and shutdown lifecycles. Clients request commands and render projections; they do not own truth.

## 5. Prospective live reconciliation

Editing during a run produces a new immutable revision. Reconciliation compares the run's pinned/reconciled revision with an explicitly selected prospective revision. Completed, started, effect-dispatched, or otherwise historically committed node executions remain attached to their original definition and provenance. Only work that has not crossed its durable commitment boundary may be redirected. Ambiguity, incompatible interfaces, removed required work, changed authority, or uncertain effects produce a typed conflict/remediation state, never silent rewriting. The invariant is absolute: history never changes.

## 6. Capability contracts and resolution

Capability descriptors contain provider-neutral facts: identity and descriptor revision, stable category, namespaced operations, exact input/output schema contracts, streaming shapes, cancellation and idempotency behavior, side-effect class, admission limits, locality, trust zones, honest optional resource observations, and bounded extensions. Mutable health, availability, load, leases, credentials, and executor handles are separate.

Blueprint tasks carry capability requirements. `milkdrift-capability-host` resolves exact pins and constraint matches against one consistent live registry snapshot, fresh availability evidence, immediate capacity evidence, and configured authority/policy facts, then the runtime records the chosen descriptor revision. Selection is stable by exact requirement, explicit priority, capability identity, and revision; after the snapshot is persisted, execution and cancellation route only to that exact generation and never fall back. Namespaced features advertise tools, structured output, reasoning controls, embeddings, images, seeds, token accounting, cancellation, or other features only when actually supported. There is no permanent enum of every provider operation and no fabricated common denominator.

Invocation requests, progress/output events, cancellation exchange, terminal outcome, usage, retryability, side-effect status, and uncertainty are versioned provider-neutral contracts. Values and artifacts use bounded references. Adapters translate and report; they never decide run state.

### Peer-as-capability boundary

`milkdrift-peer-protocol` owns only v1.0 bounded transport-neutral session, catalog, invocation, observation, cancellation, delegation, and artifact-transfer messages. `milkdrift-peer-http` owns configured HTTPS/loopback transport, relationship authentication, durable accepted-execution records, resumable observations, verified artifact staging, and the ordinary remote `CapabilityAdapter`. Runtime, control, and persistence cores do not depend on either package or HTTP/TLS crates.

The serving daemon derives an expiring authority-filtered catalog from its live capability host. The consuming daemon remaps each exact remote identity/generation into a collision-resistant local identity with `Locality::Remote`, trust zone, and exact peer/catalog provenance, then registers it through `milkdrift-capability-host`. The catalog is live observation only. A run still records one exact local `ResolvedCapabilitySnapshot`; neither peer reads the other's database or shares mutable workflow state.

Remote acceptance is durably recorded before it is reported. Reusing an idempotency key with identical canonical facts returns the same remote execution; different facts conflict. Observation sequence/cursors survive response loss. Connection closure proves neither cancellation nor terminal outcome. After accepted adapter-entry intent, missing restart evidence becomes explicit uncertainty instead of replacement execution. External side effects are never advertised as globally exactly once.

Connectivity is operator supplied through a reachable HTTPS path or an explicitly enabled loopback development route. Milkdrift does not discover peers, traverse NAT, create certificates, run a hosted coordinator, provide an overlay/VPN, synchronize models/tensors, share databases, or implement consensus.

## 7. Actors, authority, proposals, and AI control

Actor identity and scoped authority are distinct from revision authorship. Canonical `ActorRef` ownership is in `milkdrift-authority`; blueprint `AuthorRef` remains provenance only. Immutable schema-v1 grant revisions constrain commands by workflow/run, typed operation, capability identity/category/operation/profile/trust/locality, normalized filesystem roots and access, credential-free network profiles/destinations, opaque secret references, side-effect class, cost, duration, invocation, artifact, concurrency, validity, and revocation generation. Pure policy evaluation records the actor, exact grant revision, evaluator policy/version, request facts, stable reason codes, evaluated constraints, caller-supplied boundary time, result, and deterministic digest.

Every external run command presents an exact grant revision to the runtime's injected evaluator before semantic acceptance. The exact decision is part of command-result schema v2 and is committed in the same transaction as acceptance events or denial-without-events; exact redelivery returns that original decision/result without reevaluation. System transitions and worker observations use separate private runtime-owned receipt paths. The local daemon authenticates referenced bearer secrets at request time and supplies configured immutable actor/grant context; actor identity is absent from command JSON. Peer transport authentication separately establishes a stable configured `PeerId`. A hostname, display name, payload claim, descriptor, or valid credential never grants capability authority by itself.

An actor may inspect or propose without authority to apply. Approval is an explicit command/event linking proposal, exact revision or effect, approver, and policy. An AI controller is an ordinary task using a workflow-control capability under a scoped grant. It uses the same closed mutations, optimistic checks, proposals, approvals, and audit trail as a human; it has no hidden privileged node kind or mutable backdoor.

A workflow proposal is bounded, versioned, duplicate-key-safe, canonical, and digest-bound untrusted data. It names the exact workflow, base revision and digest, optional live run and observed sequence, one closed mutation batch, provenance, evidence/artifact references, rationale, risk notes, requested application policy, optional run action, and a claimed stop condition. Large reasoning remains in referenced artifacts rather than ordinary command documents. Model prose and tool calls are never control intent: only a successfully decoded structured-output value can become a proposal, and malformed or adversarial output terminates the producing task without direct workflow effect.

`milkdrift-control` is an application layer over existing owners. It privately builds and validates the complete prospective revision, derives an exact authority delta, classifies deterministic risk, stores the immutable candidate, and delegates live proposal, approval, apply, pause/resume/cancel/retry, and signal actions to `RuntimeService`. It never appends events or rewrites run history directly. Low-risk auto-apply is permitted only for future pure/read-only work under an explicit policy and an exact apply grant; terminal changes, existing/started work, side effects, provider/profile/trust expansion, subworkflow/interface changes, cancellation, and other elevated cases require the existing recorded approval path. Stale sequence, base, digest, plan, or proposal guards fail closed.

Observer, Advisor, Supervisor, Controller, and Autonomous are convenience names that expand into ordinary immutable grants with caller-supplied exact workflow/run, capability, budget, validity, and revocation scope. They are not runtime roles. Continuous control is an ordinary acyclic wrapper around an explicit pinned `Repeat` body with hard invocation/revision/mutation/node/time/cost/input/output/artifact/process/model/failure/rejection/repetition/child-depth ceilings and a human checkpoint; no arbitrary graph cycle or unbounded background loop is introduced.

## 8. Execution semantics

Sequential execution is ordinary acyclic control/data edges. A typed **branch** selects one declared arm using a safe condition AST. A **fork** creates named isolated child branches under structured concurrency. A **join** is owned by one fork and waits for all, any successful, or a satisfiable quorum; cancellation/failure policy is explicit. A **reducer/compositor** is a separate node with a declared input shape and does not masquerade as synchronization.

A **repeat** explicitly invokes a pinned acyclic body, evaluates a safe condition, and has a hard maximum iteration count plus optional tighter time/cost budgets and a terminal limit policy. A **wait** is a durable timer definition. A **signal wait** resumes only from an authenticated, correlated external signal. A **subworkflow** pins an immutable revision and interface; reuse is reference/instantiation, not an untracked copy. Explicit success, failure, and cancellation terminals make workflow results inspectable. Arbitrary cycles are rejected; ongoing control uses repeat or future prospective revisions.

Structured concurrency means a parent scope owns its children, branch-local resources, join, cancellation, and cleanup. A run cannot report a scope complete while owned work remains unaccounted for.

## 9. Workspaces, artifacts, context, provenance, and retention

Each concurrent branch receives an isolated logical workspace. Cross-branch values move only through declared data edges, artifacts, joins, reducers, or explicit merge operations. An artifact has identity, digest, media contract, size, producer, causal inputs, and retention class; large bytes stay outside semantic documents and ordinary events.

Each task revision owns a private-invariant context policy. The runtime's pure causal builder considers declared inputs, explicit control/data ancestry, exact node/semantic-role selectors, authority-filtered workspace and artifact metadata, and paged journal evidence. Sibling scopes remain invisible until an edge, join, or reducer exposes them. Stable order is causal depth, semantic kind, source node, then canonical source-reference bytes; item, reference-byte, artifact-byte, and optional provider-neutral unit estimates are admitted incrementally before content is read. Optional losses receive stable omission codes and required losses fail before dispatch. There is no chronological whole-history fallback.

The result is canonical context-manifest schema v1, bound to one run/revision/node execution/attempt and to the exact policy version/digest. It records ordered sources, causal evidence, sensitivity/authority facts, reasons, omissions, totals, budget, and a domain-separated digest. The exact restricted manifest artifact is committed before external entry and its immutable reference is carried once by invocation-request schema v2; retries of the same frozen request reuse it. For a Fresh model request the adapter verifies that artifact against the exact attempt, then inserts those same canonical bytes as the first system context block before encoding either provider protocol. References remain references: independently selected model-task content parts control any artifact byte transfer. Provenance connects every node execution to revision/node, actor, resolved capability/descriptor, invocation, inputs and selectors, outputs, artifacts, approvals, signals, effects, errors, and parent causal events. Retention policy may expire payloads while preserving safe metadata and integrity evidence; deletion is explicit and auditable.

`milkdrift-model` owns canonical provider-neutral model-task/response schema v1. Model identity and endpoint selection belong to exact capability profiles, not blueprints or the model contract. `milkdrift-model-provider` is an outer adapter with separate OpenAI-compatible and native Anthropic mappings over one bounded rustls/HTTP stack. It rejects unadvertised roles, parts, tools, schemas, reasoning, streaming, sessions, and encoded request bodies over the profile bound before provider entry. Model outputs and tool calls are data artifacts; no returned call is executed automatically.

## 10. Durable persistence and crash recovery

The persistence boundary appends checksummed, versioned events transactionally with command idempotency, workspace/accounting changes, and recovery indexes. Durable timers, signals, leases, cancellation requests, eligible work, and uncertain invocations survive process restarts. `RuntimeService::open_closed` performs only physical schema compatibility checks and returns with admission closed. `RuntimeService::new` then runs synchronous startup: it discovers nonterminal runs through bounded pages, projects and validates each active aggregate against its authoritative revision, journal head, workspace scopes/values/accounting, and frozen invocation references, classifies recoverable lease uncertainty, and opens admission only after active recovery reaches a progress-checked fixed point. Startup is bounded by active state; it does not traverse terminal history or rehash every artifact byte. A crash between external effect and acknowledgement becomes an explicit uncertain/reconciliation case governed by idempotency and side-effect facts, never guessed success.

Complete historical verification is a separate caller-owned administrative operation through resumable `StorageAdmin::scan_integrity` pages. Its redb implementation delegates the persisted physical phase order to run, scheduler, workspace, revision, snapshot, and artifact integrity modules; cursor schema v1 phase tags `0..=35`, ordering, anchor, and option binding remain unchanged. Artifact-content verification is opt-in and bound into the continuation cursor. `health()` may report a small metadata/index sample, but a clean sample is not proof that all history or artifact content is healthy. Corruption reached while recovering a nonterminal run fails startup closed before admission; corruption isolated to unrelated terminal history is reported when that object is read or an explicit scrub reaches it.

Storage engines are adapters. No database type enters blueprint or capability semantics. The current pre-release store accepts only its exact physical and internal document formats; older and future formats are refused, and no migration is claimed. Recovery tests inject truncation/reordering/duplicate delivery/failed durable boundaries where the selected backend permits. Any future migration must add hand-reviewed old-format fixtures and a restartable protocol before compatibility is claimed.

The optional projection-snapshot persistence envelope is currently schema v2 and contains a runtime-owned projection payload at schema v3. The envelope uses canonical JSON with one strict, padded RFC 4648 standard-Base64 payload string; its domain-separated, length-framed BLAKE3 checksum binds the semantic metadata and decoded raw payload bytes rather than the Base64 text. At a selected checkpoint boundary, the accepted event transaction also records a domain-separated commitment to those exact projection payload bytes in history-chain record schema v2. Storage returns a snapshot as verified only when the envelope checksum, event-prefix digest, and append-time payload commitment all agree; structural JSON limits are checked lexically before the payload value tree is allocated. Envelope v1 and projection payload v1, v2, or other unsupported checkpoints are not migrated; they are discarded and reconstructed from the journal. A snapshot covers one exact event sequence and cumulative history-chain digest and contains no lifetime event-ID or execution-ID collection.

## 11. Peer execution

A peer exposes remote capability advertisements and invocation/cancellation/event contracts over a versioned protocol. Connectivity is a pluggable, user-provided transport; core does not own peer discovery, VPNs, overlay routing, or NAT traversal. Mutual identity, trust-zone mapping, authorization, replay protection, bounded messages, artifact transfer integrity, disconnect/uncertainty behavior, and capability descriptor provenance remain explicit. A peer is remote capability access, not a second semantic truth owner.

## 12. Daemon and clients

One daemon process owns authoritative local durable state, scheduling, registries, effects, reconciliation, and secret mediation. Startup validates configuration and credential references before opening the service, opens redb once, initializes the runtime with admission closed, performs targeted recovery, registers configured adapters, starts bounded effect workers, and exposes readiness only afterward. Shutdown closes mutation admission first, reports draining, applies the configured drain/cancel/retain policy, flushes the control sidecar, joins the runtime owner, and drops storage.

Axum owns sockets and SSE framing only. Every runtime, redb, artifact, control, and registry operation crosses a bounded synchronous queue into one dedicated owner thread; a full queue returns a stable overload error. External adapter work remains on the fixed `EffectWorkerHost` threads, so neither an HTTP task nor the runtime owner holds a global lock while awaiting a process/model stream. Periodic maintenance has a configured bound and a blocking notification wait rather than a busy loop.

`milkdrift-control-protocol` owns pure external protocol 1.0 DTOs. Its read models project immutable revisions, compact runs/nodes/attempts, proposal state, timelines, capabilities, authority, artifacts, and health without serializing internal event variants or redb keys. Feed-bound opaque cursors provide bounded pages and monotonic SSE resume. `milkdrift-control-client` is the only HTTP mapping used by the CLI and future Iced client. The CLI cannot create hidden state or directly mutate journals.

Layout document schema 1 is presentation-only state with exact workflow/revision association, positions, annotations, viewport, author, update generation, and its own digest. The daemon persists it independently under optimistic generation checks. Layout never contains semantic edges, task configuration, prompts, secrets, or capability requirements and never changes a blueprint digest.

## 13. Dependency direction and forbidden coupling

The implemented production dependency direction is shown below. Arrows point
from a stable contract to a crate that consumes it:

```text
milkdrift-contracts   -> {capability, blueprint, workspace, authority, persistence, runtime, control}
milkdrift-capability  -> {blueprint, workspace, authority, persistence, runtime, capability-host, control}
milkdrift-blueprint   -> {authority, persistence, model, runtime, redb-store, control}
milkdrift-workspace   -> {authority, persistence, runtime, redb-store, control}
milkdrift-authority   -> {persistence, runtime, capability-host, control}
milkdrift-persistence -> {runtime, redb-store, control}
milkdrift-model       -> {runtime, model-provider, control}
milkdrift-runtime     -> {capability-host, control}
milkdrift-capability-host -> {local-process, model-provider, control}
milkdrift-authority   -> {secret-env, local-process, model-provider}
milkdrift-control-protocol -> {control-client, daemon}
milkdrift-control-client   -> {cli}
{authority, blueprint, capability-host, control, persistence, runtime, redb-store,
 local-process, model-provider, control-protocol} -> daemon
```

`milkdrift-contracts` owns only cross-domain implementation mechanics with
multiple production consumers: bounded JSON traversal, duplicate-key rejection,
canonical recursive JSON ordering, and the common validated-string newtype
implementation. Domain crates continue to own their identities, validation
rules, byte limits, schema versions, error vocabulary, and durable meaning.
The redb adapter also consumes blueprint documents directly for immutable
revision storage. Capability contracts know nothing about blueprints.
Blueprint uses only pure capability requirements and schema identities.
Persistence documents depend on immutable semantic/workspace contracts but own
no runtime decisions. Runtime and the redb/filesystem adapter are sibling
consumers of persistence and the immutable domain contracts; runtime uses the
adapter only as a development dependency in adapter-backed integration tests.
`milkdrift-capability-host` is an outer embeddable host implementing the runtime
`TaskExecutor` port. It owns the narrow materialization/publication port and its
`RuntimeStore` bridge, but knows no redb table or filesystem artifact layout.
`milkdrift-control` is an outer application crate consuming semantic, authority,
persistence, runtime, model, and host contracts. Its workflow-control adapter calls
the same `ControlService` reached by authenticated human/service clients and reports only normal
capability observations; it owns neither durable truth nor a host lifecycle.
`milkdrift-local-process` depends outward on that port and owns process/filesystem APIs;
it never depends on redb or mutates runtime state. `milkdrift-secret-env` is a separate
concrete secret boundary. `milkdrift-control-protocol` is a pure outward DTO boundary;
the HTTP stack exists only in `milkdrift-control-client` and `milkdrift-daemon`. Dependencies
may point toward stable semantics, never from semantics toward a host.

Forbidden in the semantic crates are Tokio or another executor, HTTP clients/servers, databases, Iced, provider SDKs, subprocess/OS APIs, transport types, secret values, tensor/inference types, live handles, clocks, randomness that affects identity, and mutable singleton registries. Project-authored code is safe Rust unless an independently proven requirement has a focused safety contract and tests.

## 14. Security, secrets, and untrusted input

Every disk, wire, provider, tool, peer, imported blueprint, artifact, path, signal, and AI-produced proposal is untrusted input. Readers enforce schema version, byte/count/depth/string/path bounds before expensive work; reject unknown core semantics; and preserve only bounded namespaced extensions. Conditions are data ASTs, not scripts.

Credentials and secret values never appear in blueprints, descriptors, requirements, events, command bodies, diagnostics, logs, or peer advertisements. `SecretRef` serializes only an opaque reference, while resolved `SensitiveSecret` values are non-serializable, non-clone, redacted, and exposed only through a narrow closure. The local daemon accepts only loopback plaintext, requires an enabled bearer-reference binding, compares credential digests in constant time, rereads file/environment references for rotation, and maps a match to server-owned actor/grant facts. Authentication and authority remain separate; permissive CORS is absent. The host owns the resolver port; `milkdrift-secret-env` resolves only explicitly mapped references and never enumerates the environment. Local-process profiles are argv templates, never shell command strings; each substitution remains one OS argument. The child begins from `env_clear`, receives only allowlisted ambient names and resolved secret refs, and secret-bearing profiles cannot stream process text. Filesystem/process effects require canonical allowlisted roots, isolated materialization, bounded regular files, traversal/symlink/hardlink rejection, and declared output imports. Side effects, authority decisions, hostile output, cancellation observations, and uncertain outcomes are provenance facts. Budget and termination controls are enforced by their owning boundary, not trusted to an AI prompt.

## 15. Disk/wire schemas and compatibility

Portable documents use explicit numeric schema versions: blueprint revision/mutation and invocation request are currently v2; the external control protocol is 1.0; daemon configuration, external cursors, CLI JSON output, layout documents, and the daemon control sidecar are schema 1; other capability documents, context manifests, model contracts, and endpoint profiles are v1. Digest inputs use recursively key-sorted deterministic JSON and deterministic collections. Unknown core variants, malformed typed identities, invalid derived fields, and unsupported future versions fail clearly. Explicit bounded DNS-namespaced extension maps are the only forward-compatible unknown field mechanism.

Readers support only versions they can interpret without guessing. A writer emits one current canonical version. Adding optional meaning still requires a schema review; changing existing meaning or canonical bytes requires a new version and fixtures. Old golden fixtures remain read tests for every supported version. Disk events, projections, daemon commands, layouts, peer messages, and artifacts each declare independent version ownership rather than sharing one global version.

Run-event envelopes are durable internal execution truth, not a promise that the
daemon exposes the storage schema directly. External historical read
models are separately versioned, paged, authorization-aware projections over
that truth. They may redact or reshape fields without changing, replacing, or
claiming ownership of the append-only event contract.

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
│   ├── contracts/
│   │   ├── canonical-json
│   │   └── validated-newtype-mechanics
│   ├── authority/
│   │   ├── actors-and-grants
│   │   ├── policy-evaluation
│   │   └── secret-references
│   ├── control/
│   │   ├── proposal-and-command-contracts
│   │   ├── risk-and-authority-policy
│   │   ├── shared-application-service
│   │   ├── workflow-control-adapter
│   │   └── bounded-controller-pattern
│   ├── runtime/
│   │   ├── scheduler
│   │   ├── execution
│   │   ├── structured-concurrency
│   │   ├── reconciliation
│   │   └── recovery
│   ├── capability/
│   │   ├── contracts
│   │   └── resolution
│   ├── capability-host/
│   │   ├── registry-and-health
│   │   ├── admission-and-generation-lifecycle
│   │   ├── materialization-and-publication-port
│   │   ├── bounded-effect-worker-owner
│   │   └── adapter-and-secret-ports
│   ├── model/
│   │   ├── context-manifest
│   │   └── task-and-response-contracts
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
│   ├── model-provider/
│   ├── local-process/
│   ├── secret-env/
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
| Shared contract mechanics | `milkdrift-contracts` owns bounded/canonical JSON mechanics and the common validated-string implementation; semantic rules remain in consuming domain crates |
| Actor/grant/policy/secret-reference authority | `milkdrift-authority::{identity,model,evaluator,secret,document}` remains pure and owns no transport authentication or live secret source; `milkdrift-daemon::auth` maps local credential references to those server-owned facts |
| Human/service/AI workflow control | `milkdrift-control::{document,command,policy,preset,service,adapter,controller,read}` owns strict proposals and application orchestration while durable revisions, authorization decisions, reconciliation, and events remain with their existing owners |
| External control protocol | `milkdrift-control-protocol` owns protocol 1.0 commands, stable errors/results, bounded read models/pages, feed-bound cursors/SSE observations, and layout schema 1 without async, HTTP, runtime, or storage types |
| Reusable control client | `milkdrift-control-client` owns version negotiation, bearer-authenticated typed HTTP calls, bounded safe-query retries and artifact ranges, and exact-cursor SSE reconnect |
| `blueprint/model` | `milkdrift-blueprint::model` (public types re-exported at crate root) |
| `blueprint/validation` | `milkdrift-blueprint::validation` |
| `blueprint/revision` | `milkdrift-blueprint::revision` |
| `blueprint/mutation` | `milkdrift-blueprint::mutation` |
| `runtime/scheduler` | `milkdrift-runtime::scheduler` and runtime controller admission/index decisions |
| `runtime/execution` | `milkdrift-runtime::{command,executor}` plus focused engine command-planning/completion/dispatch/scheduling/state/workspace modules; `projection::apply_core` is an exhaustive dispatcher into lifecycle, eligibility, execution, lease, observation, terminal, retry, and structured event-family reducers |
| `runtime/structured-concurrency` | Blueprint definitions, the runtime structured coordinator with separate repeat/subworkflow/reducer mechanics, branch/join/repeat/wait/signal/timer/subworkflow projection reducers and views, and `milkdrift-workspace::scope` |
| `runtime/reconciliation` | `milkdrift-runtime::reconciliation`, engine reconciliation, separate projection plan/action reducers and reconciliation views, and persistence-owned plan event documents |
| `runtime/recovery` | `milkdrift-runtime::{query,engine::recovery,projection::replay}`, the focused recovery reducer, and recovery indexes in the redb adapter |
| `capability/contracts` | `milkdrift-capability::{descriptor,invocation,document,identity,bounded}` |
| `capability/registry` | `milkdrift-capability-host::registry` owns bounded live registrations, observations, actual permit ownership, generation lifecycle, and queries |
| `capability/resolution` | Pure matching and exact snapshots remain in `milkdrift-capability`; deterministic authority/policy/health/capacity selection and the runtime executor bridge are in `milkdrift-capability-host` |
| `capability/effect-host` | `milkdrift-capability-host::worker` owns explicit fixed execution/control threads, bounded queues, bounded claim pages, health, panic containment, and deadline-driven drain/cancel/retain shutdown |
| `capability/materialization` | `milkdrift-capability-host::materialization` owns the exact workspace/artifact port and `RuntimeStore` bridge; concrete adapters see only isolated roots and capability-domain references |
| `workspace/context` | Immutable task policy in `milkdrift-blueprint::context`, exact manifest contracts in `milkdrift-model::context`, pure selection/publication in `milkdrift-runtime::context`, and scoped values/budgets in `milkdrift-workspace` |
| `workspace/artifacts` | Metadata/contracts in `milkdrift-workspace` and durable bytes in `milkdrift-redb-store` |
| `workspace/branches` | `milkdrift-workspace::scope` plus runtime branch/iteration/subworkflow projections |
| `persistence/events` | `milkdrift-persistence::{event,document}` with schema-v1 golden fixtures |
| `persistence/journal` | Narrow `milkdrift-persistence` ports implemented by `milkdrift-redb-store::journal::{append,discovery,queries,workspace}` |
| `persistence/projections` | Pure `milkdrift-runtime::projection`; optional checked snapshots use persistence envelope v2 around runtime projection payload v3 |
| `peer/protocol` | `milkdrift-peer-protocol::{document,identity,session,catalog,execution,artifact}` owns bounded transport-neutral protocol 1.0 messages and semantic state |
| `peer/capability-advertisement` | `milkdrift-peer-http::{service,remote}` derives authority-filtered expiring catalogs and maps exact remote generations into ordinary local capability registrations |
| `model/contracts` | `milkdrift-model::{task,context,document}` owns provider-neutral schema-v1 model tasks/responses and exact causal manifests without HTTP, runtime, provider SDK, or secret dependencies |
| `adapters/model` | `milkdrift-model-provider::{adapter,profile,http,stream,openai_compatible,anthropic}` owns endpoint policy, feature negotiation, bounded transport, two independent wire mappings, and artifact publication |
| `adapters/process` | `milkdrift-local-process::{config,process}` owns profile schema v1, direct argv entry, environment mediation, bounded pipes, declared imports, timeout/cancellation, and platform process ownership |
| `adapters/secret-env` | `milkdrift-secret-env` maps explicitly configured opaque references to exact environment names without enumerating or retaining values |
| `adapters/filesystem` | Content-addressed artifact ownership in `milkdrift-redb-store::artifact::{accounting,cleanup,path,publication}` |
| `adapters/redb` | The transactional local adapter, split across `milkdrift-redb-store::{admin,journal,store}` facades and their private child modules |
| `adapters/peer-transport` | `milkdrift-peer-http::{auth,config,http,client,store,artifact,remote,service}` owns fixed HTTPS/loopback transport, bearer identity, durable idempotency/observations, quotas, verified staging, and remote adapters |
| `apps/desktop-iced` | Not implemented |
| `apps/daemon` | `milkdrift-daemon::{config,auth,host,http}` owns validated local/peer configuration, credential-to-actor/peer mapping, redb/runtime/capability/effect/peer lifecycles, the bounded owner boundary, separated control/peer HTTP realms, readiness, and ordered shutdown |
| `apps/cli` | `milkdrift-cli` is a thin argument/presentation layer over `milkdrift-control-client`; it owns confirmations, stable JSON schema 1, output/download policy, and exit codes, never durable truth |

Physical crates are extracted only for a real dependency, lifecycle, host, publication, or multiple-consumer boundary. No empty crate or placeholder directory may be created merely to resemble the diagram. A later pass may merge or split physical packages when it preserves logical ownership and reduces coupling. Within a cohesive crate, private modules are preferred until extraction creates a measurable boundary; conversely, a growing module must split when unrelated invariants, dependencies, lifecycle, or test ownership become entangled.

## 17. Testing philosophy

Tests establish independent invariants rather than restating algorithms. Small hand-reviewed examples cover each semantic node and capability variance. Golden JSON fixtures own compatibility and exact canonical re-encoding. Property/model tests generate mutation batches and require every published revision to validate. Compile-fail examples prove private API boundaries.

The runtime and persistence layers must add deterministic state-machine tests, crash/restart and fault injection, duplicate/out-of-order delivery, uncertain-effect recovery, reconciliation histories, compatibility fixtures, and cross-adapter contract suites. Security-critical policy, graph validation, reconciliation, idempotency, and recovery logic should receive mutation testing once implemented; surviving mutants are missing assertions or deliberately justified equivalents. Coverage volume is not a substitute for testing the invariant from an independent observation.
