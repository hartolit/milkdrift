# Local control API 1.0

This document is the implemented external contract for `milkdrift-daemon`. It describes a local control plane, not a peer protocol or public internet service.

## Transport, authentication, and negotiation

The daemon serves HTTP/1 on a configured loopback address. Non-loopback plaintext configuration is rejected and CORS is not enabled. Every route, including health and version negotiation, requires `Authorization: Bearer …`. The credential is resolved from a configured file or exact environment-variable reference and maps to a server-owned actor and immutable grant facts; request JSON never supplies actor identity.

Clients negotiate with `POST /v1/version`:

```json
{"protocol":{"major":1,"minor":0}}
```

Major 1 is required. The current minor is 0. JSON success bodies use:

```json
{
  "protocol": {"major": 1, "minor": 0},
  "request_id": "req-1",
  "value": {}
}
```

The caller may provide one bounded ASCII `x-request-id`; otherwise the daemon creates one. Inputs are byte/depth/count bounded, duplicate object keys are rejected, and unknown fields are rejected for closed DTOs.

## Errors

Errors are configuration-independent and never contain tokens, headers, environment values, prompts, process/model output, artifact bytes, provider objects, or filesystem paths:

```json
{
  "protocol": {"major": 1, "minor": 0},
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
  "protocol": {"major": 1, "minor": 0},
  "command_id": "operator-stable-id",
  "expected_sequence": null,
  "expected_revision": null,
  "reason": "bounded operator reason",
  "evidence": [{"id":"artifact-or-receipt-id","kind":"artifact"}],
  "command": {"type":"pause_run","run_id":"run-1"}
}
```

`command_id` is an actor-scoped idempotency key. Repeating the exact authenticated body returns the committed result with `replayed: true`; reusing the identity with different content returns `conflict`. The durable external ledger survives daemon restart and has a configured finite record bound. Mutating requests are not implicitly retried by `milkdrift-control-client`; a caller retry must preserve the exact body and idempotency identity.

The closed command types are:

| Type | Required body fields | Purpose |
| --- | --- | --- |
| `import_blueprint` | `document` | Validate and store an immutable blueprint revision. |
| `validate_blueprint` | `document` | Validate without storing. |
| `start_run` | `run_id`, `workflow_id`, `revision_id` | Atomically create then start at an exact revision through ordinary control authority. |
| `pause_run`, `resume_run`, `cancel_run` | `run_id` | Durable run lifecycle control. |
| `signal_run` | `run_id`, `signal_id`, `signal_type`, `correlation`, `broadcast`, `payload` | Deliver a typed bounded signal. |
| `resolve_work` | `run_id`, `attempt_id`, `decision_id`, `action`, `remediation_node` | Query, retry, compensate, retain, or evidence-resolve uncertain work. |
| `submit_proposal` | `document` | Submit an exact schema-1 workflow proposal through `milkdrift-control`. |
| `decide_proposal` | `run_id`, `proposal_id`, `proposal_digest`, `proposed_revision`, `decision_id`, `decision` | Approve or reject an exact proposal. |
| `apply_proposal` | `run_id`, `proposal_id`, `proposal_digest`, `proposed_revision` | Apply an approved prospective revision through reconciliation. |
| `put_layout` | `layout` | Optimistically store presentation-only state. |

Evidence kinds accepted by the daemon are `authority_decision`, `worker_observation`, `external_receipt`, `artifact`, and `recovery_observation`. A success returns `CommandAccepted`: `command_id`, `replayed`, optional `resulting_sequence`, stable `result_type`, and a bounded command-specific `value`.

## Query routes

Every route is authenticated and authority-filtered.

| Method and path | Result |
| --- | --- |
| `GET /v1/health` | Bounded liveness, lifecycle, queue, and worker health. |
| `GET /v1/readiness` | The same model; returns 503 until startup recovery and required adapter initialization finish and while draining. |
| `GET /v1/revisions?limit=&cursor=&workflow=` | Stable bounded revision-summary page. |
| `GET /v1/revisions/{revision}` | Immutable revision summary, lineage, provenance, counts, and bounded document. |
| `GET /v1/revisions/{from}/diff/{to}` | Bounded structured semantic diff with an explicit truncation flag. |
| `GET /v1/runs?limit=&cursor=&state=&workflow=` | Stable bounded compact-run page. |
| `GET /v1/runs/{run}` | Compact current run status and retained execution frontier. |
| `GET /v1/runs/{run}/nodes/{execution}` | One retained node execution. |
| `GET /v1/runs/{run}/attempts/{attempt}` | One retained latest attempt, including exact capability/context-manifest provenance when present. |
| `GET /v1/runs/{run}/timeline?limit=&cursor=` | Paged external timeline projection with exact durable sequence anchors. |
| `GET /v1/runs/{run}/proposals?limit=&cursor=` | Bounded proposal identities/statuses discovered from the durable command ledger. |
| `GET /v1/runs/{run}/proposals/{proposal}?revision={revision}` | Exact status from `milkdrift-control`. |
| `GET /v1/capabilities` | Visible capability generations, descriptor digests, selection/drain/health/availability, and permit bounds. |
| `GET /v1/authority` | Current server-owned actor, grant, revision, revocation generation, and configured operation labels. |
| `GET /v1/artifacts/{artifact}` | Safe digest, size, media type, disposition name, and sensitivity; never a server path. |
| `GET /v1/artifacts/{artifact}/content` | One verified explicit byte range under artifact-read authority. |
| `GET /v1/layouts/{workflow}/{revision}` | Exact independent layout document. |

`limit` defaults to 100 and must be within the protocol page bound. A page contains `items`, optional `next_cursor`, and optional feed-head `observed_cursor`. Clients must request subsequent pages explicitly; the client library never auto-loads a complete run lifetime.

Artifact content accepts one `Range: bytes=start-end` request and returns 206 with `Content-Type`, `Accept-Ranges: bytes`, `Content-Range`, safe `Content-Disposition: attachment`, and `x-milkdrift-artifact-complete`. A server call returns at most 1 MiB. There is no arbitrary path access or public upload endpoint.

## Read models

External timeline entries use the stable categories `lifecycle`, `execution`, `progress`, `artifact`, `coordination`, `authority`, `recovery`, `reconciliation`, and `uncertainty`. An entry carries the exact durable sequence, timestamp, bounded actor and run/node/attempt/revision references, stable summary, and bounded structured detail. It is deliberately not an internal `RunEventKind` document.

Run models carry aggregate sequence, stable lifecycle, optional terminal outcome, workflow/revision/digest, a compact retained node frontier, and unresolved-uncertainty count. Node and attempt models carry immutable execution/revision/attempt anchors, current state, exact resolved capability, optional context-manifest artifact metadata, terminal summary, and uncertainty. Complete lifetime history remains the paged journal-backed timeline.

## Cursors and SSE

Cursors are opaque Base64url schema-1 values bound to one exact feed and either a monotonic sequence or a stable key. A malformed cursor or one copied to a different feed fails explicitly. Clients must store only a successfully observed cursor and resume after it.

The daemon exposes:

- `GET /v1/runs/{run}/stream?cursor=…` for projected timeline and compact status updates on feed `run:{run}`.
- `GET /v1/stream/capabilities?cursor=…` for a bounded retained window of capability generation/health snapshots on feed `capability-health`.
- `GET /v1/stream/health?cursor=…` for coarse daemon health on feed `daemon-health`.

SSE `data` values are `ObservationEnvelope` documents with protocol, cursor, observation time, feed, and one closed external observation: `timeline`, `run_status`, `capability`, `daemon_health`, `stream_closing`, or `resync_required`. The capability feed records a new ordered snapshot only when its bounded public generation/health view changes and retains the latest 256 observations; an older continuation receives `resync_required`.

Run-feed positions interleave durable timeline sequence (`2 × sequence`) and its following compact status (`2 × sequence + 1`). Transport heartbeats are SSE comments and are never durable events. Server generators and owner calls are bounded; backpressure retains no unbounded per-client event queue. Authentication is re-resolved on every polling cycle. Rotation/revocation, draining, invalid history, or authorization change closes the feed with an observable closing/resync item where possible.

`milkdrift-control-client::subscribe` reconnects retryable transport failures with its last successfully decoded cursor. Reconnect never submits or replays a command.

## Layout schema 1

`LayoutDocument` contains `schema_version`, exact `workflow_id` and `revision_id`, positive optimistic `generation`, server-overwritten `author`, independent BLAKE3 `digest`, bounded `nodes` positions/dimensions, `collapsed_groups`, non-executable `annotations`, and optional `viewport`. The first generation is 1; a changed document must advance by exactly one. The daemon recomputes author and digest before storing.

Layout cannot contain executable edges, node/task configuration, requirements, prompts, secrets, or semantic mutations. It is stored in the atomically synced schema-1 control sidecar, independently from the immutable blueprint revision. Layout edits therefore do not create a revision or alter its semantic digest.

## CLI automation contract

`milkdrift-cli --json` prints one compact line per result:

```json
{"schema_version":1,"type":"run.show","value":{}}
```

JSON mode has no colors or terminal control sequences. Exit categories are 0 success, 2 invalid input/confirmation, 3 authentication or authority, 4 conflict, 5 retryable unavailable/overload/transport, 6 not found, 7 internal/nonclassified failure, and 8 when `run show` observes a failed terminal task. Cancel and proposal decision/apply operations require interactive `yes` or `--yes`; JSON-mode high-risk operations require `--yes`. Credentials come from `--token-file`/`MILKDRIFT_TOKEN_FILE` or the configured environment-variable name, never a token command argument. Artifact downloads require an explicit new destination and remove a partial file after failure.
