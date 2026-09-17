# 0038 — Independent hosts share execution without sharing workflow history

- Status: accepted direction; implementation assigned to adaptive-hosts 01
- Date: 2026-09-18
- Extends: [0018](0018-peer-idempotency-and-uncertainty.md), [0020](0020-one-authorized-control-and-read-plane.md), [0024](0024-peer-execution-hot-retention-and-tombstones.md), [0037](0037-constrained-peer-placement.md)
- Revises: mandatory runtime composition in [0015](0015-single-daemon-runtime-owner.md), run-only materialization assumptions in [0010](0010-host-owned-materialization.md)

## Context

An operator should be able to call an installed model or process without inventing a workflow.
Current normal daemon startup constructs runtime/control services. Adapter execution context requires
run/revision/node/execution/attempt coordinates, and model preparation loads a workflow manifest.
Serving peer output already has an external producer, but its accounting key is represented as a
`RunId`. Peer workers record entry before calling `execute_exact_with_context`, whereas local runtime
work prepares before committing its entry/account decision. Exposing that helper as a direct API
would leave preparation, authority, accounting and recovery obligations with the endpoint.

## Decision

One daemon executable has execution-only and workflow-enabled compositions. Common owners supply
authentication, authority, clock, storage, artifacts, live generations, durable serving work, and
resource management when configured. Only the workflow role constructs runtime/control and workflow
workers. A compile-time dependency is not evidence that a service must be instantiated. Removing
the role refuses while live or unresolved workflow obligations exist. Closed history stays intact;
execution-only routes report absent role explicitly, and offline inspection remains available without
starting a scheduler. A workflow-enabled owner supplies authorized online workflow history.

Invocation origin is a validated direct request or a workflow delegation with its real coordinates.
Transport is independent: HTTP clients and peers can carry either permitted origin. Acceptance keys
bind store/host identity, authenticated caller realm and principal, and the caller's exact request
key. Canonical bytes include selected generation, origin, inputs, grants/delegation, limits and target.
Different principals cannot collide, and changed bytes under one key conflict after archival too.

Authentication creates the origin/delegation basis. A delegated worker credential is bound to its
parent acceptance, authority, account and input scope, and is unusable as an independent credential.
The host derives those facts rather than accepting an origin switch in JSON. Peers authenticate the
issuer and validate a targeted, bounded delegation; a peer name alone grants no caller authority.
Truly direct credentials admit only explicit authorized inputs and host/per-call limits.

| Case | Acceptance and history | Preparation and final entry | Artifacts, cancellation and restart |
| --- | --- | --- | --- |
| Direct model/process call | Generalized serving execution owner persists exact acceptance before reply. No run exists. | Host acquires the exact generation and prepares bounded data; serving owner rechecks authority, cancellation and allowances and commits entry intent before consuming the prepared handle. | Core artifacts name the host invocation and charge its allowance. Serving owner records observations and cancellation; pre-entry recovery revalidates, possible entry without terminal proof stays uncertain. |
| Local workflow attempt | Runtime journal owns scheduling, attempt and accepted facts. No second serving record for the same attempt. | Same host preparation; runtime retains its atomic final-entry/account transaction and ticket checks. | Core artifacts retain local workflow causes and account reservation. Runtime owns cancellation and recovery; host routes only to the selected adapter generation. |
| Workflow request served remotely | Origin runtime owns workflow history; serving host durably accepts the delegated operation, with an exact link to origin. | Serving owner uses the same preparation and entry mechanism, enforcing the accepted delegation in addition to its own policy. | Serving core artifacts name the serving invocation and retain external causal commitments. Authorized transfer imports results at origin. Both owners retain their own cancellation and recovery facts; neither edits the other's history. |

Acquisition, preparation, final checks and consumption form one mechanism in capability-host.
Preparation performs authorized bounded local reads without external effects. The durable owner
rechecks the exact generation/request, current authority, ticket, cancellation, evidence prerequisites
and applicable reservations after preparation. Only a newly committed entry decision permits entry.
Local refusal before intent, possible submission, observed response, durable terminal and lost
reporting remain distinct. Make obsolete production execution shortcuts private or remove them.
Exact replay recovers accepted work, while current read authority still governs disclosure.

Move transport-independent acceptance/claim/report/cancel/recovery orchestration from peer-http into
capability-host; generalize the existing peer execution persistence ports and redb transactions.
Peer HTTP retains authentication/framing, relationship/catalog adaptation and transfers. Runtime
continues to depend on its executor port; the host implements that bridge. Do not move runtime
events into the serving owner or introduce a generic ledger for all local attempts. Separate a
module before adding a crate; independent startup does not require removing every Cargo edge.

Workflow causal selection remains runtime-owned under [0011](0011-causal-context-manifests.md),
[0031](0031-context-enforcement-and-retained-evidence.md) and [0036](0036-explicit-model-continuation.md).
The serving owner freezes a distinct direct selection: supplied inputs and authorized references,
their digests/sizes, policy, omissions and limits, bound to the accepted invocation. It never searches
workflow history implicitly. Adapters consume validated selection and verify bytes. Direct models
initially support fresh requests; unsupported continuation refuses. Context does not become an
optional unchecked payload. The model owner keeps workflow/model context semantics; host-owned
direct selection and materialization use narrow artifact/workspace ports, not fabricated manifests.

Workspace owns typed artifact producer/ownership meaning; persistence owns publication and accounting
ports. Generalize those to local workflow and host invocation ownership, including materialization,
integrity, transfer, retention and authorized inspection. Replace the run-shaped peer accounting
namespace, preserving the fact that no local run was created. A bounded client upload uses the same
artifact store with caller quotas, immutable metadata and cleanup; no arbitrary destination paths.

For remote controlled work, persistence owns an allowance transfer linked to the exact originating
reservation and serving acceptance. Reserve the supported upper bound at origin before dispatch;
the serving host enforces that allowance and returns authenticated settlement evidence. Settlement
converts the same reservation into attributable use once; it is not a second charge. Missing or
unknown use retains reservations or refuses a required bound. Unmediated calls inside an arbitrary
process remain outside direct-model metering. [0041](0041-published-method-invocation.md) extends
this relationship to published internal runs.

## Compatibility and consequences

This decision changes no current writer or supported reader. Assignment 01 must review invocation,
context, producer/publication, peer execution, grants/delegation, account, configuration and public
DTO versions against current constants and fixtures before changing their meaning. Preserve exact
current replay within a supported store generation. Never infer new authority or direct origin
from missing old fields. Unsupported pre-release stores refuse before mutation under the existing
exact-current storage policy; retain original generations for offline preservation/inspection.
There is no designed migration here. Supported historical bytes retain their original meaning;
optional projections may rebuild from supported history only. Upgrade all current producers and
consumers together instead of retaining an unreviewed legacy execution path.

## Alternatives and reconsideration

Synthetic runs would make direct provenance false. A second local attempt journal would compete
with runtime transactions. A hidden runtime behind disabled routes would not establish independent
hosting. Keep one hosting implementation and two justified durable owners instead. Reconsider a
physical package split only when actual dependency consumers require it; reconsider compatibility
only with reviewed old-writer fixtures and an explicit preservation or migration protocol.
