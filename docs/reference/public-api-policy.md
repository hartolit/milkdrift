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

`ProjectSettings`, `TaskExecution` and `TaskProjectBinding` are blueprint contracts shared by
control, runtime, persistence and execution adapters. Admission resolves a reviewed project
generation or a complete explicit choice into one immutable binding. The binding carries exact
native folder identities, permitted scopes, access and tools; an empty explicit tool list remains
empty. Human and model requests use the same resolver and retain their original canonical request
for replay. A text edit can preserve its accepted binding without adopting newer project defaults.
The separate project generation guard belongs to the workflow commit transaction, so a concurrent
settings change cannot accept newly resolved work against an unchecked generation.
Automatic feedback retains its source binding; a changed project generation closes that repair
attempt with a durable refusal instead of silently adopting new permissions.
`ProjectStore`, registrations, operation states and mutations are workspace storage contracts used
by the daemon and shared task control owner. Redb alone commits bounded project receipts and
workflow associations; native folder observations come from the execution host. Creating reserves
both registration and recovery capacity before its filesystem effect. Explicit reconciliation
settles that original intent without rewriting accepted task documents or deleting folders.
`InspectProject` and `ManageProject` express independent current authority over the pinned folder.
Project DTOs and client methods are external transport projections, including the read-only
`TaskExecutionDto::Restricted` marker that keeps permitted task inspection available without
disclosing inaccessible folder bindings or creating a reusable authoring definition.
`control-protocol::projects` owns exact command and receipt framing, bounded folder choices and
configured environment projections. Native `ControlClient` project methods use the same public
routes as Svelte and CLI; `workflows_in_project` adds project/earlier discovery filters without
changing workflow ownership. A hidden project association may appear as null in an authorized
workflow read, while the earlier filter uses the actual durable association.
Runtime's `create_resolved`, `edit_resolved` and typed replay methods preserve the native request's
storage byte bound while its `*_exact` public entry points retain the smaller transport bound.
Control is the production caller supplying both the original request and its resolved definition.

`TaskExecutor::pin_project_paths` delegates physical path capture to the execution host.
`capture_native_project_root` and `verify_project_binding` are workspace adapter contracts used by
project registration and process preparation; control does not independently interpret filesystem
identity. Runtime carries the binding through resolution and dispatch, and adapters revalidate it
before effects. Native claims still own conflicting writer exclusion across workflows and aliases.
The project-aware process mode pins its working directory to that same accepted root. Its
test-only context constructor stays behind the existing helper feature.
`CapabilityAdapter::accepts_project_without_access` captures the configured process isolation
choice once at registration. The host uses it to admit an explicit no-access task without
changing old process descriptor digests or trusting a client assertion about isolation.
`CapabilityAdapter::native_path_claims` declares a configured process working directory to the
task host. The model loop collects these claims from its authorized process tools before entering
the model, so earlier fixed-directory task profiles share the same writer exclusion and uncertain
attempt holds. These implicit claims do not grant native file tools additional readable roots.
`NativePathScope` is the host's adapter-effect contract. Process adapters use the validated
`ProjectProcessAccess` view from `AdapterExecutionContext::project_process`, which checks the
selected generation, whole-folder scope and change policy without exposing task definitions to
those adapters. The context also revalidates the native identity at entry and supplies the typed
task workflow identity needed by managed storage.
`install_native_path_store` accepts the daemon composition's current
canonical operator paths. The existing native path owner checks these at admission and entry,
including retained bindings after restart; it does not rewrite their accepted settings or change
independent process-profile authority.

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
`admit_model_batch_exact` preserves the source batch's canonical bytes when control resolves its
project choices. `AdapterExecutionContext::model_tool_owner` exposes only the retained parent
invocation and finite call bound needed by managed execution to acquire a nested tool hold; it
does not give a model a separate admission path.
`task_tool_invocation` supplies the shared deterministic child identity to capability-host and
the managed store. `ManagedResourceStore::reserve_managed_task_tool` commits that child's existing
generation hold only against an entered task parent and the exact bounded process selection.
The durable `ManagedExecution::TaskTool` variant retains that parent and request digest for
integrity and uncertainty recovery; it adds no scheduler or resource table.
`ManagedWorkerAdapter::with_project_pin_helper` and `run_project_pin_helper` are workspace adapter
contracts consumed by daemon composition and its early internal helper role. The library does not
discover an ambient executable. The helper owns only a directory descriptor and finite process
lifetime; it creates no workflow, manager, credential source or runtime policy.

`WorkflowStore` commits a snapshot and exact receipt atomically with generation and capacity checks.
Its artifact, entered-invocation and managed-use facts allow the durable owners to enforce holds
and publication without decoding a second copy of runtime semantics. Native-path claim ports
coordinate physical path identity and preserve uncertain writers. They are adapter contracts,
not alternate schedulers.
`NativePathClaimStore::claim_serving_native_paths` checks the exact current serving dispatch
claim in the same transaction that acquires paths. The serving record owner releases those paths
atomically with terminal evidence or pre-entry requeue; cancellation races cannot recreate them,
and entered uncertainty keeps them protected. Direct and peer invocations therefore share the
same physical conflict owner as project tasks without introducing another execution registry.

Public task DTOs in `control-protocol::tasks` are transport contracts for CLI, Svelte and independent
JSON consumers. Rust generates the TypeScript declarations and envelope decoder. Summary/read/result
DTOs deliberately have different payload sizes: a discovery page never embeds all captured files.
`validate_response_capacity`, cursor binding and error status mapping remain pure wire helpers.
`TaskSummaryDto::entered` exposes the runtime's durable entry intent separately from start acceptance;
the browser and CLI can distinguish preparation without fetching frozen input bytes. Control 3.5
coordinates this required field across readers and generated browser types.

`blueprint::authoring` describes incomplete proposals and connection intentions for shared control;
it does not replace immutable task definitions. `TaskDependencies` reads historical arrays with
their original predicates and writes new independent `TaskOrderCondition` maps. `Rename` changes
workflow metadata without a task obligation. Optional `TaskDefinition::capability_generation`
retains the reviewed adapter generation for admission and pre-start checks; absence preserves old
durable encoding. `TaskInputMedia` and the capability-owned authoring contract let adapters, host,
runtime and control enforce the same known-media rules. `TaskExecutor::task_authoring` returns the
descriptor and connector contract together so control checks existing catalog authority against
all descriptor facts before exposing choices.

`WorkflowAuthoringSnapshot`, `WorkflowAuthoringCommit` and independently versioned position types
are workspace storage contracts. `WorkflowCommit::authoring` lets runtime's `edit_authoring_exact`
commit the reviewed batch, exact receipt and remaining proposal together. Redb owns conditional
replacement, protected source references and bounded capacity accounting; it never interprets
the proposal as a second workflow. `TaskControlService` owns authorized checkpoint, review and Run
construction. `AuthoringScope`, `AuthoringReview` and bounded `AuthoringIssue` are consumed by the
daemon's transport mapping. Their public DTOs and client methods also serve CLI and Svelte.
`WorkflowTopologyDto` is a bounded accepted-connection projection with its own generation-bound
cursor, independent of selected task/result reads. Its rows never grant authority over accepted
definitions or make saved presentation references authoritative.

`PersistenceError::NativePathContention` identifies only validated overlapping claims and exposes no
blocking owner or path. The host maps it to `ExecutorError::NativePathBusy` for ordinary-task waiting;
other storage or identity failures retain their distinct refusal. `TaskExecutor::defer_before_entry`
releases preparation claims only while runtime owns a proven-unentered dispatch after dropping its
prepared handle. Failed cleanup retains uncertainty. `TaskRuntime::recover` restores this state to
the ordinary scheduler; it no longer hands a second dispatch queue to the worker host.

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
