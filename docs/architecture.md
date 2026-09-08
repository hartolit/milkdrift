# Architecture

Milkdrift separates the definition of work, the history of its execution, and authority to change
what happens next. A client submits intent to the daemon; the runtime accepts facts durably and
asks external capabilities to perform work. Clients inspect projections of those facts instead
of opening storage or reconstructing adapter behavior themselves.

This document explains which component owns each part of that operation and the invariants that
connect them. The terminology and package map below support the detailed lifecycle sections.
Read [vision](product/vision.md) for intent, [status](product/status.md) for exact current versions
and qualification, and [the source-learning route](README.md#learning-the-implementation) to trace
the executable system. Engineering policy belongs in [engineering rules](development/engineering-rules.md).

## Terminology

- **Blueprint:** reusable declarative workflow or subworkflow package. **Workflow:** top-level
  identity with revision lineage. **Revision:** immutable semantic snapshot with exact parents.
- **Node:** definition-time unit. **Node execution:** one runtime occurrence. **Attempt:** one
  invocation under an exact capability selection. **Run:** durable execution pinned to a revision
  lineage. **Edge:** explicit control and/or typed data dependency.
- **Requirement:** what a task needs. **Descriptor:** immutable advertisement of one capability
  generation. **Observation:** mutable health/load/availability or a reported execution fact.
- **Author:** bounded revision provenance. **Actor:** authenticated principal. **Grant:** immutable
  scoped authority revision. **Controller:** an actor allowed to influence future work.
- **Command:** validated intent with authority, idempotency, and optimistic expectations.
  **Event:** immutable accepted fact. **Projection:** derived operational state.
  **Effect:** interaction with an external capability or infrastructure.
- **Workspace:** scoped logical state. **Artifact:** immutable content reference with digest,
  media type, size, sensitivity, producer, causal inputs, and retention class.
  **Context manifest:** exact selected and omitted task evidence.
- **Reconciliation:** prospective application of a revision to live work. **Prompt sequence:**
  bounded import/template compiled into ordinary revisions. **Layout:** presentation state outside
  semantic identity. **Peer:** remote execution host with its own accepted invocation records.

## Owners and dependency direction

The physical workspace below is the complete package map. Names in the first column correspond to
directories; every package is named `milkdrift-` plus the final directory component, except the CLI
executable, which is `milkdrift`. Private children organize each owner's implementation.

| Package directory | Owned responsibility and principal modules |
| --- | --- |
| `crates/contracts` | Shared canonical/bounded JSON, validated-string, digest-lexical, UTF-8 truncation, and validating-deserialization mechanics. Domains own meaning and wire shapes. |
| `crates/capability` | Identities, requirements, descriptors, pure matching, resolved snapshots, invocation/observation/cancellation contracts (`descriptor`, `invocation`, `document`). |
| `crates/blueprint` | Immutable graph, structured definitions, task/context policy, validation, revisions, mutation (`model`, `validation`, `revision`, `mutation`, `context`). |
| `crates/workspace` | Scope lineage, values, artifact metadata, provenance, and workspace budgets. |
| `crates/authority` | Actors, grants, resource selectors, decisions, frozen execution basis, pure evaluation, and secret references (`model`, `evaluator`, `document`, `secret`). |
| `crates/model` | Provider-neutral task/response and exact causal-manifest contracts (`task`, `context`, `document`). |
| `crates/persistence` | Durable documents and narrow journal/revision/workspace/artifact/application/peer/clock ports; controller account validation and transitions. |
| `crates/runtime` | Commands, scheduling, final entry/reporting, structured work, projection, recovery, reconciliation, and causal discovery/selection (`engine`, `projection`, `context`). |
| `crates/capability-host` | Live generations, selection/permits, adapter port, bounded workers, secrets/materialization ports, and `RuntimeStore` bridge (`registry`, `worker`, `materialization`). |
| `crates/control` | Proposals, deterministic risk, grants/presets, controller lifecycle, application orchestration, and ordinary workflow-control adapter (`service`, `policy`, `controller`, `adapter`). |
| `crates/prompt-sequence` | Strict JSON/Markdown imports, ordinary blueprint compilation, stage association, and prospective remediation; no executor or storage. |
| `crates/control-protocol` | External command/read/layout DTOs, codecs, negotiation, authenticated cursors; no HTTP/runtime/storage types. |
| `crates/control-client` | Typed authenticated HTTP, bounded safe-query retries and artifact ranges, exact-cursor SSE reconnect. |
| `crates/peer-protocol` | Transport-neutral session/catalog/execution/cancellation/artifact contracts and strict codecs. |
| `adapters/local-process` | Profile validation and byte identity; direct argv, preparation, streams, monitoring, outputs, and platform process ownership (`config`, `process`). |
| `adapters/model-provider` | Endpoint policy, feature negotiation, bounded HTTP/SSE, independent OpenAI-compatible and Anthropic mappings, artifact publication. |
| `adapters/local-secret` | Explicit environment/restricted-file references; no enumeration or retained secret values. |
| `adapters/peer-http` | Authentication, configured transport, catalogs, remote capability adapters, fixed dispatch workers, artifact transfer, and peer service lifecycle. |
| `adapters/redb-store` | Physical schema/transactions, journal/indexes, snapshots, accounts, application/peer retention, artifacts, and administrative integrity scans. |
| `apps/daemon` | Configuration compilation, authentication, single owner queue, command/read adaptation, HTTP/SSE, startup/maintenance/shutdown (`config`, `auth`, `host`, `http`). |
| `apps/cli` | Arguments and presentation over the control client; bounded input/output/session/streaming owners and command-family routing. |
| `tools/evidence` | Unpublished development leaf: actual-binary harness, mutation runner, benchmarks, operational and external evidence. |

Dependencies point toward stable semantics. Capability knows nothing about blueprint; blueprint
uses pure requirements and schema identities. Persistence consumes immutable domain contracts and
owns no runtime decisions. Runtime and redb are sibling consumers of persistence; runtime's redb
dependency is development-only. Capability-host implements runtime's `TaskExecutor`; adapters
implement the host-owned port without opening redb or deciding run state. Control consumes the
semantic, authority, persistence, runtime, model, and host boundaries and delegates to their owners.

```text
semantic contracts ← persistence ports ← runtime ← capability host ← adapters
        ↑                    ↑              ↑              ↑           ↑
        └────────────── control/import ──────┘              └── daemon ─┘
control protocol ← control client ← CLI                         ↑
        └───────────────────────────────────────────────────────┘
```

The diagram shows dependency layers, not every Cargo edge. Daemon explicitly composes concrete
storage, secret, process/model/peer adapters. The CLI directly consumes only control-client,
control-protocol, and prompt-sequence. HTTP, database, OS, provider, and async types stay outside
semantic contracts. No UI or local inference package exists. Exact manifest boundaries are checked
by `tools/evidence/tests/repository_contracts.rs`.

Canonical capability-owned identities (`PeerId`, `SchemaId`, `ExtensionKey`, `BoundedJson`,
`TrustZone`) are imported directly; consuming domains do not re-export alternative owners.
[Public API policy](reference/public-api-policy.md) governs exports and test-only features.

## Definitions and prospective control

A revision is created by one complete versioned mutation batch against genesis or an exact
optimistic base. The private candidate must validate before publication. Identity includes
semantic content and exact parents, excluding layout, time, and map/JSON object ordering.
Multiple parents mean a deliberately resolved merge, never automatic conflict invention.

Runs record append-only accepted events. Reconciliation preserves completed, started,
effect-dispatched, and otherwise committed executions under their original revision and
provenance. Only uncommitted future work may be redirected. Incompatible interfaces, required work
removal, stale sequence/base/digest/plan guards, authority changes, or uncertain effects produce
typed refusal/conflict/remediation. The closed reconciliation matrix owns both classification and
allowable actions; a new definition cannot reinterpret old evidence.

Proposals are bounded, duplicate-safe, canonical untrusted documents binding workflow, exact base,
optional run/sequence, mutation batch, provenance, evidence, rationale, risk, application policy,
optional run action, and stop condition. Large reasoning stays in artifacts. Only decoded structured
model output can become a proposal; prose and returned tool calls do not execute control actions.

Control privately validates the candidate, computes authority delta and deterministic risk, stores
the revision, and delegates acceptance to runtime. It never appends run events itself. Low-risk
auto-apply requires explicit policy and exact apply authority and is limited to future pure/read-only
work. Terminal/started work, side effects, profile/trust expansion, subworkflow/interface changes,
cancellation, and elevated changes require recorded approval. Approval links exact proposal,
revision/effect, approver, and policy. Preset names expand into grants rather than runtime roles.

## One command and external-effect path

Consider a client starting a run. The reply confirms the accepted command; task completion arrives
later through durable adapter observations. Two connected paths preserve that distinction:

```text
client request
    |
    v
authentication → authority → durable command result → reply
                                  |
                                  v
                              scheduler
                                  |
                                  v
                           capability entry
                                  |
                                  v
                        observations → read models
```

If the reply is lost, exact request replay recovers the command result. If an external operation
loses its result after entry, the runtime may retain an uncertain attempt. Command replay does
not turn that missing external evidence into a success or authorize another invocation.

Each externally initiated mutation binds actor, exact grant revision/digest, command identity,
canonical complete request, and optimistic guards. Authentication supplies server-owned actor
facts; command JSON cannot claim them. Runtime acceptance or denial-without-events commits the
canonical authority decision and result atomically. Exact redelivery returns that decision/result
without reevaluation; changed canonical facts conflict.

Start freezes an `ExecutionAuthorityBasis`: actor, grant and policy identities/versions/digests,
workflow/run/initiating revision lineage, decision provenance, and revocation generation. Children
inherit that basis. Later revisions may narrow it, never replace or enlarge it. All reachable task,
reducer, and nested-workflow requirements are checked against the frozen envelope before start or
adoption without needing a live provider.

The host evaluates every semantic candidate against authority before mutable health or capacity.
Stable selection uses the exact requirement, explicit priority, capability identity, and revision.
Resolution, exact-generation claim, and immediately-before-entry evaluation retain canonical
decisions. Their requests bind descriptor category/operation/profile/trust/locality/peer,
side effects/idempotency, adapter filesystem/network/secret needs, and budgets. Execution and
cancellation never fall back to another generation. Revocation denies future entry and releases
leases without rewriting already-entered work.

An adapter must declare immutable authority requirements and explicit start, drain, and shutdown
behavior. Start finishes before registration is published. Health preserves the caller's boundary
time. Cancellation acknowledgements bind invocation and request sequence. Registry locks are not
held over adapter code; fixed caller-owned workers use bounded queues, claim pages, panic
containment, and deadline-driven shutdown. Every adapter runs the shared conformance suite.

`TaskExecutor` has one incremental durable reporter path. Adapters submit observations; runtime
owns sequence acceptance, terminal uniqueness, heartbeat lease extension, and workflow transitions.
A durable terminal observation outranks a later worker failure. Missing terminal evidence after
entry remains uncertain. Retry eligibility follows recorded side-effect and idempotency facts;
neither cancellation requests nor connection closure prove that an external effect stopped.
Worker/system receipts are private runtime paths, not alternate external authority.

## Scoped authority and disclosure

All local/peer commands, information-bearing reads, pages, and subscriptions use the same pure
evaluator. Grants constrain workflow/run, operation, capability identity/category/operation/profile,
execution trust/locality, paths/access, network, secrets, artifact identity/sensitivity, layout,
peer, workspace, diagnostic class, budgets, validity, and revocation. Allow dimensions use explicit
`Any` or bounded nonempty `Only` beneath whole-scope `DenyAll`; empty collections cannot grant
wildcards. Shared-layout revisions are explicit; actor-owned layout vocabulary is reserved.

Collections are filtered before hidden identities are fetched or projected. Artifact metadata and
content are independently authorized against stored sensitivity. Readiness discloses less than
health. Consequential mutations, protected releases, and peer administration retain bounded
decision provenance without credentials or payloads. Authentication and grants remain separate
even for valid tokens, peer relationships, descriptor labels, or model role claims.

Bearer digests are compared in constant time. Explicit environment/file references are reread for
rotation through the same secret resolver used by adapters and peers. The resolver bounds values
and never enumerates ambient environment. [Authority configuration](operations/authority.md) owns
selector syntax, path grammar, dangerous-scope acknowledgements, and restart/revocation procedure.

External cursors bind feed position, actor, exact grant, decision, query/filter digest, and a
credential-derived MAC. They cannot cross credential rotation, actor, grant replacement, or query
scope. Streams authenticate and authorize establishment and every bounded poll; revocation stops
future disclosure distinctly from transport failure. External historical DTOs are separately
versioned projections, never serialization of internal events, redb rows, or provider payloads.
[Control API](reference/control-api.md) owns wire details.

## Structured execution

Control/data edges are acyclic. Branch uses a safe condition AST to select a declared arm. Fork
creates named isolated child scopes. Its join waits for all, any successful, or a satisfiable
quorum with explicit failure/cancellation policy; reducer is a separate declared data operation.
Repeat invokes a pinned acyclic body with a hard iteration maximum, optional tighter time/cost
limits, and terminal limit policy. Wait/timer and authenticated correlated signal waits are durable.
Subworkflow pins a revision and interface rather than copying mutable definitions. Explicit
success/failure/cancellation terminals cannot complete a scope with unaccounted owned work.

Prompt-sequence compilation emits fresh/explicitly continued coding tasks, distinct verification,
artifact-presence branches, review/signal holds, and ordinary prospective remediation. It owns
import bounds, canonical digests, identifiers, and template policy only. Repository ownership,
authority, scheduling, and effects stay with existing owners.

## Context and artifacts

Context answers what evidence one task should receive. A task revision requests sources and
budgets; runtime selects from causally visible work and records the selection before dispatch.
The capability host then loads the selected bytes. Keeping selection separate from loading lets
an inspector explain what the attempt received even after newer work or a revision exists.

Branches isolate mutable workspace state. Cross-branch transfer requires declared data edges,
artifacts, joins, reducers, or explicit imports. An authorized persistent host repository is an
explicit sequential-process choice; parallel work requires separate worktrees/scopes and merge
capabilities. It does not imply a Git implementation or sandbox.

Task revisions own context policy and output semantic roles. Runtime discovers metadata through a
bounded recent journal tail frozen at one sequence plus compact projection anchors at exact
historical terminal sequences. It joins governing revisions, scope lineage, workspace/artifact
metadata, and frozen authority. Sources include inputs, bounded ancestors, exact executions, roles,
failures/decisions, explicit evidence, join results, and imported subworkflow outputs. Lifetime
settled history alone cannot exhaust discovery; there is no whole-history chronological fallback.

Historical ancestry follows each execution's governing revision. A join exposes only its declared
result to descendants of that exact join; sibling failures and private output cannot cross by
association. Subworkflow imports follow their exact durable parent link. Omission records must not
reveal protected identities or sizes. Ordering is causal depth, semantic kind, node, execution, and
canonical reference bytes. Scan/depth/event-summary/item/artifact/per-item/byte/manifest/unit
bounds apply deterministically. Optional losses retain policy/budget/authority/missing/corrupt/
unsupported/superseded/isolation reasons; fail-closed policy requires required losses to fail before
dispatch. Current enforcement gaps in required checks, omission redaction, and session intent are
recorded in [status](product/status.md#limitations-now) and the
[runtime builder](../crates/runtime/src/context.rs); the declared policy is not proof of enforcement.

The canonical manifest binds run/revision/execution/attempt and policy digest. Each selection binds
content digest/size, semantic tags, governing revision, scope/sequence/time, producer actor,
capability/generation/profile/peer/invocation, causal evidence, sensitivity, authority, and reason.
Omissions, totals, budget, and a domain-separated digest complete the record. Its restricted
artifact is committed and journal-published before scheduling can reach an adapter; invocation
carries the compact reference. A retry rebinds the prior selection to its new attempt without
rescanning later history. A different selection requires a distinct manifest/attempt.

The host materializes only selected non-direct content after authorization and verifies digest,
size, and media facts. Model requests receive the manifest as system context and delimited
untrusted evidence; negotiation includes injected features. Unsupported roles/images/binary parts
fail before HTTP. Processes explicitly map reserved manifest/context inputs through existing
input-file policy; no ambient global context file appears. Outputs retain manifest/input provenance.

Artifact publication uses bounded resumable chunks, exact offsets, digest/size checks, and atomic
metadata/accounting acceptance after content publication. Read authority, integrity verification,
and ranges remain independent. Abort/cleanup release owned reservations; replay and content
deduplication do not duplicate logical charges. Explicit retention may expire bytes while keeping
safe metadata and integrity evidence; compaction never silently deletes artifact content or outputs.

## History, compaction, and recovery

Long-running workflows need complete history without keeping every settled attempt in the active
scheduler state. The journal retains accepted facts; the projection keeps what current work still
needs. Compaction changes the latter, so historical inspection must follow retained identities
back to journal pages rather than interpreting a compact view as the entire run.

| State | Authority and bound |
| --- | --- |
| Journal | Complete immutable event history and exact command results. |
| Active projection | Current operational obligations and compact terminal frontiers. |
| Snapshot | Optional bounded, verified recovery checkpoint. |
| Historical read model | Stable-cursor paged reconstruction of accepted facts. |

Projection folding retains full occurrences while eligible, dispatched, retrying, uncertain,
cancelling, awaiting successor scans, owning structured work, or carrying unconsumed import or
reconciliation obligations. Once closed, each current scope/node frontier retains at most one
terminal summary: governing revision, creation/terminal boundaries, attempt identity/count,
conservative side effects, selected route, and live output/provenance references.

Settled progress, old attempts/leases, consumed signals, fired timers, superseded frontiers and
revision pins, closed recovery/reconciliation records, and consumed branch/repeat/subworkflow
ownership retire to the journal. Stable identities, sequence anchors, and history digests locate
older evidence. Fixed workflow shape and bounded concurrency do not grow active state merely with
elapsed iterations. Shape, live branches, unresolved safety obligations, outputs/artifacts/context,
and configured workspace bounds can still increase it; this is no universal constant-memory claim.

Journal transactions atomically append checksummed events with runtime receipts, workspace/account
changes, and recovery indexes. Separate narrow application ports own external receipts, layouts,
proposal discovery, and bounded security audit. A receipt exists in exactly one hot or cold tier;
oldest-first bounded archival, new receipt insertion, and same-store layout/proposal effects commit
atomically. Cold storage retains the identical canonical request/result for replay and conflict.
If runtime acceptance precedes external receipt commit, recovery replays the stable internal
command and commits the missing receipt; it does not redo the effect.

`RuntimeService::open_closed` checks physical compatibility without admitting work. Startup recovers
active runtime/application owners before adapter registration, peer recovery, worker startup, and
admission. It does not scan all terminal history or hash all artifact bytes. Active corruption fails
startup closed; unrelated terminal corruption is found on read or explicit scrub.

`StorageAdmin::scan_integrity` performs bounded resumable historical verification under one read
transaction per page. Physical phases and cursor policy cover primary records, derived indexes,
receipts/layouts/proposals/audit, artifacts, and controller accounts. Artifact-content verification
is opt-in and bound to the continuation. Health samples are not full integrity proof.

Snapshots have independent persistence envelope and runtime payload versions. Strict padded
standard Base64 carries payload bytes. A domain-separated, length-framed checksum binds metadata
and decoded payload, and the selected append transaction records an exact payload commitment in
the history chain. Envelope checksum, event-prefix digest, and append-time commitment must agree
before a snapshot is verified. Lexical bounds precede tree allocation. Unsupported or invalid
optional snapshots are discarded for authoritative journal replay; no lifetime ID set belongs in
a checkpoint.

## Controller resource accounting

The control lifecycle parses immutable digest-bound controller policy in an ordinary pinned repeat
wrapper. Policy binds identity, wrapper/body/interface, all cumulative ceilings, checkpoints,
fail-closed unknown-usage behavior, stop behavior, currency, labels, and provenance. Unknown policy,
digest mismatch, contradictory wrapper, zero limits, or legacy metadata-only patterns fail. A
controller cannot modify its own policy; another authorized actor needs a new reconciled revision.

Persistence owns account declaration/state, reservations, optimistic revision digest, and immutable
run binding. Its six ceilings are cost, input units, output units, artifact bytes, process entries,
and model entries. Redb changes these only in existing journal/artifact transactions. Descendants inherit
the exact account; nested policies in an already bound descendant are refused. Final entry
revalidates authority, prepares the exact generation/permit and request-specific admission envelope,
then atomically commits entry intent and account admission before adapter code. Frozen descriptor
category determines process/model entry counts.

Unit, cost, and artifact obligations reserve before entry. Unknown bounds, currency mismatch,
blocked state, overflow, or ceiling violation produces durable denial. Terminal evidence settles
only authoritative use; missing bounded observations retain remainders and block future admission.
Uncertainty retains every remainder; over-envelope use blocks the account. Logical artifact
publication charges its exact reservation or run binding in the metadata transaction. Immutable
account revisions replay each change from its predecessor and exact transition/publication source.

Lifecycle assessment consumes settled totals plus outstanding reservations; it separately owns
cycles, revisions, proposals, elapsed time, failures, rejections, and depth. Repeat's structural
guard is one iteration higher so assessment records the reached ceiling before excess child
creation. Proposal dimensions are checked before revision storage; approval/application reassess
cumulative policy. Checkpoints use ordinary durable repeat-continuation decisions with revocation
and limit rechecks. Reached bounds record dimension/current/limit or unknown usage and fail without
provider retry. [Status](product/status.md) owns the production activation gate and evidence.

## Adapter trust and peer durability

Process profiles bind digest/size, optional deployment revision, safe path digests, platform/file
observations, full profile and execution-policy digests, trust class, and ownership facts. Bounded
registration verification precedes descriptor creation. Health and immediately-pre-spawn checks
re-resolve roots, identity, metadata, and bytes; mismatch makes that generation sticky-unavailable
until explicit new registration. Safe Rust leaves a minimized verification-to-entry race; no
atomic open-handle execution guarantee is claimed.

Direct argv substitutions remain single OS arguments. Children begin with cleared environment and
receive only allowlisted names/resolved secrets. Secret-bearing profiles cannot stream text.
Canonical roots, isolated materialization, bounded regular files, traversal/symlink/hardlink
refusal, and declared output imports mediate access. `TrustedHostProcess` still has daemon-account
privileges; `SandboxedProcess` requires a distinct enforcing adapter. On Unix, immediate-child exit
with a live owned group initiates bounded group teardown even if descendants retain output pipes.
[Process operations](guides/local-process.md) owns platform and profile details.

Model adapters reject unadvertised roles, parts, tools, schemas, reasoning, streaming, sessions, and
oversized encoded requests before entry. Output/tool calls remain artifacts, not automatic tool
execution. The two provider mappings preserve their own response/stream semantics and truthful
usage, cancellation, idempotency, and side-effect limits.

Peers consume expiring authority-filtered catalogs. Exact remote identities/generations map to
collision-resistant local capability identities with typed peer, locality, trust, and catalog
provenance. The origin owns workflow truth; the serving daemon owns durable remote acceptance.
Every acceptance/query/page/cancel acknowledgement binds the caller's exact request or URL identity.

Serving acceptance atomically binds canonical idempotency, relationship/catalog/generation,
authority, capacity, and dispatch availability before responding. Fixed workers claim durable
leases and record entry separately; known-entered work is never automatically replaced. Identical
requests replay from hot records or tombstones; changed requests conflict for the store generation.
Hot observations are contiguous bounded rows with a rolling digest. Eligible terminal/uncertain
records archive oldest-first atomically, retaining identity, provenance, authority, cancellation,
accounting, observation count/digest, and final disposition; detailed observations and peer links
retire while core artifacts keep independent retention.

Connection closure is neither cancellation nor terminal proof. Post-entry missing restart evidence
becomes uncertainty. Pre-entry cancellation preserves acknowledgement-completion recovery even
when terminal observation committed just before clock/storage failure. Inbound/outbound artifacts
use core publication/read ports. [Peer protocol](reference/peer-protocol.md) owns wire/retention
details; [peer operations](operations/peers.md) owns operator connectivity and quotas.

## Daemon lifecycle and compatibility

The [daemon](../apps/daemon/README.md) connects these owners into one process. Startup establishes
what can safely continue before accepting new work; shutdown keeps storage available until workers
have finished their final reports. Operators arrange configuration, credentials, external services,
and backup through [daemon operations](operations/daemon.md).

Strict bounded duplicate/unknown-field-rejecting TOML compiles once into normalized immutable owner
plans. Enabled peer mode requires identity; internal owners receive only their section. One bounded
synchronous owner queue serializes runtime, control, storage, clock, and registry operations.
Typed one-shot closures capture response channels; HTTP owns sockets/framing, not semantic state.
Queue saturation has a typed overload response. External work runs on fixed workers without a
global lock. Weak peer-facing store handles prevent router lifetime from extending storage ownership.

One fallible daemon clock advances a durable high-water fact before authorization, expiry,
scheduling, or timestamps; artifact acceptance advances it in the same transaction. Failure or
rollback refuses work and records redacted health evidence. Restart cannot forget a later observed
instant. Elapsed time during downtime still trusts the OS clock. Post-entry peer clock/store loss
retries the exact release/uncertainty transition without another adapter invocation.
Fresh samples are taken only after acquiring the redb write transaction, so concurrent artifact or
receipt publication cannot overtake an earlier sample and manufacture a rollback. The configured
store clock supplies all fresh durable observations; caller-owned timestamps retain exact
watermark validation through the persistence port.

One mutex owns coherent operational health and feed generation. Queue guards release occupancy on
dequeue/drop. Owner-request panic closes ordinary admission; queued requests recheck readiness.
The owner remains available for final clock/persistence/shutdown work, skips normal maintenance,
and cannot claim clean shutdown. These are local admission facts, never workflow history.

Shutdown closes external admission, drains peers/runtime, disconnects registries, and applies the
configured drain/cancel/retain deadline. The owner continues servicing final worker writes while
peer/effect workers join, then joins itself and releases storage. Unresolved identities remain
truthful. [Daemon operations](operations/daemon.md) owns exact startup, backup, and shutdown procedure.

Writers emit one current canonical version per family. Readers accept only versions whose meaning
is unambiguous; unknown core variants, malformed identities, invalid derived fields, duplicates,
and unsupported versions fail. Bounded DNS-namespaced extensions are the explicit unknown-field
mechanism. Semantic/canonical-byte changes require version review and hand-reviewed fixtures;
moving code does not. Current read behavior is tabulated in [status](product/status.md).

Storage physical/internal versions are exact-current with no migration. Legacy sidecars cannot
be ignored or imported as competing authority. Layout has an independent digest and optimistic
generation keyed by workflow/revision; it contains no semantic edges, tasks, prompts, secrets, or
requirements and changes neither revision identity nor runtime history. Rust source compatibility
follows current consumers under the public API policy; durable and wire compatibility remain
independently owned contracts.
