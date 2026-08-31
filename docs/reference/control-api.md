# Local control API 2.1

This document is the implemented external contract for `milkdrift-daemon`. It describes a local control plane, not a peer protocol or public internet service.

## Transport, authentication, and negotiation

The daemon serves HTTP/1 on a configured loopback address. Non-loopback plaintext configuration is rejected and CORS is not enabled. Every route, including health and version negotiation, requires `Authorization: Bearer …`. The credential is resolved from a configured file or exact environment-variable reference and maps to a server-owned actor and exact immutable grant revision; request JSON never supplies actor identity. Authentication alone grants no operation. Every route declares a typed authority operation and resource family, and the daemon's owner evaluates it before returning information or mutating state.

Clients negotiate with `POST /v1/version`:

```json
{"protocol":{"major":2,"minor":1}}
```

Major 2 is required. Protocol 1 is deliberately unsupported because attempt inspection now carries
the complete frozen authority basis and execution-boundary decisions. The current minor is 1 and
adds redacted application-receipt lifecycle health. The
authenticated `/v1/...` HTTP route namespace is stable and independent from the negotiated envelope
version. JSON success bodies use:

```json
{
  "protocol": {"major": 2, "minor": 1},
  "request_id": "req-1",
  "value": {}
}
```

The caller may provide one bounded ASCII `x-request-id`; otherwise the daemon creates one. Inputs are byte/depth/count bounded, duplicate object keys are rejected, and unknown fields are rejected for closed DTOs.

## Errors

Errors are configuration-independent and never contain tokens, headers, environment values, prompts, process/model output, artifact bytes, provider objects, or filesystem paths:

```json
{
  "protocol": {"major": 2, "minor": 1},
  "request_id": "req-1",
  "code": "conflict",
  "message": "bounded redacted description",
  "retryable": false,
  "details": {"actual_sequence": "12"}
}
```

Stable codes are `unauthenticated`, `unauthorized`, `invalid_input`, `conflict`, `not_found`, `overload`, `unavailable`, `corruption`, `uncertain`, `unsupported_version`, `timeout`, and `internal`. Principal HTTP mappings are 401, 403, 400, 409, 404, 429, 503, 500, 409, 426, 504, and 500 respectively. Bounds failures may use 413. Retryability is explicit; conflict and authorization failures are not made retryable.

## Commands

All mutations use `POST /v1/commands`. A command envelope has no actor field:

```json
{
  "protocol": {"major": 2, "minor": 1},
  "command_id": "operator-stable-id",
  "expected_sequence": null,
  "expected_revision": null,
  "reason": "bounded operator reason",
  "evidence": [{"id":"artifact-or-receipt-id","kind":"artifact"}],
  "command": {"type":"pause_run","run_id":"run-1"}
}
```

`command_id` is an actor-and-grant-scoped idempotency key. The daemon computes a canonical digest over the protocol/command schema, complete command envelope, authenticated actor, and exact grant identity/revision/digest before application. Repeating that exact request returns the durably stored accepted result with `replayed: true`, or the exact stored deterministic rejection; reusing the identity with different content or after grant replacement returns `conflict`. Receipts survive daemon restart and retain exact replay for the complete store generation. A configured hot bound limits recent operational placement only; bounded oldest-first archival moves immutable documents to transparent cold storage, and new-command commit can reclaim capacity transactionally. Mutating requests are not implicitly retried by `milkdrift-control-client`; a caller retry must preserve the exact body, idempotency identity, and authority basis.

For layout writes and proposal discovery, the receipt and same-store application effect commit in one redb transaction. Runtime/control effects retain their existing idempotent transaction as authority. If the daemon crashes after such an effect commits but before its application receipt, redelivery uses the same stable internal command identity, observes runtime replay, and commits the missing external receipt without applying replacement work. Transient storage, overload, unavailable, timeout, uncertain, corruption, and internal failures are not converted into durable rejections.

The closed command types are:

| Type | Required body fields | Authority operation | Purpose |
| --- | --- | --- | --- |
| `import_blueprint` | `document` | `import_blueprint` | Validate and store an exact immutable workflow/revision. |
| `validate_blueprint` | `document` | `validate_blueprint` | Validate one exact workflow/revision without storing it. |
| `import_prompt_sequence` | `document` | `import_blueprint` | Compile bounded schema-2 JSON/Markdown-derived data and store the ordinary immutable revision. |
| `validate_prompt_sequence` | `document` | `validate_blueprint` | Compile and validate a prompt sequence without storing its generated revision. |
| `start_run` | `run_id`, `workflow_id`, `revision_id` | `create_run`, then `start_run` | Atomically create then start at an exact revision through ordinary control authority. |
| `pause_run`, `resume_run`, `cancel_run` | `run_id` | `pause`, `resume`, `cancel` | Durable exact-run lifecycle control. |
| `signal_run` | `run_id`, `signal_id`, `signal_type`, `correlation`, `broadcast`, `payload` | `deliver_signal` | Deliver a typed bounded signal to an exact run. |
| `resolve_work` | `run_id`, `attempt_id`, `decision_id`, `action`, `remediation_node` | action-derived `inspect_attempt`, `retry`, `apply`, `approve`, or `terminate` | Query, retry, compensate, retain, or evidence-resolve uncertain work. |
| `submit_proposal` | `document` | `propose` | Submit an exact schema-1 workflow proposal through `milkdrift-control`. |
| `decide_proposal` | `run_id`, `proposal_id`, `proposal_digest`, `proposed_revision`, `decision_id`, `decision` | `approve` | Approve or reject an exact proposal. |
| `apply_proposal` | `run_id`, `proposal_id`, `proposal_digest`, `proposed_revision` | `apply` | Apply an approved prospective revision through reconciliation. |
| `put_layout` | `layout` | `write_layout` | Optimistically store presentation-only state for the exact workflow/revision/shared owner. |

Evidence kinds accepted by the daemon are `authority_decision`, `worker_observation`, `external_receipt`, `artifact`, and `recovery_observation`. A success returns `CommandAccepted`: `command_id`, `replayed`, optional `resulting_sequence`, stable `result_type`, and a bounded command-specific `value`.

The wire command carries the decoded prompt-sequence JSON document. Markdown parsing is owned by
the CLI/library before submission, and the daemon independently performs strict schema validation
and ordinary blueprint compilation. Validate/import responses include schema/sequence/workflow,
revision and semantic identity, import and repository-profile digests, and ordered stage-node
summaries. The full schema is documented in [`prompt-sequence-v2.md`](prompt-sequence-v2.md).

## Query routes

Every route is authenticated and authority-filtered. List queries constrain or filter at the owner boundary before projection, so hidden identities do not appear in counts, gaps, or result payloads. A missing object is returned only after the caller is authorized for its supplied scope; an authorization failure remains the bounded `unauthorized` error.

| Method and path | Result |
| --- | --- |
| `POST /v1/version` | Protocol negotiation under `negotiate_control_protocol`. |
| `GET /v1/health` | Detailed lifecycle, queue, worker, failure, and redacted hot/cold receipt archival health under `inspect_daemon_health` plus daemon detailed-health scope. |
| `GET /v1/readiness` | Coarse liveness/readiness with zeroed operational detail under `read_readiness`; returns 503 while not ready. |
| `GET /v1/revisions?limit=&cursor=&workflow=` | Stable bounded revision-summary page. |
| `GET /v1/revisions/{revision}` | Immutable revision summary, lineage, provenance, counts, and bounded document. |
| `GET /v1/revisions/{from}/diff/{to}` | Bounded structured semantic diff with an explicit truncation flag. |
| `GET /v1/runs?limit=&cursor=&state=&workflow=` | Stable bounded compact-run page. |
| `GET /v1/runs/{run}` | Compact current run status and retained execution frontier. |
| `GET /v1/runs/{run}/nodes/{execution}` | One retained node execution. |
| `GET /v1/runs/{run}/attempts/{attempt}` | One exact current or historical attempt; journal paging supplies older attempts without retaining lifetime history in the compact run model. Includes capability/provider/peer linkage, frozen snapshot/trust/implementation provenance, and separately authorized context-manifest detail when present. |
| `GET /v1/runs/{run}/timeline?limit=&cursor=` | Paged external timeline projection with exact durable sequence anchors. |
| `GET /v1/runs/{run}/proposals?limit=&cursor=` | Bounded proposal identities/statuses from the durable validated proposal projection; exact status remains owned by `milkdrift-control`. |
| `GET /v1/runs/{run}/proposals/{proposal}?revision={revision}` | Exact status from `milkdrift-control`. |
| `GET /v1/capabilities` | Only generations within capability scope, with descriptor category/operations/locality/peer/trust, provider profile where allowed, and scoped health/availability. |
| `GET /v1/peers` | Only configured peer identities within `inspect_peer` scope. |
| `GET /v1/peers/{peer}` | One authorized configured peer status. |
| `POST /v1/peers/{peer}/{connect|reload|disconnect|drain|revoke}` | Exact peer administration under `administer_peer`; the action is audit-recorded. |
| `GET /v1/authority` | Current server-owned actor, grant, revision, revocation generation, and configured operation labels. |
| `GET /v1/artifacts/{artifact}` | Safe digest, size, media type, disposition name, and sensitivity; never a server path. |
| `GET /v1/artifacts/{artifact}/content` | One verified explicit byte range under artifact-read authority. |
| `GET /v1/layouts/{workflow}/{revision}` | Exact independent layout document. |

`limit` defaults to 100 and must be within the protocol page bound. A page contains `items`, optional `next_cursor`, and optional feed-head `observed_cursor`. Clients must request subsequent pages explicitly; the client library never auto-loads a complete run lifetime.

Artifact metadata and content are separately authorized against the exact immutable artifact identity and stored sensitivity before either is disclosed. Content accepts one `Range: bytes=start-end` request and returns 206 with `Content-Type`, `Accept-Ranges: bytes`, `Content-Range`, safe `Content-Disposition: attachment`, and `x-milkdrift-artifact-complete`. A server call returns at most 1 MiB. There is no arbitrary path access or public upload endpoint. Protected metadata/content decisions are retained in the bounded security audit with actor, grant revision/digest, operation, resource digest, decision digest, outcome, and reason codes; raw credentials and content are absent.

## Read models

External timeline entries use the stable categories `lifecycle`, `execution`, `progress`, `artifact`, `coordination`, `authority`, `recovery`, `reconciliation`, and `uncertainty`. An entry carries the exact durable sequence, timestamp, bounded actor and run/node/attempt/revision references, stable summary, and bounded structured detail. It is deliberately not an internal `RunEventKind` document.

Run models carry aggregate sequence, stable lifecycle, optional terminal outcome, workflow/revision/digest, a compact retained node frontier, and unresolved-uncertainty count. Node models retain the latest attempt for compact status, while the exact-attempt route pages authoritative history for an older identity. Attempt models carry immutable attempt state, exact capability/descriptor/provider/peer linkage, optional context-manifest artifact metadata, terminal summary, and uncertainty. `capability_provenance` carries the exact frozen snapshot digest and execution trust class. For a byte-pinned local process it also carries the safe implementation identity, executable content digest and size, complete profile digest, execution-policy digest, and optional package/documentation references; executable paths are never returned.

When a manifest exists, the daemon separately evaluates `read_artifact_content` for that exact restricted artifact. An allowed read verifies its schema, digest, size, and attempt binding, then returns a bounded context object containing the immutable task policy, selected causal/provenance metadata, stable omissions, totals, applied budget, and a truncation flag. A denial sets `context_access` to `denied` and returns neither policy nor entry/omission detail; `metadata_only` means only the compact manifest reference was disclosed. Artifact bytes remain available only through the separately authorized bounded range route. Complete lifetime history remains the paged journal-backed timeline.

## Cursors and SSE

Cursors are opaque bounded Base64url schema-2 values. They bind an exact feed and position/key to the authenticated actor, grant identity/revision/digest, authority decision, and a domain-separated digest of the complete resource/filter scope. A credential-derived keyed MAC prevents modification or reuse after credential rotation. A malformed, stale, cross-actor, cross-grant, cross-resource, or cross-filter cursor fails as bounded `invalid_input`; a broader replacement grant does not reinterpret an old continuation. Clients must store only a successfully observed cursor and resume after it.

The daemon exposes:

- `GET /v1/runs/{run}/stream?cursor=…` for projected timeline and compact status updates on feed `run:{run}`.
- `GET /v1/stream/capabilities?cursor=…` for a bounded retained window of capability generation/health snapshots on feed `capability-health`.
- `GET /v1/stream/health?cursor=…` for coarse daemon health on feed `daemon-health`.

SSE `data` values are `ObservationEnvelope` documents with protocol, cursor, observation time, feed, and one closed external observation: `timeline`, `run_status`, `capability`, `daemon_health`, `stream_closing`, or `resync_required`. Capability feed snapshots and retention windows are partitioned by authority-scope digest, so hidden generations cannot affect another actor's counts, ordering, or cursors. The feed retains the latest 256 observations per scope; an older continuation receives `resync_required`.

Run-feed positions interleave durable timeline sequence (`2 × sequence`) and its following compact status (`2 × sequence + 1`). Transport heartbeats are SSE comments and are never durable events. Server generators and owner calls are bounded; backpressure retains no unbounded per-client event queue. Authentication and exact authority are reevaluated on every bounded polling cycle. Rotation, revocation, narrowing, draining, invalid history, or authorization change stops future disclosure and closes the feed with an authorization closing/resync item where possible; already delivered history is not rewritten.

`milkdrift-control-client::subscribe` reconnects retryable transport failures with its last successfully decoded cursor. Reconnect never submits or replays a command.

## Layout schema 1

`LayoutDocument` contains `schema_version`, exact `workflow_id` and `revision_id`, positive optimistic `generation`, server-overwritten `author`, independent BLAKE3 `digest`, bounded `nodes` positions/dimensions, `collapsed_groups`, non-executable `annotations`, and optional `viewport`. The first generation is 1; a changed document must advance by exactly one. The daemon recomputes author and digest before storing.

Layout cannot contain executable edges, node/task configuration, requirements, prompts, secrets, or semantic mutations. Each exact workflow/revision layout is stored as an independently checked schema-1 application record in redb, independently from the immutable blueprint revision. Layout edits therefore do not create a revision, run event, or semantic digest change. Concurrent changed writes must advance from the current generation by exactly one; stale writes are durable deterministic conflicts under the external command receipt policy.

## CLI automation contract

`milkdrift-cli --json` prints one compact line per result:

```json
{"schema_version":1,"type":"run.show","value":{}}
```

JSON mode has no colors or terminal control sequences. Exit categories are 0 success, 2 invalid input/confirmation, 3 authentication or authority, 4 conflict, 5 retryable unavailable/overload/transport, 6 not found, 7 internal/nonclassified failure, and 8 when `run show` observes a failed terminal task. Cancel and proposal decision/apply operations require interactive `yes` or `--yes`; JSON-mode high-risk operations require `--yes`. Credentials come from `--token-file`/`MILKDRIFT_TOKEN_FILE` or the configured environment-variable name, never a token command argument. Artifact downloads require an explicit new destination and remove a partial file after failure.
