# Code-change map

This map assigns the source changes needed for the [implementation sprint](README.md). The
[task-workflow decision](../../../decisions/0051-daemon-task-workflows.md) owns the selected
semantics and reasons. These are proposed changes, not a claim that the current code implements
them. The map uses current source and the preserved v3 observations; it does not repeat that
investigation or carry its superseded architecture into this sprint.

The October 17 partial user review adds the prepared
[02a connection](02a-daemon-connection-identity.md) and
[02b visual](02b-visual-design-and-brand.md) corrections before 04. Their current owners are:

| Boundary | Correction and affected consumers |
| --- | --- |
| Daemon `Owner`, authority, persistence/redb | One shared display name, narrowly authorized durable rename, exact receipts and bounded current-store adoption; retain stable installation identity. |
| Public session/command DTOs, control client, CLI and Rust fixture | Read/bootstrap shared metadata, execute the same rename command, keep identity and actor isolation; update generated TypeScript and operator instructions. |
| `apps/web/src/App.svelte`, `components/ConnectionForm.svelte`, `lib/connections.ts`, `lib/transport.ts` | Address/credential connection, automatic pinning and old-profile adoption, shared-name refresh, clear replacement refusal. |
| `apps/web/src/style.css`, `App.svelte`, task/timeline/inspector/editor components and public assets | Apply the supplied dark visual direction and real logo throughout the working client; preserve current operations, responsive layout and accessible task views. |

These are selected corrections, not implemented capabilities. The remainder maps the original
feature assignments; their tested standing is owned by status and the current handoff.

## One representation and one scheduler

Keep the existing `blueprint`, `runtime`, `control`, `persistence`, `authority`, `workspace`,
capability and transport package boundaries. Replace the ordinary workflow representation inside
those owners with daemon-owned tasks, explicit dependencies and input bindings. Do not compile
these tasks into the old structured graph, add a second runtime, or make the Svelte client translate
ordinary work into revision/proposal/controller operations.

`blueprint` owns validated future task definitions and reusable method templates. `runtime` owns
readiness, attempts, frozen execution inputs, scheduling, completion and recovery. `control` owns
shared admission of human and agent requests; `authority` decides their scope. `persistence` owns
atomic storage ports and `redb-store` implements them. Capability-host and its adapters retain their
external-operation responsibilities. Keeping these crate names does not require keeping obsolete
public types or preserving their old wire shapes.

Phase 01 introduces the general task/dependency/input representation, version guards and workflow
limits, even though its first visible journey begins with a single task. Phases 03 and 04 extend
that production path with model-produced changes and methods. They do not replace the first slice.

## First slice and cutover cost

Phase 01 is the largest structural assignment: a useful standalone Svelte client must accompany
replacement of the old ordinary workflow path. It is not just an endpoint wrapper or a small UI
change. Make a coherent, tested production browser commit for submitting meaningful work, reading
its real result and reconnecting before doing the remaining cutover cleanup. That checkpoint must
use the selected task representation and the runtime's sole scheduling path. It may precede removal
of now-unreachable old modules and fixtures; it may not add a parallel selectable workflow engine.
Finish the cleanup, callers and focused regressions before the phase handoff.

The replacement uses a new store format and fresh explicitly selected store path. Refuse an old
store clearly without opening it for mutation, importing it, or treating it as empty. The archived
code and preservation references retain old readers and evidence. This is a development cutover,
not permission to reset a store, stop a service, migrate active work or weaken old managed protection.
Any real external operation discovered during implementation remains with its existing owner.

The first slice removes workflow-backed publication and evaluated-learning product routes because
they depend on the retired workflow model. It preserves generic direct execution, configured model
and process adapters, execution-only hosting, peer tool transport and managed-resource safeguards.
Removing publication does not authorize bypassing a protected installation's checks or converting
its state into an unprotected installation. Old stores remain refused by the new format.

## Owner and consumer changes

Paths below are repository-relative. Existing paths identify the source to change; proposed modules
belong under the stated owner. Search their consumers before editing and update them in the same
phase. The list is a responsibility map, not a restriction against correcting an affected caller.

| Owner and source | Selected change and consumers | Phase |
| --- | --- | --- |
| `crates/blueprint/src/model/node.rs`, `model/structured.rs`, `revision.rs`, `mutation.rs`, `validation.rs` and `validation/` | Replace `NodeKind`, structured control ports, pinned child references and revision-mutation authoring with task definitions, explicit dependencies, input bindings and versioned future edits. Reuse bounded validation and content identity mechanics where their meaning fits. Update authority, runtime, persistence, protocol, daemon and CLI consumers. | 01; template definitions in 04 |
| `crates/runtime/src/engine/dispatch.rs`, `engine/scheduling.rs`, `scheduler.rs`, `projection/` | Make task readiness and attempt state the canonical scheduler/projection model. Preserve bounded admission and fair discovery. Freeze actual instructions and resolved inputs at start. Remove dependence on screen order, identifiers or prose for scheduling. | 01; multiple tasks and failure-input behavior in 03 |
| `crates/runtime/src/engine/structured/`, `engine/reconciliation.rs`, `reconciliation.rs` | Remove pinned child runs, graph fork/join/reducer/repeat machinery and graph reconciliation adoption from the production task path. Prospective task edits use explicit task versions and lifecycle checks. No lowering layer remains. | 01 |
| `crates/runtime/src/executor.rs`, `engine/effects/entry.rs`, `engine/effects/reporter.rs`, `engine/recovery.rs` | Port the existing preparation, final authority check, durable entry intent, exact reports, cancellation and uncertain-outcome rules to task/attempt identities. Preserve selected generation and accepted request after interruption; do not replay an entered effect merely because its response is absent. | 01; remote continuation exercised in 05 |
| `crates/control/src/command.rs`, `service.rs`, `controller/`, `adapter/`, `policy.rs`; `apps/daemon/src/host/commands/control.rs` | Replace graph proposal/controller lifecycle with the ordinary typed task-change path shared by people and model responses. Retain scoped authorization and cumulative allowance enforcement. Remove the requirement to install an old controller lifecycle before workflow accounting can govern entry. Model output remains proposed data until this owner accepts it. | 01 and 03 |
| `crates/authority/src/model/resource.rs`, `model/grant.rs`, `model/execution.rs`; `apps/daemon/src/host/authorization.rs` | Carry workflow/task scope and frozen execution provenance through the current evaluator. Add the narrow method-use, template-owner-edit and connection-administration operations when their clients arrive. Actor claims in model text grant no authority. | 01, 04 and 05 |
| `crates/persistence/src/journal/commit.rs`, `event/`, `snapshot.rs`, `revision.rs`, `controller_account/`; `adapters/redb-store/src/schema.rs`, `store/open.rs`, `journal/`, `snapshot.rs`, `revision.rs` | Replace old graph records and readers with fresh-format task storage. Reuse guarded atomic commit, exact receipt, accounting, artifact and clock mechanisms. Definition edits, accepted task state and their discoverability commit together. Replace controller-specific naming/ownership where needed so cumulative workflow limits survive retries and corrections. | 01; expansion transaction in 04; expiry in 05 |
| `crates/control-protocol/src/{command,authoring,read,result}.rs`; `crates/control-client/src/` | Introduce the public task commands, reads and failure responses used equally by CLI, Svelte and agents. Retain bounded documents, authenticated cursors and exact replay/conflict behavior. Replace ordinary `SavedRunRequest` prerequisites with daemon discovery and request-result recovery. | 01 onward |
| `apps/daemon/src/host/commands/`, `host/runs.rs`, `host/attempts/`, `host/results.rs`, `http/reads.rs`, `http/streams.rs` | Replace ordinary blueprint/run/proposal routing with workflows and tasks. Provide authorized lists and result inspection without known IDs, then title/description/time search, origins and follow-up links. Actual start/end times come from execution facts; queued work has no invented time. | 01; advanced discovery in 05 |
| `apps/daemon/src/http.rs`, `auth/`, `config/{wire,compile}.rs` | Add the configured browser-origin, authentication and streaming behavior required by the standalone client. Existing router code deliberately omits CORS. Integrate preflight and rejected-origin behavior with supported HTTPS deployment rather than relying on native HTTP tests. | 01 |
| Proposed `apps/web/` | Build a standalone Svelte client over public operations. Keep connection profiles, workflow views, settings and permissions separated by selected daemon. Production submission/result/reconnect comes first; later phases add editing, automatic work, methods and environments. No CLI execution, store access or client scheduling. | 01 through 06 |
| `apps/cli/src/command/{workflow,blueprint,sequence,run,proposal,controller,method}.rs`, `workflow_args.rs`, `session.rs`; `crates/prompt-sequence/` | Replace the obsolete workflow command families and draft/recovery-file requirements with thin task operations. Remove the prompt-sequence compiler when its old consumers are removed. Keep exact request recovery useful without making a local file the discovery authority. Update machine output and documented examples with the supported path. | 01; additions match 03–05 |
| `crates/model/src/task.rs`; `crates/capability-host/src/adapter/context.rs`, `materialization/`; `adapters/model-provider/` | Reuse provider-neutral instructions, responses and tool-call data. Materialize authorized text-file inputs through the owning host into bounded model content; preserve frozen byte identity. Current provider mappings refuse generic artifact/file content despite representing those variants. A DTO alone cannot satisfy file input. | 01 and 03 |
| Proposed method modules in `crates/blueprint`, `crates/control`, `crates/persistence`, `adapters/redb-store`; daemon/client method operations | Implement the daemon-wide template library, owner edits and queued references. Activation atomically records the selected saved template content, instantiated task identities, inserted tasks and released reference. Interrupted replay cannot select newer content or insert twice. Ordinary future-task edits then apply to materialized copies. | 04 |
| `apps/daemon/src/host/peers.rs`, `http/peers.rs`, `config/`; `adapters/peer-http/`, `crates/peer-protocol/` | Add durable daemon-owned connection CRUD and credential-reference administration. Existing endpoints only connect/drain configured peers; they do not create persistent configuration. Keep local administrative rights separate from remote execution credentials. Clients inspect redacted host state through their selected daemon. | 05 |
| `crates/capability-host/src/serving/`, `registry/`, `worker.rs`; `apps/daemon/src/host/peer_store.rs` | Preserve independent direct/delegated execution, exact accepted request identity, generation ownership, remote cancellation and artifact transfer. Change workflow-origin provenance consumers to the task model without moving remote workflow ownership to the host. | 01 compatibility of retained owners; 05 connected journey |
| `crates/capability-host/src/managed.rs`, `managed/`; `crates/persistence/src/managed/`; `adapters/managed-linux/` | Preserve generic approved setup, generation checks, resource limits, editing claims, lifetime holds, prepare/apply/reopen/remove and uncertainty. Remove only workflow-publication/evaluated-learning hooks whose product callers retire. Keep ordinary files and tools responsible for project procedures and documentation. | 01 removal boundary; 05 use and regression |
| `apps/daemon/src/host/maintenance.rs`; persistence artifact, journal and receipt owners; `adapters/redb-store/src/artifact/cleanup.rs` and related paging | Add configured rolling expiry with durable holds for unfinished work, pending input references and unresolved operations. Retention changes use the same guarded owner as reference changes. Bound cleanup and report blocked capacity honestly. Use explicit expired-content responses while preserving exact request replay/conflict. | 05 |
| Root manifests; `tools/evidence/`; tests and canonical docs | Remove retired package/API dependencies and fixtures, update repository contracts and API inventories, and maintain actual-binary evidence owners. Add appropriate Svelte checks and browser evidence. Backend harnesses remain Rust. | Every owning phase; integrated gate in 07 |

## Retired product paths

Add the narrow native-path claim implementation under capability-host with durable persistence
records in 01, and parallel overlap/restart coverage in 03. Existing managed editing claims are
installation-scoped and cannot be assumed to own ordinary project folders. The same persistence
owners reserve settlement and mandatory control capacity before accepting tasks/connections.

Phase 01 removes ordinary workflow publication and evaluated-learning registrations and their
callers across `crates/control/src/published*`, `crates/control/src/learning*`,
`crates/capability-host/src/published.rs`, `serving/published.rs`, `registry/published.rs`,
`apps/daemon/src/host/commands/publications.rs`, `commands/learning/`, and startup capability
composition. Remove the obsolete DTOs, CLI operations, storage tables/readers and graph-specific
tests along with them. `crates/persistence/src/published.rs` and agreement records must not remain
as an alternate workflow owner solely to retain old APIs.

Review `crates/capability-host/src/managed/protected.rs` and platform verifier hooks by actual caller:
remove dead workflow-publication/evaluation support, preserve enforcement needed by retained generic
managed operations, and never relax a check just to make the build pass. Retired examples and their
evidence stay available in the preservation refs; maintained operator examples must describe the new
product. Do not require the new workflow engine to reproduce the Slotbook product or learning system.

The old subworkflow, independent-copy and method-publication operations are not three alternate
implementations of methods. They retire. The new method operation expands current saved content into
the owning workflow only when activation is accepted.

## Checks to port and extend

Preserve the behavior that a test establishes, not an obsolete type hierarchy. Rewrite its setup
through the new production command path where needed and delete assertions for retired semantics.
Keep test-only control of external responses; a model fixture must not construct tasks, schedule
them or recover their storage on behalf of the daemon.

| Existing evidence or test owner | Behavior to retain and new case to add |
| --- | --- |
| `apps/daemon/tests/control_plane/durability.rs` | Exact idempotency/restart and stale-guard refusal; add simultaneous task edits and unknown submit outcome followed by fresh-client discovery. |
| `apps/daemon/tests/control_plane/inputs.rs` | Frozen, isolated supplied inputs and pre-entry bounds; add missing inputs, failed-result outcomes, pending consumers, text-file materialization and prospective rebinding. |
| `apps/daemon/tests/control_plane/discovery.rs`, `independent_client.rs` | Public JSON-client completeness and grant-rechecked discovery; remove the need to know a workflow ID and test disjoint authorized scopes. |
| `apps/daemon/tests/control_plane/streams.rs`, `apps/daemon/src/host/tests/live_streams.rs`, `crates/control-client/src/sse/tests.rs` | Cursor restart semantics, authority isolation and credential change; add browser reconnect after disposal of client state and truthful resynchronization after retention. |
| `crates/runtime/tests/structured_runtime/reconciliation/cancellation.rs` | Cancellation before final entry, after effect claim, and one safe replacement after confirmed stop; port to task cancellation and replacement without graph ancestry. |
| `crates/runtime/tests/structured_runtime/retry_recovery/`, `executor_faults.rs`, `data_integrity/` | Entry classification, interrupted reporting and unsafe-retry refusal; add recovery during accepted task changes and template expansion. |
| `crates/persistence/tests/contracts/atomic_commands.rs`, redb contract and reopen tests | Atomic command/result/index/account transitions; add template edit-versus-activation, nested-reference holds, no duplicate expansion, cleanup-versus-new-reference and capacity refusal. |
| `apps/daemon/tests/control_plane/direct.rs`, `roles.rs`, `resources.rs`, `process_cleanup.rs`; capability-host serving/conformance suites | Retained independent execution-only hosts, direct operations, resources and process lifetime; exercise after publication/controller removals in phase 01. |
| `apps/daemon/tests/two_daemon_peer.rs`, `two_daemon_peer/placement.rs`; peer adapter/serving suites | Authenticated remote catalogs, accepted remote operation recovery and generation draining; add persistent connection CRUD, revoked access and interrupted command inspection without duplicate entry. |
| Managed-linux lifecycle and resource suites | Exact owned-resource behavior, refusal, limits and reopen; distinguish controlled platform fixtures from explicitly authorized physical Podman evidence. |
| Proposed `apps/web` browser suite and Rust actual-binary harness | Meaningful submission, real output, disconnected continuation, multi-daemon separation, failed-input repair, template edit/restart, remote uncertainty and fresh-client search. Test browser behavior, not just native HTTP endpoints. |

Phase 02 and phase 06 are actual user-review-and-repair checkpoints for the early and combined
journeys. Their feedback may change behavior within the selected account; they are not cosmetic
acceptance screens. Phase 03 owns automatic additions, parallel research, corrective work and limits.
Phase 04 owns methods. Phase 05 owns persistent remote administration, retention and advanced
inspection. Phase 07 runs the full gate and the combined product journey, fixes findings and verifies
the resulting code. Earlier phases compile changed packages and callers and run focused checks;
they do not claim full-system acceptance or leave known in-scope defects for phase 07.

## Disposition of former findings

The [v3 review and evidence](../milkdrift-remedial-planning-v3/deliverables/review-and-evidence.md)
retains the original observations, source scope, disagreements and limitations. The new sprint
changes their disposition; it does not rewrite their historical results.

| Preserved finding | Disposition in this sprint |
| --- | --- |
| Nested Any cancellation produced a scheduler/projection ownership disagreement; crossed All could stall. See the [runtime diagnostic](../milkdrift-remedial-planning-v3/working/compatibility-runtime-evidence.md). | These are real defects in the archived graph implementation. Phase 01 removes that graph machinery; it must not claim to have repaired old runs. Port final-entry/cancellation invariants to the new task path and test them independently. Do not reproduce the old branch-scope algorithm in methods. |
| Branch choice changed with port spelling in the [priority diagnostic](../milkdrift-remedial-planning-v3/working/priority-runtime-evidence.md). | The old branch primitive retires. Explicit dependencies and accepted model changes determine task meaning. Add variation in task IDs and display order so neither becomes an undeclared condition or dependency. |
| Existing inputs can be declared before their values exist. | Reuse this demonstrated separation. The defect to avoid is changing it into authoring-time input evaluation or model-text scheduling. |
| Existing pinned child execution and independent copy differ from deferred local method expansion. | Replace both for the new method use; phase 04 supplies one atomic activation contract and local origin evidence. |
| Browser evidence was negative; independent JSON clients worked. | Phase 01 supplies real browser transport/authentication behavior and preserves the independent-client boundary. A Rust client result is not browser proof. |
| A local cancellation fence cannot prove a remote effect stopped. | Keep remote acceptance/status/cancellation ownership and unresolved holds. Exercise interrupted remote command recovery in phase 05 and the combined journey. |
| Earlier proposal/program acceptance and compatibility conclusions concerned superseded representations. | They are not migration requirements or user validation. Preserve their evidence; remove obsolete contracts only with their callers and record the new fresh-store boundary clearly. |

No production changes or runtime checks were performed to produce this map. Its source inspection
supports the change locations and current limitations; implementation phases must produce the new
behavioral evidence.
