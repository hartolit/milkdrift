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
- A **prompt sequence** is a bounded operator import/template compiled into an ordinary immutable blueprint revision; it is not a runtime scheduler or node kind.
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

Compaction means removal from active operational state only. It never deletes run events, artifact content, artifact metadata, or output records. Retained summaries keep the stable execution/attempt/revision identities, sequence boundary, history digest, and live artifact/output references required to locate older evidence. External inspectors and the causal-context builder can reconstruct exact historical context on demand by paging the journal and resolving those durable references.

"Bounded" is relative to semantic liveness, not a universal constant-memory claim. Projection and checkpoint size may grow with workflow shape, intentionally active branches, unresolved safety obligations, retained outputs/artifacts/context, bounded worker ownership evidence, and configured workspace limits. For fixed workflow shape and bounded active concurrency, settled event count and elapsed iterations do not by themselves increase active state.

The headless runtime owns transition decisions, projections, leases, scheduling, recovery, and reconciliation through narrow persistence/execution ports. `milkdrift-capability-host::EffectWorkerHost` is the explicit embeddable owner of bounded caller-created effect and cancellation workers; it has no singleton or async runtime. The daemon owns that object together with registry, secret, redb, artifact, recovery, and shutdown lifecycles. Clients request commands and render projections; they do not own truth.

## 5. Prospective live reconciliation

Editing during a run produces a new immutable revision. Reconciliation compares the run's pinned/reconciled revision with an explicitly selected prospective revision. Completed, started, effect-dispatched, or otherwise historically committed node executions remain attached to their original definition and provenance. Only work that has not crossed its durable commitment boundary may be redirected. Ambiguity, incompatible interfaces, removed required work, changed authority, or uncertain effects produce a typed conflict/remediation state, never silent rewriting. The invariant is absolute: history never changes.

## 6. Capability contracts and resolution

Capability descriptors contain provider-neutral facts: identity and descriptor revision, stable category, namespaced operations, exact input/output schema contracts, streaming shapes, cancellation and idempotency behavior, side-effect class, admission limits, locality, trust zones, honest optional resource observations, and bounded extensions. Mutable health, availability, load, leases, credentials, and executor handles are separate.

Blueprint tasks carry capability requirements. Before a run starts or adopts a prospective revision, the runtime validates every reachable task, reducer, and nested workflow requirement against the run's frozen execution-authority envelope without requiring a live provider. `milkdrift-capability-host` then evaluates every exact semantic candidate against that authority before considering mutable health or capacity. Selection is stable by exact requirement, explicit priority, capability identity, and revision; only an allowed candidate can become the durable snapshot. Resolution, exact-generation claim, and final adapter entry each record a canonical decision, and execution and cancellation never fall back to another generation after selection. Namespaced features advertise tools, structured output, reasoning controls, embeddings, images, seeds, token accounting, cancellation, or other features only when actually supported. There is no permanent enum of every provider operation and no fabricated common denominator.

Invocation requests, progress/output events, cancellation exchange, terminal outcome, usage, retryability, side-effect status, and uncertainty are versioned provider-neutral contracts. Values and artifacts use bounded references. Adapters translate and report; they never decide run state.

### Peer-as-capability boundary

`milkdrift-peer-protocol` owns only v1.2 bounded transport-neutral session, catalog, invocation, originating execution provenance, observation, cancellation, delegation, archived-history disposition, and artifact-transfer messages. `milkdrift-peer-http` owns configured HTTPS/loopback transport, relationship authentication, the fixed dispatch-worker owner, resumable hot observations, archived terminal/uncertain replay, core artifact-transfer adaptation, and the ordinary remote `CapabilityAdapter`. Every invocation acceptance, lookup, observation page, and cancellation acknowledgement binds the exact request or URL identity a consuming client supplied. Narrow persistence ports own peer execution semantics and the redb adapter implements them; no HTTP/TLS type enters those contracts.

The serving daemon derives an expiring authority-filtered catalog from its live capability host. The consuming daemon remaps each exact remote identity/generation into a collision-resistant local identity with `Locality::Peer`, a typed peer identity, trust zone, and exact peer/catalog provenance, then registers it through `milkdrift-capability-host`. The catalog is live observation only. A run still records one exact local `ResolvedCapabilitySnapshot`; neither peer reads the other's database or shares mutable workflow state.

Remote acceptance is durably recorded before it is reported. Reusing an idempotency key with identical canonical facts returns the same remote execution from either the hot record or compact tombstone; different facts conflict permanently within the store generation. Active and reconnect-horizon records retain contiguous observation rows and a rolling digest. Eligible terminal/uncertain rows move oldest-first in one transaction to a compact tombstone that preserves identity, acceptance/provenance/authority/cancellation/accounting, observation count/digest, and the final terminal or uncertainty disposition while deleting detailed observation rows and peer observation-artifact links. Core artifacts remain under independent artifact retention. Connection closure proves neither cancellation nor terminal outcome. After accepted adapter-entry intent, missing restart evidence becomes explicit uncertainty instead of replacement execution. External side effects are never advertised as globally exactly once.

Connectivity is operator supplied through a reachable HTTPS path or an explicitly enabled loopback development route. Milkdrift does not discover peers, traverse NAT, create certificates, run a hosted coordinator, provide an overlay/VPN, synchronize models/tensors, share databases, or implement consensus.

## 7. Actors, authority, proposals, and AI control

Actor identity and scoped authority are distinct from revision authorship. Canonical `ActorRef` ownership is in `milkdrift-authority`; blueprint `AuthorRef` remains provenance only. Immutable schema-v4 grant revisions constrain commands and reads by workflow/run, typed operation, capability identity/category/operation/profile/trust/locality, normalized filesystem roots and access, credential-free network profiles/destinations, opaque secret references, exact artifact identity/sensitivity, shared-layout revision, peer identity, daemon diagnostic class, workspace scope, side-effect class, cost, duration, invocation, artifact, concurrency, validity, and revocation generation. Capability and artifact identities use explicit `Any` or bounded nonempty `Only` selectors beneath an explicit whole-scope `DenyAll`; absence or an empty collection never grants wildcard access. Layout authority is deny-all or explicitly selected shared revision state; actor-owned layouts remain reserved until protocol and persistence have one reviewed owner identity. Empty artifact, layout, peer, workspace, or explicit capability-deny scopes grant nothing. Pure policy evaluation records the actor, exact grant revision, evaluator policy/version, request facts, stable reason codes, evaluated constraints, caller-supplied boundary time, result, and deterministic digest.

Authentication establishes only an actor session and exact immutable grant revision. Every externally initiated local or peer command, information-bearing query, page, and subscription constructs a typed operation/resource request and passes through the same `AuthorityEvaluator` before its owner is called. Presets and peer action lists are deterministic configuration expansion into operation sets; they are never executable permission checks. Collections are filtered by the grant before hidden identities are fetched or projected. Protected artifact metadata and content are independently authorized against immutable stored sensitivity, and readiness is deliberately less detailed than health. Consequential mutations, protected artifact releases, and peer administration/invocation retain bounded decision provenance without logging credentials or payloads.

Every external run command presents an exact grant revision and digest to the runtime's injected evaluator before semantic acceptance. The exact decision is part of command-result schema v2 and is committed in the same transaction as acceptance events or denial-without-events; exact redelivery returns that original decision/result without reevaluation. On start, the runtime freezes an `ExecutionAuthorityBasis` that binds actor, grant identity/revision/digest, policy identity/version, workflow/run and initiating revision lineage, accepted decision provenance, and revocation generation. Structured child runs inherit that exact basis. Later revisions and attempts may narrow it but cannot replace or widen it.

Capability requirements are evaluated against the frozen envelope before start or revision adoption. Runtime selection constructs an exact request from descriptor identity/revision, category, operation, provider profile, locality, peer, trust zones, side-effect class, idempotency, adapter-declared filesystem/network/secret needs, and budget facts. The same canonical evaluator is called for candidate resolution, exact-generation claim, and immediately before adapter code. Each boundary's request and decision are durable provenance. Revocation or grant narrowing denies future resolution or entry with typed authorization evidence and releases any lease; it does not rewrite a capability that already entered or its eventual terminal truth.

System transitions and worker observations use separate private runtime-owned receipt paths. The local daemon authenticates referenced bearer secrets at request time and supplies configured immutable actor/grant context; actor identity is absent from command JSON. Peer transport authentication separately establishes a stable configured `PeerId`, while remote work remains bound to the initiating actor's execution basis. A hostname, display name, payload claim, descriptor, relationship, or valid credential never grants capability authority by itself.

An actor may inspect or propose without authority to apply. Approval is an explicit command/event linking proposal, exact revision or effect, approver, and policy. An AI controller is an ordinary task using a workflow-control capability under a scoped grant. It uses the same closed mutations, optimistic checks, proposals, approvals, and audit trail as a human; it has no hidden privileged node kind or mutable backdoor.

A workflow proposal is bounded, versioned, duplicate-key-safe, canonical, and digest-bound untrusted data. It names the exact workflow, base revision and digest, optional live run and observed sequence, one closed mutation batch, provenance, evidence/artifact references, rationale, risk notes, requested application policy, optional run action, and a claimed stop condition. Large reasoning remains in referenced artifacts rather than ordinary command documents. Model prose and tool calls are never control intent: only a successfully decoded structured-output value can become a proposal, and malformed or adversarial output terminates the producing task without direct workflow effect.

`milkdrift-control` is an application layer over existing owners. It privately builds and validates the complete prospective revision, derives an exact authority delta, classifies deterministic risk, stores the immutable candidate, and delegates live proposal, approval, apply, pause/resume/cancel/retry, and signal actions to `RuntimeService`. It never appends events or rewrites run history directly. Low-risk auto-apply is permitted only for future pure/read-only work under an explicit policy and an exact apply grant; terminal changes, existing/started work, side effects, provider/profile/trust expansion, subworkflow/interface changes, cancellation, and other elevated cases require the existing recorded approval path. Stale sequence, base, digest, plan, or proposal guards fail closed.

Observer, Advisor, Supervisor, Controller, and Autonomous are convenience names that expand into ordinary immutable grants with caller-supplied exact workflow/run, capability, budget, validity, and revocation scope. They are not runtime roles. The controller contracts describe continuous control as an ordinary acyclic wrapper around an explicit pinned `Repeat` body; no arbitrary graph cycle, privileged AI node, or unbounded background loop is introduced. The production daemon does not currently install that lifecycle or admit continuous controllers.

The controller builder embeds strict controller-policy schema 1 in immutable semantic revision metadata. Its `cp1_` digest binds the stable controller identity, exact wrapper workflow/node, pinned body workflow/revision/interface, every invocation/revision/proposal/time/cost/unit/artifact/process/model/failure/rejection/repeat/child limit, checkpoint interval, fail-closed unknown-usage policy, stop behavior, control-operation requirements, currency, labels, and provenance. Unknown policy versions, digest changes, wrapper/body contradictions, zero limits, and metadata-only legacy patterns fail validation. A controller cannot change or remove its own policy; another authorized actor can only change it by creating and reconciling a new immutable revision.

New resolved-capability snapshots freeze and digest the descriptor category used for controller process/model accounting. Existing schema-1 snapshots written before that additive fact remain replayable under their original digest; their absent category is conservatively accounted as both resource-bearing categories so legacy history cannot become a limit bypass.

`milkdrift-control::ControllerLifecycleOwner` is the one policy parser and accounting owner for focused library integrations and tests. An embedding may install it into `RuntimeService` only while admission is closed. Before activation and every repeat-cycle entry, the deterministic runtime supplies the exact run/revision/node/execution, caller-clock boundary, and current projection. The owner derives progress from durable child terminals, frozen resolved capability categories, attempt usage, logical artifact bytes, run-actor-attributed prospective revisions/rejections, controller start time, and the immutable body graph. Unconstrained tasks are conservatively counted as potential model and process work before child entry; admitted attempts are classified from the exact resolved descriptor snapshot. Missing model/process cost or input/output usage fails closed. The returned `Continue`, `HumanCheckpoint`, or `BoundReached` is recorded as `ControllerAssessmentRecorded` immediately before iteration creation in the same runtime commit. This projection-time accounting does not yet reserve or atomically decrement every cumulative process/model/artifact/cost limit at the final external-entry boundary, so the production daemon deliberately leaves the owner uninstalled and controller activation fails closed.

The controller policy owns cumulative ceilings. Repeat's native maximum is a one-iteration-higher structural guard so the canonical assessment records the exact reached bound before any excess child is created; controller wrappers do not duplicate duration/cost ownership in `RepeatBudget`. Exact mutation/node proposal dimensions are checked on the validated proposal before its revision is stored. Controller-authored approval and application reassess cumulative policy, while ordinary authority and reconciliation still decide the transition. A checkpoint is an existing durable repeat-continuation request with a digest-derived identity; continuation uses the normal authorized decision command, rechecks revocation and all limits, and records another assessment before approval. A reached bound records its dimension/current/limit (or explicit unknown usage), deterministically fails the controller without provider retry, and remains inspectable from the compact current frontier and journal after restart.

## 8. Execution semantics

Sequential execution is ordinary acyclic control/data edges. A typed **branch** selects one declared arm using a safe condition AST. A **fork** creates named isolated child branches under structured concurrency. A **join** is owned by one fork and waits for all, any successful, or a satisfiable quorum; cancellation/failure policy is explicit. A **reducer/compositor** is a separate node with a declared input shape and does not masquerade as synchronization.

A **repeat** explicitly invokes a pinned acyclic body, evaluates a safe condition, and has a hard maximum iteration count plus optional tighter time/cost budgets and a terminal limit policy. A **wait** is a durable timer definition. A **signal wait** resumes only from an authenticated, correlated external signal. A **subworkflow** pins an immutable revision and interface; reuse is reference/instantiation, not an untracked copy. Explicit success, failure, and cancellation terminals make workflow results inspectable. Arbitrary cycles are rejected; ongoing control uses repeat or future prospective revisions.

Structured concurrency means a parent scope owns its children, branch-local resources, join, cancellation, and cleanup. A run cannot report a scope complete while owned work remains unaccounted for.

An imported implementation prompt sequence is a product-facing composition of these primitives.
Each stage is a fresh or explicitly continued coding task, a distinct verification task, a safe
artifact-presence branch, and either the next stage or an explicit failure/review route. Review uses
a fresh causal policy and a signal wait. A remediation request creates an ordinary prospective
revision through the shared proposal and reconciliation service; completed executions retain their
original revision and never become eligible again. The import layer owns parsing, bounds, canonical
digests, generated identifiers, and template policy only. It owns no executor, repository, grant,
persistence, scheduler, or control privilege.

## 9. Workspaces, artifacts, context, provenance, and retention

Each concurrent branch receives an isolated logical workspace. Cross-branch values move only through declared data edges, artifacts, joins, reducers, or explicit merge operations. An artifact has identity, digest, media contract, size, producer, causal inputs, and retention class; large bytes stay outside semantic documents and ordinary events.

An operator may authorize one persistent host repository for sequential fresh-process tasks. The
local-process adapter revalidates that exact canonical directory under a declared read-write root at
each entry while keeping host-owned input/context materialization and output publication isolated.
This is a trusted-host authority decision, not a sandbox or Git implementation. Parallel work uses
separate worktrees/scopes and explicit merge/reducer capabilities.

Each task revision owns a private-invariant context policy, and task output metadata is the canonical owner of output semantic roles such as requirement, decision, implementation, verification, and review. One runtime candidate-source boundary searches a bounded recent journal tail through a frozen sequence, anchors settled executions from compact projection facts at their exact terminal sequences, and joins those durable facts with the exact historical revision graph, workspace, artifact metadata, scope lineage, and frozen execution authority. It discovers declared inputs, bounded ancestors, exact nodes/executions, tagged outputs, failures/uncertainty/decisions, explicit workspace/evidence references, joins, and imported subworkflow results as metadata before reading selected content. Historical ancestry uses the revision that governed each execution; a later reconciliation never reinterprets it. A join exposes only its declared result to semantic descendants of that exact join; sibling output and branch-local failures do not cross merely because their scope participated. Imported subworkflow output resolves through its exact durable subworkflow-to-parent-execution fact. Redacted branch/authority omissions do not disclose protected names or sizes. Stable order is causal depth, semantic kind, source node, execution, and canonical source-reference bytes. Candidate-scan, depth, event-summary, item, artifact-count, per-item, byte, artifact-byte, manifest-byte, and optional provider-neutral unit bounds are enforced deterministically. Optional losses receive stable policy, budget, authority, missing/corrupt, unsupported, superseded, or branch-isolation codes; required losses fail before dispatch. There is no chronological whole-history fallback and lifetime settled history alone cannot exhaust discovery.

The result is canonical context-manifest-body schema v2 inside the schema-v1 model document envelope, bound to one run/revision/node execution/attempt and to the exact policy version/digest. Each selected entry binds semantic tags, exact source digest/size, governing revision, execution/attempt/scope/sequence/time, producer actor/capability/descriptor/provider/peer/invocation, causal evidence, sensitivity/authority, and inclusion reason. The manifest also records bounded redacted omissions, totals, applied budget, and a domain-separated digest. The exact restricted manifest artifact is committed and journal-published before the scheduling event that can reach an adapter. Invocation-request schema v2 carries its compact reference. A retry attempt rebinds the exact prior selection to the new attempt without rescanning newer history; a policy-driven different selection is a distinct manifest and attempt.

After that durable boundary, only selected non-direct values/artifacts are materialized through authorized host-owned ports and verified against manifest digest, size, and media facts. Fresh model requests receive the canonical manifest as a system context block plus selected text/JSON or image evidence as explicitly delimited untrusted user data; negotiation covers those adapter-injected wire features and unsupported roles, images, or generic binary evidence are rejected before HTTP. Local-process profiles may explicitly map `milkdrift.context_manifest` and reserved selected-context input names using their existing input-file rules; no global context file is created. Output artifact provenance points to the manifest and exact invocation inputs. Retention policy may expire payloads while preserving safe metadata and integrity evidence; deletion is explicit and auditable.

`milkdrift-model` owns canonical provider-neutral model-task/response schema v1. Model identity and endpoint selection belong to exact capability profiles, not blueprints or the model contract. `milkdrift-model-provider` is an outer adapter with separate OpenAI-compatible and native Anthropic mappings over one bounded rustls/HTTP stack. It rejects unadvertised roles, parts, tools, schemas, reasoning, streaming, sessions, and encoded request bodies over the profile bound before provider entry. Model outputs and tool calls are data artifacts; no returned call is executed automatically.

## 10. Durable persistence and crash recovery

The persistence boundary appends checksummed, versioned events transactionally with command idempotency, workspace/accounting changes, and recovery indexes. Narrow application-state ports separately own external command receipts, presentation layouts, proposal discovery, and bounded security audit; they are not a generic key/value facility. An application receipt has exactly one authoritative physical placement in the hot or cold table. The bounded hot tier has a derived completion-order index; transparent cold storage retains the identical immutable receipt document and is not counted against the hot bound. Exact lookup and stable administration pages span both tiers. Redb atomically archives a bounded oldest-first hot batch, inserts a new receipt, and applies a same-store layout or proposal-index effect in one transaction, so capacity reclamation cannot expose dual or absent ownership and no configured record count limits store lifetime. Runtime effects remain authoritative in the existing runtime transaction and use stable internal command identities, so a crash after runtime acceptance but before application-receipt commit reconciles by replaying runtime acceptance and then committing the missing receipt. Durable timers, signals, leases, cancellation requests, eligible work, uncertain invocations, layouts, and application receipts survive process restarts.

`RuntimeService::open_closed` performs only physical schema compatibility checks and returns with admission closed. Startup recovery remains explicitly closed while the daemon validates active runtime state and probes application-state owners, then registers and health-checks adapters, recovers configured peer work, starts bounded effect workers, and resumes runtime admission last. Startup is bounded by active state; it does not traverse terminal history or rehash every artifact byte. A crash between external effect and acknowledgement becomes an explicit uncertain/reconciliation case governed by idempotency and side-effect facts, never guessed success.

Complete historical verification is a separate caller-owned administrative operation through resumable `StorageAdmin::scan_integrity` pages. Its redb implementation delegates the persisted physical phase order to run, scheduler, workspace, revision, snapshot, artifact, hot/cold receipt, receipt-order, layout, proposal-index, and security-audit integrity modules; cursor schema v2 uses physical phase tags `0..=41`. Artifact-content verification is opt-in and bound into the continuation cursor. `health()` may report a small metadata/index sample and exact receipt-tier accounting, but a clean sample is not proof that all history or artifact content is healthy. Corruption reached while recovering a nonterminal run or loading required application owners fails startup closed before admission; corruption isolated to unrelated terminal history is reported when that object is read or an explicit scrub reaches it.

Storage engines are adapters. No database type enters blueprint or capability semantics. The current pre-release store accepts only its exact physical and internal document formats; older and future formats are refused, and no migration is claimed. Recovery tests inject truncation/reordering/duplicate delivery/failed durable boundaries where the selected backend permits. Any future migration must add hand-reviewed old-format fixtures and a restartable protocol before compatibility is claimed.

The optional projection-snapshot persistence envelope is currently schema v2 and contains a runtime-owned projection payload at schema v4. Payload v4 adds exact terminal-sequence anchors to compact settled executions so bounded context discovery can address their durable evidence without a lifetime scan. The envelope uses canonical JSON with one strict, padded RFC 4648 standard-Base64 payload string; its domain-separated, length-framed BLAKE3 checksum binds the semantic metadata and decoded raw payload bytes rather than the Base64 text. At a selected checkpoint boundary, the accepted event transaction also records a domain-separated commitment to those exact projection payload bytes in history-chain record schema v2. Storage returns a snapshot as verified only when the envelope checksum, event-prefix digest, and append-time payload commitment all agree; structural JSON limits are checked lexically before the payload value tree is allocated. Envelope v1 and projection payload v1, v2, v3, or other unsupported checkpoints are not migrated; they are discarded and reconstructed from the journal. A snapshot covers one exact event sequence and cumulative history-chain digest and contains no lifetime event-ID or execution-ID collection.

## 11. Peer execution

A peer exposes remote capability advertisements and invocation/cancellation/event contracts over a versioned protocol. The origin owns workflow truth; the serving peer owns one durable remote record. Acceptance atomically binds the request/idempotency digest, relationship/catalog/capability generation, authority decision, capacity accounting, and dispatch availability. Fixed workers claim durable leases, record adapter entry separately, and never automatically replace known-entered work. Observations are append-only bounded rows; cancellation request/acknowledgement, disconnect, terminal evidence, and uncertainty remain distinct. Explicit archival retains idempotency, provenance, and security facts. Inbound/outbound bytes use the ordinary core artifact publication/read authority rather than a peer repository. Connectivity is a pluggable, user-provided transport; core does not own peer discovery, VPNs, overlay routing, or NAT traversal. A peer is remote capability access, not a second semantic truth owner.

## 12. Daemon and clients

One daemon process owns authoritative local durable state, scheduling, registries, effects, reconciliation, and secret mediation. `host` composes the lifecycle and bounded owner queue; focused child owners cover commands/receipts, definitions, runs/attempts, layouts, proposals, artifacts, and capability registration/queries. Operator configuration is bounded TOML schema 9: a raw duplicate/unknown-field-rejecting document is normalized and compiled once into immutable private-field storage, authentication, runtime, adapter, peer, and shutdown plans. Peer hosting is an explicit disabled/enabled sum type, so enabled-without-identity state is not representable after decoding. Internal owners receive only their effective section rather than mining a mutable global configuration document. Startup validates those plans and credential references, refuses legacy sidecar authority, opens redb once, recovers runtime with admission closed, loads application-state owners, registers and health-checks adapters, recovers peer work, starts bounded effect workers, resumes runtime admission, and exposes readiness only afterward. Shutdown closes HTTP and mutation admission first, begins peer/runtime draining, disconnects peer registries, applies the configured drain/cancel/retain effect policy, joins workers and the owner thread, and drops storage while reporting unresolved work truthfully.

Axum owns sockets and SSE framing only. Every runtime, redb, artifact, control, and registry operation crosses a bounded synchronous queue into one dedicated owner thread; a full queue returns a stable overload error. Each queue entry owns a one-shot closure with its concrete response channel captured inside it, so HTTP calls a typed `DaemonHost` method and receives that method's declared result directly. There is no parallel operation/result enum protocol and no runtime response-variant downcast to keep synchronized with the routes. The queue erases only the task's concrete type at the scheduling edge; authorization, refusal, result ownership, and shutdown intent remain explicit in the typed host method. External adapter work remains on the fixed `EffectWorkerHost` threads, so neither an HTTP task nor the runtime owner holds a global lock while awaiting a process/model stream. Periodic maintenance has a configured bound and a blocking notification wait rather than a busy loop.

Daemon lifecycle, queue occupancy, worker load, receipt retention, peer retention, and redacted
failure observations form one mutex-owned operational health projection rather than independent
atomics that can produce a torn read. One private update boundary advances the in-process feed
generation only when that projection changes, and stream cursors pair the generation with the same
locked snapshot. An owned guard accounts for each request admitted to the bounded owner queue and
releases occupancy on dequeue or drop. These facts govern only local admission and observability;
they are not workflow, execution-history, or storage truth.

`milkdrift-control-protocol` owns pure external protocol 2.2 DTOs. Protocol 2 makes the frozen run authority basis and resolution/claim/final-entry decisions part of every available attempt inspection; minor 1 adds redacted application-receipt lifecycle health, and minor 2 adds durable attempt output/progress/usage plus exact redacted process/model provenance. Closed protocol-1 clients are deliberately rejected rather than receiving an unversioned shape change. Its read models project immutable revisions, compact runs/nodes/attempts, proposal state, timelines, capabilities, authority, artifacts, peers, and health without serializing internal event variants, command/result bytes, or redb keys. Authenticated cursor schema 2 binds feed position to actor, exact grant identity/revision/digest, decision digest, query resource/filter digest, and a credential-derived MAC. A continuation cannot cross actor, credential rotation, grant replacement, or query scope. Streams authorize establishment, use the same bound cursor, and reauthenticate and reevaluate at each bounded polling cycle; revocation stops future disclosure and is distinguishable from transport failure. `milkdrift-control-client` is the only HTTP mapping used by the CLI and any separately authorized external client. The CLI cannot create hidden state or directly mutate journals.

Layout document schema 1 is presentation-only state with exact workflow/revision association, positions, annotations, viewport, authenticated author, update generation, and its own digest. The daemon persists it through the application-layout port in an independently keyed redb row under optimistic generation checks. Layout never contains semantic edges, task configuration, prompts, secrets, or capability requirements and never changes a blueprint digest.

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
{authority, blueprint, capability, contracts, control, persistence, workspace} -> prompt-sequence
{authority, blueprint, capability-host, control, persistence, runtime, redb-store,
 local-process, model-provider, control-protocol, prompt-sequence} -> daemon
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
the HTTP stack exists only in `milkdrift-control-client` and `milkdrift-daemon`.
`milkdrift-prompt-sequence` is an outer import/template layer over stable semantic, authority,
control, persistence-identity, and workspace contracts; it emits ordinary revisions/proposals and
cannot call adapters or persistence. Dependencies
may point toward stable semantics, never from semantics toward a host.

Forbidden in the semantic crates are Tokio or another executor, HTTP clients/servers, databases, Iced, provider SDKs, subprocess/OS APIs, transport types, secret values, tensor/inference types, live handles, clocks, randomness that affects identity, and mutable singleton registries. Project-authored code is safe Rust unless an independently proven requirement has a focused safety contract and tests.

## 14. Security, secrets, and untrusted input

Every disk, wire, provider, tool, peer, imported blueprint, artifact, path, signal, and AI-produced proposal is untrusted input. Readers enforce schema version, byte/count/depth/string/path bounds before expensive work; reject unknown core semantics; and preserve only bounded namespaced extensions. Conditions are data ASTs, not scripts.

Credentials and secret values never appear in blueprints, descriptors, requirements, events, command bodies, diagnostics, logs, or peer advertisements. `SecretRef` serializes only an opaque reference, while resolved `SensitiveSecret` values are non-serializable, non-clone, redacted, and exposed only through a narrow closure. The local daemon accepts only loopback plaintext, requires an enabled bearer-reference binding, compares credential digests in constant time, rereads file/environment references for rotation, and maps a match to server-owned actor/grant facts. Authentication and authority remain separate; permissive CORS is absent. The host owns the resolver port; `milkdrift-secret-env` resolves only explicitly mapped references and never enumerates the environment. Local-process profiles are argv templates, never shell command strings; each substitution remains one OS argument. The child begins from `env_clear`, receives only allowlisted ambient names and resolved secret refs, and secret-bearing profiles cannot stream process text. Filesystem/process effects require canonical allowlisted roots, isolated materialization, bounded regular files, traversal/symlink/hardlink rejection, and declared output imports. Side effects, authority decisions, hostile output, cancellation observations, and uncertain outcomes are provenance facts. Budget and termination controls are enforced by their owning boundary, not trusted to an AI prompt.

Every capability descriptor has an exact execution trust class. The current local-process adapter is `TrustedHostProcess`: the child runs with daemon-account host privileges, and mediation of argv, environment, materialization, import paths, and output bounds is not an isolation boundary around arbitrary executable behavior. `SandboxedProcess` is reserved for a separate adapter that actually enforces a complete container, namespace, VM, or equivalent boundary. Exact requirements and authority scopes may constrain the class, so sandbox-required work cannot resolve to the host-process adapter.

Local-process profile schema v2 binds one immutable generation to the operator-declared BLAKE3 executable digest and size, optional package/deployment revision, safe configured/canonical path digests, regular-file/platform observations, full profile digest, execution-policy digest, trust class, and process-ownership facts. Registration uses bounded streaming verification before constructing the descriptor. Health and the immediately-pre-spawn boundary re-resolve the path and reverify the same bytes, root, metadata, and identity; a mismatch makes that adapter generation sticky-unavailable and requires explicit registration of a new revision. The frozen resolution snapshot retains bounded descriptor-extension provenance, which the authorized attempt inspector exposes without host paths. Portable safe Rust still leaves a minimized race between final verification and OS entry; no atomic open-handle execution guarantee is claimed.

## 15. Disk/wire schemas and compatibility

Portable documents use explicit numeric schema versions: blueprint revision/mutation, invocation request, the context-manifest body, local-process profiles, prompt-sequence imports are currently v2; durable hot peer-execution records are v3; run-event envelopes are v2; resolved-capability snapshots are v2; authority decisions are v2; daemon configuration is v9; authority grants are v4; external authenticated cursors are v2; the external control protocol is 2.2 and peer protocol is 1.2; CLI JSON output, layout documents, durable application receipts/layouts/peer-execution tombstones, the model document envelope, other capability documents, model-task/response contracts, and endpoint profiles are schema 1. Redb physical schema 8 and internal document format 11 are exact-current and deliberately refuse older or future stores without migration. Artifact authority uses an explicit whole-scope `DenyAll` or a nonempty `Any`/bounded-`Only` identity selector paired with nonempty sensitivities; layout authority uses `DenyAll` or explicit shared-layout revision selectors. Empty collections never mean wildcard, and actor-owned layouts remain reserved rather than advertised as executable daemon state. Obsolete file-per-peer-execution and parallel peer-artifact directories are explicitly refused. Daemon configuration v8 and earlier is refused: v8 was the superseded JSON/global-document boundary, and earlier selector intent also cannot be recovered safely. Prompt-sequence v1 is refused because its checkpoint, stage-budget, and profile shapes were not all executable. Context-manifest v1 is refused because it cannot prove selected content or exact producer provenance. Local-process profile v1 is refused because its path-only executable reference cannot prove implementation identity. Digest inputs use recursively key-sorted deterministic JSON and deterministic collections. Unknown core variants, malformed typed identities, invalid derived fields, and unsupported future versions fail clearly. Explicit bounded DNS-namespaced extension maps are the only forward-compatible unknown field mechanism.

Readers support only versions they can interpret without guessing. A writer emits one current canonical version. Adding optional meaning still requires a schema review; changing existing meaning or canonical bytes requires a new version and fixtures. Old golden fixtures remain read tests for every supported version. Disk events, projections, daemon commands, layouts, peer messages, and artifacts each declare independent version ownership rather than sharing one global version.

Run-event envelopes are durable internal execution truth, not a promise that the
daemon exposes the storage schema directly. External historical read
models are separately versioned, paged, authorization-aware projections over
that truth. They may redact or reshape fields without changing, replacing, or
claiming ownership of the append-only event contract.

## 16. Logical ownership and crate extraction

The following map matches the current physical workspace. Focused private modules
within these packages carry the logical responsibilities described in the table
below; absent products are not represented as placeholder directories.

```text
milkdrift/
├── crates/
│   ├── authority/
│   ├── blueprint/
│   ├── capability/
│   ├── capability-host/
│   ├── contracts/
│   ├── control/
│   ├── control-client/
│   ├── control-protocol/
│   ├── model/
│   ├── peer-protocol/
│   ├── persistence/
│   ├── prompt-sequence/
│   ├── runtime/
│   └── workspace/
├── adapters/
│   ├── local-process/
│   ├── model-provider/
│   ├── peer-http/
│   ├── redb-store/
│   └── secret-env/
├── apps/
│   ├── cli/
│   └── daemon/
├── tools/
│   └── evidence/
└── docs/
```

The exact current logical-to-physical mapping is:

| Logical responsibility | Current physical crate/module |
| --- | --- |
| Shared contract mechanics | `milkdrift-contracts` owns bounded/canonical JSON mechanics and the common validated-string implementation; semantic rules remain in consuming domain crates |
| Actor/grant/policy/secret-reference authority | `milkdrift-authority::{identity,model::{capability,decision,execution,grant,resource},evaluator,secret,document}` remains pure and owns no transport authentication or live secret source; `milkdrift-daemon::auth` maps local credential references to those server-owned facts |
| Human/service/AI workflow control | `milkdrift-control::{document,command,policy,preset,service,adapter,controller,read}` owns strict proposals and application orchestration while durable revisions, authorization decisions, reconciliation, and events remain with their existing owners |
| External control protocol | `milkdrift-control-protocol` owns protocol 2.2 common envelopes/cursors/codecs with focused private `command`, `read`, and `layout` modules for mutation DTOs, observation/read DTOs, and layout schema 1; it contains no async, HTTP, runtime, or storage types |
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
| `workspace/context` | Immutable task policy/output roles in `milkdrift-blueprint::context`, exact schema-v2 manifest contracts in `milkdrift-model::context`, authoritative candidate discovery with focused direct-input and selected-materialization owners in `milkdrift-runtime::context::source`, pure selection/publication in `milkdrift-runtime::context`, and scoped values/budgets in `milkdrift-workspace` |
| `workspace/artifacts` | Metadata/contracts in `milkdrift-workspace` and durable bytes in `milkdrift-redb-store` |
| `workspace/branches` | `milkdrift-workspace::scope` plus runtime branch/iteration/subworkflow projections |
| `persistence/events` | `milkdrift-persistence::{event,document}` with exact legacy schema-v1 and current schema-v2 golden fixtures |
| `persistence/journal` | Narrow `milkdrift-persistence` ports implemented by `milkdrift-redb-store::journal::{append,discovery,queries,workspace}` |
| `persistence/projections` | Pure `milkdrift-runtime::projection`; optional checked snapshots use persistence envelope v2 around runtime projection payload v4 |
| `peer/protocol` | `milkdrift-peer-protocol::{document,identity,session,catalog,execution,artifact}` owns bounded transport-neutral protocol 1.2 messages and semantic state |
| `peer/capability-advertisement` | `milkdrift-peer-http::{service::{authority,catalog},remote}` derives authority-filtered expiring catalogs and maps exact remote generations into ordinary local capability registrations |
| `model/contracts` | `milkdrift-model::{task,context,document}` owns provider-neutral schema-v1 model tasks/responses and schema-v2 exact causal-manifest bodies without HTTP, runtime, provider SDK, or secret dependencies |
| `adapters/model` | `milkdrift-model-provider::{adapter,profile,http,stream,openai_compatible,anthropic}` owns endpoint policy, feature negotiation, bounded transport, two independent wire mappings, and artifact publication |
| `adapters/process` | `milkdrift-local-process::config` owns byte-pinned trusted-host profile schema v2 and immutable descriptor construction; private `process::{identity,prepare,spawn,streams,monitor,outputs,reporting,platform}` modules own bounded identity verification, direct argv entry, environment mediation, pipes, declared imports, timeout/cancellation, and platform process ownership |
| `adapters/secret-env` | `milkdrift-secret-env` maps explicitly configured opaque references to exact environment names without enumerating or retaining values |
| `adapters/filesystem` | Content-addressed artifact ownership in `milkdrift-redb-store::artifact::{accounting,cleanup,path,publication}` |
| `adapters/redb` | The transactional local adapter, split across `milkdrift-redb-store::{admin,journal,store}` facades and their private child modules; `peer::{accounting,integrity,retention,validation}` owns durable peer accounting, whole-store verification, archival compaction, and document validation around the cohesive execution-store implementation |
| `adapters/peer-transport` | `milkdrift-peer-http::{auth,config,http,client,store,artifact,dispatch,remote,service::{artifact_transfer,authority,catalog,lifecycle,worker}}` owns fixed HTTPS/loopback transport, bearer identity, bounded worker lifecycle, core artifact transfer, quotas, and remote adapters; redb owns durable peer rows through persistence ports |
| `apps/desktop-iced` | Not implemented |
| `apps/daemon` | `milkdrift-daemon::{config,auth,host,http}` owns validated local/peer configuration, credential-to-actor/peer mapping, redb/runtime/capability/effect/peer lifecycles, the bounded owner boundary, separated control/peer HTTP realms, readiness, and ordered shutdown; `host::read_model` separately owns external DTO/error projection |
| `apps/cli` | `milkdrift-cli` is a thin argument/presentation layer over `milkdrift-control-client`; it owns confirmations, stable JSON schema 1, output/download policy, and exit codes, never durable truth |

Physical crates are extracted only for a real dependency, lifecycle, host, publication, or multiple-consumer boundary. No empty crate or placeholder directory may be created merely to resemble the diagram. A later pass may merge or split physical packages when it preserves logical ownership and reduces coupling. Within a cohesive crate, private modules are preferred until extraction creates a measurable boundary; conversely, a growing module must split when unrelated invariants, dependencies, lifecycle, or test ownership become entangled.

Pre-release public APIs follow current consumers under
[`docs/reference/public-api-policy.md`](docs/reference/public-api-policy.md). Semantic contracts,
adapter ports, validated serialized documents, and application entry points remain public.
Provider wire payloads, storage rows, daemon read-model projection, operational profile inspection,
and test fault hooks stay private or explicitly feature-gated with their owners. In particular,
peer HTTP does not re-export persistence records merely because it consumes the persistence port.

## 17. Testing philosophy

Tests establish independent invariants rather than restating algorithms. Small hand-reviewed examples cover each semantic node and capability variance. Golden JSON fixtures own compatibility and exact canonical re-encoding. Property/model tests generate mutation batches and require every published revision to validate. Compile-fail examples prove private API boundaries.

The runtime and persistence layers must add deterministic state-machine tests, crash/restart and fault injection, duplicate/out-of-order delivery, uncertain-effect recovery, reconciliation histories, compatibility fixtures, and cross-adapter contract suites. The structured-runtime integration target keeps shared contract builders in `structured_runtime/builders.rs` and scenario families in independent child modules; scenario-specific fakes remain local instead of growing one universal backend. Security-critical policy, graph validation, reconciliation, idempotency, and recovery logic should receive mutation testing once implemented; surviving mutants are missing assertions or deliberately justified equivalents. Coverage volume is not a substitute for testing the invariant from an independent observation.
