# Public API policy

Milkdrift is unreleased. Workspace libraries, daemon and clients upgrade together. Rust APIs,
public protocols and durable development formats may change in one coherent revision; update
producers, consumers, fixtures and documentation together and refuse unsupported versions.
An incompatible upgrade must not reinterpret saved facts or silently give old requests authority.

## Classification

Every exported item has a current reason to cross its package boundary:

1. **External product contract:** intentionally consumed by an application or adapter.
2. **Workspace adapter contract:** implemented or consumed by another workspace package.
3. **Durable schema contract:** required to interpret accepted storage or exchanged data.
4. **Test-only exposure:** explicitly gated fixture/fault support.
5. **Accidental exposure:** no current consumer or invariant; remove or narrow it.

A workspace consumer can justify visibility without promising a stable third-party API. Root
re-exports make an intentional entry point or returned type nameable. Private implementation
modules stay private. Do not preserve a retired owner solely to keep an old API inventory unchanged.

## Surface ownership

Blueprint task definitions and bindings are semantic contracts consumed by runtime/control and
serialized in frozen starts. Runtime's task records expose immutable starts/results to persistence
and authorized daemon projections. Construction and transition ownership remain in runtime;
clients never write those records directly.

`MAX_TASK_OUTPUT_BYTES` and `TaskOutputDeclaration::validate_reference` expose the shared finite
output contract to runtime and daemon projections. `validate_task_set` takes a retained-output
resolver supplied by runtime so an actual historical publication can satisfy a future connection;
the browser cannot supply publication evidence. Runtime and control's `replay_exact` methods recover
the same canonical actor-scoped receipt before resolving today's authoring choices. Control checks
current receipt-read authority; an absent receipt still follows ordinary mutation admission.
Runtime's `replay_edit` gives native control callers the same receipt preflight for a typed batch,
retaining the storage request bound without applying the smaller public transport envelope bound.
`StoreInvocationDataAccess::with_task_authority` lets the daemon install its existing evaluator and
clock at local task materialization. The host checks actual stored artifact scope before releasing
bytes; this does not replace independent direct/peer admission or change the accepted grant.

`InvocationAdmissionEnvelope::execution_duration_ms` is an adapter contract: its bound must come
from an enforced attempt deadline, never a descriptor estimate. Runtime's `TaskExecutionDuration`
is durable evidence for an accepted reservation and its elapsed-time watermark. These exports
support the same cumulative workflow account; they do not create a second scheduling or budget owner.

`TaskActionPolicy` and `TaskTool` are task-definition contracts. Runtime's `ModelAdmission` and its
status retain the source response, decision and exact receipt linkage; control consumes them to
recover admission, while public DTOs expose a bounded projection. `admit_model_batch` uses the same
runtime edit transaction as humans. The host's `ModelToolLoopAdapter` consumes provider-neutral model
messages and the configured adapter's maximum envelope; its loop cannot create workflow tasks.
Frozen dispatch tool fields carry explicit permissions into that host boundary, never UI state.

`WorkflowStore` commits a snapshot and exact receipt atomically with generation and capacity checks.
Its artifact, entered-invocation and managed-use facts allow the durable owners to enforce holds
and publication without decoding a second copy of runtime semantics. Native-path claim ports
coordinate physical path identity and preserve uncertain writers. They are adapter contracts,
not alternate schedulers.

Public task DTOs in `control-protocol::tasks` are transport contracts for CLI, Svelte and independent
JSON consumers. Rust generates the TypeScript declarations and envelope decoder. Summary/read/result
DTOs deliberately have different payload sizes: a discovery page never embeds all captured files.
`validate_response_capacity`, cursor binding and error status mapping remain pure wire helpers.

`CapabilityAdapter::task_authoring`, `TaskAuthoringContract` and `TaskAuthoringPort` are workspace
adapter contracts. Process and model adapters derive connector choices from their existing private
configuration; the host validates and captures them once per registered generation, then filters
them with the same operation scope as capability discovery. They are not hashed descriptor fields,
so introducing this read projection does not change old frozen descriptor digests. Session authoring,
project-root and supplied-input DTOs are transport projections for the canvas. Their names, bounds
and availability come from those existing owners, never from a browser-owned capability catalog.

`DaemonDisplayName` is the one validated Rust name type. `DaemonMetadataStore` and its metadata,
rename and receipt types are workspace storage contracts used by the daemon's private metadata
module and implemented by redb. They bind names to an existing installation and atomically retain
actor-scoped exact receipts; they do not create a generic settings service or a synthetic workflow.
`RenameDaemon` crosses the authority boundary independently of workflow editing. Existing grant
schema-5 documents remain readable, while changed preset content requires deliberate grant revisions.

`DaemonMetadataDto`, `RenameDaemonRequest`, `RenameDaemonReceipt` and the session's name fields are
external transport contracts shared by Svelte and the Rust client/CLI. Display-name validation stays
with the persistence type, not the DTOs or frontend. `ControlClient::with_daemon_identity` preserves
an explicitly learned pin on subsequent requests and refuses replacing it on that client. It does
not make an endpoint trustworthy or impose a persistent profile on every native caller.
`DaemonIdentityMismatch` distinguishes replacement from an ordinary optimistic conflict so clients
can discard inaccessible active state without discarding a valid conflict draft.

Independent `DirectInvocationDraft`, request preparation, serving invocation records, peer limits
and artifacts remain their host/protocol owners. Managed-resource requests and durable holds remain
independent of ordinary task definitions. No public graph, proposal, controller, prompt-sequence,
published-workflow or evaluated-learning API survives solely for old product consumers.

Evidence library exports serve separate Rust fixture/measurement binaries. Product dependency
resolution must not enable `test-support`, `test-admin` or `operational-evidence` helper features.
The shared child/HTTP fixture lifecycle preserves failure and cleanup evidence; it is not a product
server implementation. UI caches and generated DTOs add no durable authority.

## Review method

Trace changed exports through production callers, tests, docs and wire/durable readers. Inspect
library roots and default/all-feature inventories, keeping raw generated reports out of Git:

```sh
mkdir -p target/public-api
cargo public-api -p PACKAGE -sss --all-features --color never > target/public-api/PACKAGE.all-features.txt
cargo public-api -p PACKAGE -sss --color never > target/public-api/PACKAGE.default.txt
```

The default comparison is required for packages with test/evidence features. Check dependency
resolution independently for production daemon/CLI; a workspace all-feature build can hide a
helper-feature leak. Record which inventories and checks actually ran in verification evidence,
including toolchain or environment limits. A type count is not proof of a coherent boundary.

The prompt-01 task cutover intentionally removes the previous graph/publication/controller API
families. Review new task, storage, host and transport boundaries together with their consumers.
Later method and automation exports require their own explicit assignments and review.
