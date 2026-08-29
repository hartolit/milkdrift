# ADR 0020: One authorized command and read plane

- Status: accepted
- Date: 2026-08-29

## Context

Milkdrift already authorized durable run commands and capability entry through immutable grants, but
the daemon also interpreted preset booleans for imports, layouts, artifacts, and administrative
reads. Several reads reached stores or live registries directly, capability catalogs began from a
wildcard, peer relationships used a separate action predicate, and continuation cursors proved only
their feed. Authentication could therefore select an actor while a second policy path decided what
that actor could see or do. Grant narrowing could not reliably invalidate a page or reconnect.

Reads are capabilities: revision names, artifact metadata, provider identities, peer configuration,
queue load, and cursor ordering can all disclose protected facts even when no bytes are mutated.

## Decision

Authentication produces one `ActorSession` containing the actor, exact expanded immutable grant
revision, evaluation context, and a redacted credential-derived cursor key. Presets and peer action
lists are deterministic configuration shorthand for typed operation sets only. They are not session
roles and are never queried at an executable boundary.

One `GrantSetEvaluator` instance serves the daemon owner, control/runtime services, capability host,
and peer-service relationship grants. Each external route declares a typed operation family and
typed resource family. The owner resolves immutable resource facts, evaluates the exact grant, and
only then calls a command/query owner. Collections are constrained or filtered before projection.
Artifact metadata and bytes use separate operations and evaluate the stored immutable sensitivity.
Readiness and detailed health are deliberately different operations and daemon scopes.

Schema-2 grants add explicit artifact, layout, peer, daemon, and workspace scopes plus an explicit
capability deny-all representation. Empty protected scopes deny access. Daemon configuration schema
3 requires those scopes and rejects older schemas rather than inventing access. Operations exist for
currently implemented routes and for adjacent reserved administration contracts; a reserved
operation does not create a route.

Authenticated cursor schema 2 binds feed position to actor, grant identity/revision/digest,
decision digest, resource/filter digest, and a credential-derived keyed MAC. Servers accept only
that form. Open streams reauthenticate and reevaluate on every bounded poll; reconnects also prove
the complete binding. Capability stream caches are partitioned by scope digest.

Consequential runtime/control mutations keep their existing durable decision provenance. The
bounded redb application-audit port additionally records protected artifact releases, blueprint
import, layout mutation, and peer administration. ADR 0022 supersedes the former sidecar storage
detail. Peer accepted-execution records retain the exact allowing decision. Audit records contain
actor, grant identity/revision/digest, operation, resource digest, decision digest, outcome, and
stable reason codes, never credentials or protected payloads.

## External operation inventory

The inventory below is exhaustive for the implemented local HTTP surface. “Durable” means the
authority decision is retained because the operation mutates durable state or releases protected
content; ordinary refreshes are evaluated but not appended to an unbounded audit.

| Route or command | Actor and exact resource | Sensitivity / effect | Authority operation | Durable / continuation rule |
| --- | --- | --- | --- | --- |
| `POST /v1/version` | authenticated local actor; daemon protocol | low-information read | `negotiate_control_protocol` | no cursor |
| `GET /v1/readiness` | local actor; daemon coarse state | coarse read with operational counters removed | `read_readiness` | streamless |
| `GET /v1/health`, `/v1/stream/health` | local actor; daemon detailed state | queue/worker/failure read | `inspect_daemon_health` | stream cursor is actor/grant/daemon-scope bound |
| `import_blueprint`, `validate_blueprint` | local actor; parsed workflow plus exact revision | durable definition mutation / validation | `import_blueprint`, `validate_blueprint` | import decision audited; command replay binds exact grant |
| `start_run` | local actor; exact workflow/revision/run | durable execution mutation | `create_run`, then `start_run` | transactional runtime decisions; command replay binds exact grant |
| `pause_run`, `resume_run`, `cancel_run`, `signal_run` | local actor; exact resolved workflow/run | durable control mutation | `pause`, `resume`, `cancel`, `deliver_signal` | runtime command decision retained |
| `resolve_work` | local actor; exact workflow/run/attempt | durable or inspecting control | action-derived `inspect_attempt`, `retry`, `apply`, `approve`, or `terminate` | runtime command decision retained |
| `submit_proposal`, `decide_proposal`, `apply_proposal` | local actor; exact workflow/run/proposal/revision | protected durable control | `propose`, `approve`, `apply` | control/runtime decision retained |
| `put_layout` | local actor; exact workflow/revision/shared layout | presentation-state mutation | `write_layout` | bounded audit; no semantic revision change |
| `GET /v1/revisions`, `/{revision}`, `/{from}/diff/{to}` | local actor; workflow and exact revision lineage | protected definition read | `inspect_revision` | list is preconstrained; cursor binds workflow/filter/grant |
| `GET /v1/runs`, `/{run}` | local actor; exact workflow/run | protected execution summary | `inspect_run` | list is preconstrained; cursor binds state/workflow/run scope |
| `GET /v1/runs/{run}/nodes/{execution}` | local actor; workflow/run/execution | protected execution provenance | `inspect_node_execution` | no cursor |
| `GET /v1/runs/{run}/attempts/{attempt}` | local actor; workflow/run/attempt | capability/context provenance | `inspect_attempt` | no cursor |
| run timeline and `/stream` | local actor; exact workflow/run | protected historical projection | `inspect_timeline` | page/stream cursors bind actor/grant/run/filter; open stream reevaluates |
| proposal list/status | local actor; exact workflow/run/proposal/revision | protected reconciliation state | `inspect_proposal` | page cursor binds actor/grant/run/filter |
| capability list and `/stream/capabilities` | local actor; exact visible generations/providers | descriptor/provider/health read | `list_capabilities`, `inspect_capability_health`, `inspect_provider_profile` | filter before projection; cache/cursor partitioned by scope digest |
| peer list/show | local actor; each exact configured peer | relationship/catalog/health read | `inspect_peer` | hidden peers are removed before response |
| peer connect/reload/disconnect/drain/revoke | local actor; exact configured peer | administrative mutation | `administer_peer` | bounded audit |
| `GET /v1/authority` | local actor; own exact grant plus daemon flag | security-policy read | `inspect_own_authority` | no other actor/grant is exposed |
| artifact metadata | local actor; exact artifact plus stored sensitivity | protected metadata read | `read_artifact_metadata` | bounded audit for protected release |
| artifact content range | local actor; exact artifact plus stored sensitivity/range | protected byte release | `read_artifact_content` | bounded audit; no public-content bypass |
| layout read | local actor; exact workflow/revision/shared layout | protected presentation read | `read_layout` | independent of semantic mutation scope |

The peer realm is independently authenticated but uses the same authority semantics:

| Peer route | Exact resource | Authority operation | Continuation / provenance |
| --- | --- | --- | --- |
| handshake | configured relationship | `negotiate_peer_session` | current credential and grant evaluated |
| catalog | relationship plus each capability/provider/health fact | `inspect_peer`, `list_capabilities`, `inspect_capability_health`, `inspect_provider_profile` | filtered before digest/count construction |
| invoke | relationship, exact capability/operation/side effect/budget | `invoke_peer_capability` and `inspect_peer_execution` | allowing decision stored with accepted execution |
| request lookup, observation page, observation stream | exact relationship/execution | `inspect_peer_execution` | every page/poll reevaluates; revocation emits authorization termination |
| cancellation | exact relationship/execution | `cancel_peer_capability` | acknowledgement remains separate from outcome |
| artifact negotiate/content/abort | relationship, transfer, direction, metadata | `peer_artifact_upload` or `peer_artifact_download` | transfer scope and quota evaluated before bytes |

Public CLI and control-client methods are transport mappings for these routes and introduce no
additional authority path. The in-process workflow-control capability uses the same
`ControlService` and immutable grant decisions as a human client. Route-registration guard tests
reject new raw local or peer route declarations that omit typed authority/resource mapping.

## Rejected alternatives

- Retaining preset or relationship `may_*` predicates as a fallback, because that recreates a
  second policy owner and makes equal grants behave differently by actor label.
- Authorizing only HTTP handlers, because direct owner/store helpers and in-process control would
  remain bypasses.
- Fetching complete collections and relying on clients to hide entries, because counts, gaps,
  errors, cursors, and caches leak identities.
- Feed-only unsigned cursors or server-side unbounded cursor sessions, because neither gives a
  compact proof of actor/grant/filter continuity.
- Auditing every UI refresh as a durable event, because harmless reads would create unbounded
  execution history unrelated to workflow truth.

## Consequences

Equal grants and requests yield equal deterministic decisions for human and AI actors. Credential
validity no longer implies broad reads. Narrowing, replacement, revocation, or credential rotation
invalidates future pages and reconnects; open streams stop future disclosure without rewriting
already delivered history. Provider, peer, artifact, layout, and detailed-health visibility must be
granted explicitly. Grant/config/cursor/storage version changes are intentionally incompatible and
require deliberate operator migration. ADR 0022 completes persistence convergence without claiming
a general audit API.

## Reconsideration triggers

Add a new external route only with a typed operation/resource declaration and matrix test. Add
server-side continuation sessions only if a demonstrated query cannot be bound compactly. Add
dynamic local grant reload only with atomic grant-generation publication and the same page/stream
invalidation semantics.
