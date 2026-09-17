# 01 — Independently usable execution hosting

## Assignment and dependency

Complete independent hosting end to end after 00 is accepted. Read `handoffs/00.md`,
[the sprint README](README.md), [the discussion](discussed-direction.md), and
[AGENTS.md](../../../../AGENTS.md) in its required order. Apply
[implementation practice](../../practices/implementation.md),
[documentation practice](../../practices/documentation.md), and the
[verification policy](../../workflow.md). You own the entire host-execution responsibility and every
current consumer affected by its generalization.

[ADR 0038](../../../decisions/0038-independent-host-execution.md) is the adopted owner decision:
move serving lifecycle semantics out of peer-http into capability-host, generalize existing
persistence, retain runtime's local transaction, and represent direct selection and artifact
ownership honestly. Review actual schemas/fixtures before assigning versions; no migration is
preapproved by 00. ADRs 0039–0041 describe later consumers, not unused interfaces to scaffold now.

A direct client must invoke useful process and model operations on an execution-only host. A desktop
workflow must invoke those same operations remotely. Neither request requires a workflow runtime on
the serving host. Existing local workflow execution retains its guarantees and history. A hidden
runtime behind disabled UI routes, synthetic workflow identities, or a no-op-only demonstration does
not meet the assignment.

## Trace before editing

At the planning baseline, begin with `apps/daemon/src/host/startup.rs`, daemon configuration/owner/
shutdown and authority routing; `crates/capability-host/src/adapter.rs`, `registry/execution.rs`,
`materialization.rs`, and workers; `crates/peer-protocol/src/execution.rs`; `adapters/peer-http/src/service/`
and its remote adapter; `crates/persistence/src/peer.rs`; redb peer/artifact/application owners;
`crates/model` context contracts; process/model/control adapters; and control protocol/client/CLI.
Trace actual callers and tests throughout the checkout. Use the architecture, current model guide,
peer protocol, and ADRs on authority, context, artifacts, and peer retention to preserve their meaning.

Search all constructors and consumers of workflow-shaped adapter context, peer provenance,
`ExecutionAuthorityBasis`, direct execution helpers, producer identity, materialization directories,
controller reservations, and command receipts. Do not stop at the peer wire struct: output storage,
inspection, cleanup, continuation, and recovery are part of this change.

## Required implementation

### 1. Real startup roles

Compile strict configuration into execution-only or workflow-enabled composition. Establish the
common store, clock, authority, registry, adapter lifecycle, serving work, and authenticated API once.
Construct workflow runtime, control service, and workflow workers only in the workflow-enabled role.
Expose truthful role/capability availability to clients. Workflow-control operations and adapters
cannot be advertised as usable when their owner does not exist.

Refuse role removal over active or unresolved workflow obligations; preserve closed history and
provide an appropriate inspection path. Recovery must not silently reinterpret old records, delete
state, or start missing roles. Startup admission opens only after relevant execution/artifact owners
recover. Shutdown retains storage until reporting and owned workers are resolved under the existing
deadline policy. Do not use a daemon per resource or split the product into competing executors.

### 2. Honest invocation identity and authority

Represent direct and workflow-originated invocation as explicit validated meanings. Keep transport
and origin separate. Scope request replay to authenticated caller and host; namespace peer-supplied
identifiers to prevent collisions. Retain exact workflow coordinates when they exist without
requiring them for truly direct work. Direct clients use client authentication, not fabricated peers.

Use the shared evaluator for discovery, input reading, invocation, result inspection, cancellation,
and all protected metadata. Resolve immutable resource facts below transport handlers. Do not trust
caller-supplied actor, origin, grant, or producer claims. A delegated worker cannot choose direct
origin to bypass its parent restrictions, branch visibility, or applicable account. Keep exact
accepted authority evidence while checking current permission before new entry and disclosure.

Preserve local controlled workflow reservations. For remotely served controlled operations, bind
an enforceable per-call allowance to the originating reservation and accepted request; retain unknown
usage and outstanding obligations on response loss. Do not label a separate remote observation as
another independent charge or release an allowance merely because the client timed out. Truly
independent requests have explicit per-call and host-wide bounds, not invented controller runs.
Nested workflow-backed service accounting is added in 04; ordinary remote process/model accounting
must work now. Unmetered arbitrary agent-internal provider calls remain explicitly unqualified.

### 3. One prepared execution mechanism with correct durable owners

Generalize acquire/prepare/enter/report/cancel mechanics for direct, peer, and workflow callers.
Preparation must not perform the external effect. It validates inputs and captures the exact request
under the selected generation. After preparation, the owning service revalidates current authority,
lease/cancellation state, limits, and exact selection before committing entry intent and reservations.
Only then may the prepared adapter enter. No endpoint may call a permissive lower-level execute helper
as a shortcut around these obligations. Remove or make inaccessible superseded production bypasses.

Keep the runtime journal and its atomic final-entry/account transition authoritative for local
workflow attempts. Generalize serving-side execution persistence for standalone and incoming remote
work instead of creating a second journal for every local task. Move semantics out of HTTP-specific
code where necessary; keep transport concerns in adapters. Reuse genuine shared logic without
merging transaction owners that enforce different facts.

Persist acceptance before acknowledgement, distinguish claim from possible entry, and retain one
terminal fact or truthful uncertainty. Exact replay after restart or archival returns the same
accepted work; same key/different request conflicts. Cancellation acknowledgement remains distinct
from completion. Pre-entry recovery revalidates; post-entry uncertainty never authorizes blind
re-execution. Bound queues, workers, pages, observations, input/output bytes, and cleanup waits.

### 4. Complete direct input and artifact support

Retain workflow causal manifests, required evidence, omission redaction, scope isolation, exact
attempt binding, and existing continuation. Add an explicit direct-selection meaning: only supplied
or referenced inputs, authorized and frozen for the accepted invocation. Do not search workflow
history or arbitrary filesystem state implicitly. Bind input digests, sizes, policy, limits, and
origin to the prepared request and retained selection.

Support fresh direct model requests and useful direct process inputs/outputs. Refuse unsupported
direct continuation honestly rather than downgrading it. Adapters verify bytes against validated
selection; missing/mismatched provenance, forbidden references, or excessive input refuses before
the forbidden read or effect. Do not make the manifest nullable to make current checks disappear.

Generalize artifact producer/owner, publication accounting, materialization naming, retention,
transfers, authorized reads, and integrity scans. Remove fake run namespaces as the semantic owner
of direct/peer output. Use one ordinary artifact store; caller references cannot create producer
truth. Add the bounded public input-upload/import path needed for a direct client to supply actual
files/model requests through product APIs, with atomic publication, cancellation/cleanup, integrity,
sensitivity, and per-caller quotas. Do not require the client to write the daemon database or a
private fixture directory. Never accept an arbitrary host path as an authorized upload destination.

### 5. Public operation and complete adoption

Provide typed client/API/CLI submission, capability discovery, request lookup, invocation inspection,
bounded follow/wait, artifact upload/download, and cancellation. Both synchronous acceptance and
subsequent outcome must be understandable in machine output and operator text. Include explicit
endpoint/host selection for multiple owners without a new discovery system or implicit credential
sharing. Non-loopback connectivity must use the supported authenticated secure transport; do not
relax current HTTPS rules for the two-host demonstration.

Adopt changed contracts in all process/model/peer/control consumers, validators, recovery paths,
fixtures, examples, docs, and production construction. Decide public/wire/durable version changes
under 00's compatibility decisions. Unsupported old stores fail before mutation; supported records
remain exact. A cross-cutting crate/module correction is in scope when required for this ownership.
Unused generic frameworks, compatibility aliases, and two equivalent entry paths are not.

## Evidence required now

Exercise production process/model adapters with deterministic external fixtures and actual daemon/CLI
binaries. At least the following cases must have observable assertions:

| Case | Required observation |
| --- | --- |
| Direct process and fresh model call on execution-only host | Useful output/artifacts, honest direct provenance, no workflow service construction or fake run records. |
| Desktop workflow invokes the same serving capabilities | Same serving acceptance/preparation/reporting path with real origin linkage; local runtime still owns its workflow facts. |
| Local workflow after refactor | Existing causal/continuation, controller admission, cancellation, and recovery behavior remains valid; no duplicate local execution ledger. |
| Lost acceptance reply, repeated request, restart, archival | One external entry for exact replay; changed request conflicts; protected results still require current read permission. |
| Wrong origin, colliding identifiers, cross-caller request lookup | Refusal or independent scoped identity; no authority confusion or data disclosure. |
| Missing/oversized/forbidden inputs and altered artifact bytes | Refusal before forbidden reading/entry; failed uploads leave no visible partial artifact. |
| Revocation during preparation, cancelled claim, exhausted allowance | No external entry or unjustified reservation release. |
| Crash before entry and after possible entry | Safe revalidation only for pre-entry; otherwise retained uncertainty, including failures to publish terminal evidence. |
| Unsupported role/old store and role change with obligations | Actionable refusal; unchanged protected historical bytes. |
| Bounded capacity, panic, and shutdown | No unowned threads/processes, lost permits, unsafe fallback, or uncapped buffers. |

Use external request counters/markers and reopen stores with complete object teardown. A mocked
“success” return does not prove no duplicate entry or actual startup composition. Preserve existing
peer service storage/lifecycle/fault suites, model mock-endpoint context/accounting tests, process
conformance, daemon two-host tests, and repository contracts; extend the shared conformance suite
where the adapter contract changes. Run the full gate in the sprint README.

Publish maintained direct-call and execution-only examples through production readers. Measure idle
threads/memory for both roles as evidence with environment details, not as a universal performance
promise. Live model/physical UM790 qualification is completed in 06; supported real endpoint usage
must already be possible through the same implementation.

## Completion and handoff

The stop condition is a complete independently useful host with process and fresh model operations,
client/API/CLI access, correct shared prepared execution, honest artifacts, and tested refusal/recovery.
Do not stop after schema edits or move incomplete adapter support to 02. Fix deeper in-scope defects
and remove superseded paths before handoff.

Write `handoffs/01.md`: current behavior, actual owner/module moves, deleted bypasses, exact versions
and read policies, public commands, test/evidence locations, and any genuinely unexecuted platform
qualification. Update status and owning docs without claiming the future managed setup exists.
