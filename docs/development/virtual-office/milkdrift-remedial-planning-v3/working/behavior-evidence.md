# Current behavior evidence — Cyra

This investigation supplies factual input to 01 and questions for 02. It does not select a
product model or endorse an architecture before the 02 gate. The important finding is that
Milkdrift already distinguishes workflow ownership, accepted host operations, reusable definitions,
published service calls and surviving installations in executable code. Several proposed client
outcomes still need a complete public route or browser evidence; names alone do not establish one.

## Source, exposure and evidence limits

- Investigator: **Cyra**, a separate source/consumer session dispatched by the coordinator.
- Inspected source HEAD: `908e7893f5dadb84d12712573c8daaa946829e39` on 2026-10-10. Initial
  `git status --short` was empty. `908e789` adds the v3 sprint; `6252766` adds the logo;
  `c016cd3` records live-stream evidence. The package's `c016cd3` reference is therefore an
  ancestor, not current HEAD. No reset or source edit was performed.
- Exposure: repository constitution; current vision, architecture, status and roadmap; practice
  guide, implementation/documentation practices and workflow; v3 README, 01, 04, scenarios,
  preservation context and deliberation protocol; relevant ADRs 0038 and 0041; current source,
  consumers, manifests and tests. This was implementation intake, not a blind primary-intent
  interpretation. No coordinator-preferred architecture was supplied.
- **Source finding** below means inspected code or test assertions. **Documented** means a
  maintained explanation or historical report. **Inference** states a consequence derived from
  those grounds. **Investigation recommendation** proposes a discriminating next step, not an
  approved implementation requirement.
- This session ran read-only source/history commands and retrieved primary browser specifications.
  It ran **no Cargo command, product journey, physical resource operation or browser-origin
  experiment**. Test names below locate executable assertions; they are not new passing results.
  The coordinator owns serialized test execution and its retained evidence.

The workspace manifest contains Rust domain libraries, adapters, daemon/CLI, evidence tools and
the Rust Slotbook example. It contains no Svelte or other frontend package. The daemon manifest
still depends on runtime/control at compile time; the role distinction below is a construction
and execution distinction, not an independently minimized binary claim.

## 1. A connection observes an owner; it does not become one

**Source finding.** [DaemonHost](../../../../../apps/daemon/src/host.rs) is a cloneable handle to
the same bounded owner queue, authentication registry and lifecycle. Its `Owner` retains the
store, capability host and optional `WorkflowServices`/effect workers. In
[startup](../../../../../apps/daemon/src/host/startup.rs), `DaemonHost::start_mode` creates one
owner thread; `Owner::open` opens the store, binds the durable serving host identity, and constructs
`RuntimeService` plus `ControlService` only for `HostRole::WorkflowEnabled`. Execution-only
startup calls `refuse_workflow_obligations` before becoming ready. Recovery startup separately
requires the workflow role and cannot become execution-ready.

The HTTP router holds a `DaemonHost` handle, not a client-owned runtime. The browser/tab/CLI
connection has no stored workflow owner role. `ControlClient::subscribe` explicitly drops local
observation without sending cancellation. `DaemonHost` documentation and shutdown code require
an explicit owner shutdown; closing a socket does not establish external quiescence.

Useful tests in `apps/daemon/tests/control_plane/roles.rs`:

- `execution_only_exposes_its_role_and_refuses_workflow_commands`: the role is publicly readable,
  workflow import returns nonretryable `Unavailable`, and no run records appear.
- `execution_only_refuses_live_workflow_obligations_without_rewriting_history`: role removal
  refuses active work and leaves its summary unchanged; restoring workflow mode can reopen it.
- `removing_workflow_role_preserves_closed_history_for_offline_inspection`: accepted closed
  history remains byte-equivalent through the role change. This establishes preservation and
  offline inspection, not ordinary online workflow reads on an execution-only host.
- `serving_installation_identity_cannot_change_on_restart`: changing `host_id` refuses reopen.

**Inference.** S07 and S22 require a client reference qualified by the responding host identity,
not just a name, URL or peer label. [SavedRunRequest](../../../../../crates/control-client/src/saved_run.rs)
already binds a start to `AuthorityRead` and refuses submission when the current host, actor or
exact grant differs. That is one existing defense against replaying a saved request at a replaced
endpoint. It does not provide a complete multi-connection draft/cache model. No three-connection
browser orchestration or cache-isolation test was observed here.

## 2. Direct calls and delegated workflow work reach the same serving owner

There are two durable execution histories for a remotely delegated operation because they record
different facts. There is no observed second serving journal for an ordinary local workflow attempt.

| User operation | Definition and durable owner | Entry/effect owner | Recovery and disclosure |
| --- | --- | --- | --- |
| Direct process or fresh-model call on B | No workflow definition or run is invented. B's serving acceptance records caller, request and exact selection. | Capability host prepares; serving transaction commits entry; selected adapter performs the operation. | B's invocation lookup/observations/output/cancel routes, exact replay and current read checks. |
| A's local workflow task | A's saved revision and runtime journal own occurrence/attempt/history. | Shared capability-host preparation; runtime final-entry/account transaction; local adapter. | A's run/attempt/result/timeline and runtime reconciliation. |
| A's workflow task delegated to B | A owns run meaning/history. B owns accepted operation and its linked foreign provenance. | B authorizes, prepares and commits serving entry before its adapter. | A retains uncertainty/allowance and imports authenticated results; B retains its acceptance/effect evidence. Connection loss does not transfer either owner. |

The direct public trace is:

1. [HTTP invocation handlers](../../../../../apps/daemon/src/http/invocations.rs) authenticate the
   bearer and call the serving owner. `PeerService::prepare_client_invocation` in
   [direct.rs](../../../../../crates/capability-host/src/serving/direct.rs) obtains the current
   authorized catalogue, creates the exact selection/profile/limits/deadline, and checks input
   authority. Preparation reserves no generation or capacity.
2. `invoke_client` derives the caller from the authenticated actor and the durable local host.
   Existing request lookup precedes fresh catalogue admission. Replay retains the originally
   accepted authorization, checks current disclosure permission and compares the canonical digest;
   changed bytes return `idempotency_conflict`.
3. Both `invoke_client` and peer `invoke` converge at
   [PeerService::accept_serving](../../../../../crates/capability-host/src/serving.rs), which calls
   `PeerExecutionStore::admit_peer_execution`. The accepted reply confirms durable admission,
   not adapter entry. Fixed workers claim that durable queue.
4. [run_claimed_once](../../../../../crates/capability-host/src/serving/worker.rs) checks deadline,
   drain, cancellation and authority; prepares the exact generation; then rechecks authority,
   claim and deadline. `mark_peer_entered` commits entry before the prepared handle is consumed.
5. Durable terminal/uncertain evidence wins over an adapter-return error. If entry may have
   happened but no terminal exists, `mark_peer_uncertain` retains that fact. Notifications and
   worker wakeups are not execution authority.

[DirectInputSelection::new](../../../../../crates/capability-host/src/adapter/direct.rs) permits
only supplied inline values and exact artifact metadata. It refuses workflow manifests, reserved
context names and workspace-value references. This is an explicit-input call, not implicit access
to a connected workflow's memory. `validate_request` prevents changing the selection after freezing.

For outgoing delegation,
[RemoteCapabilityAdapter::execute_prepared](../../../../../adapters/peer-http/src/remote.rs)
requires actual durable workflow context. Its `DelegatedAuthorization` carries real
run/revision/node/execution/attempt, target peer, exact capability/operation, nonce, expiry,
limits, controller reservation and publication ancestry. It stages the selected input artifacts,
submits the exact request and imports authorized outputs. A missing catalogue or connection does
not authorize arbitrary fallback selection. `PeerRegistry::connect/apply_catalog` imports
capability advertisements; it is not a remote revision or workflow editing API.

**Test assertions located.**

- `apps/daemon/tests/control_plane/direct.rs::direct_process_upload_replay_and_restart_have_no_workflow_records`
  uses the production process adapter, an external entry marker and an execution-only daemon.
  It checks wrong-host/digest/quota/unauthorized upload refusal, distinct caller namespaces,
  useful output, exact replay/conflict and restart without synthetic workflows.
- `apps/daemon/tests/two_daemon_peer.rs::authenticated_catalog_registers_and_disconnect_drains_remote_generation`
  exercises authenticated advertisements and remote execution turnover through two daemons.
  Its serving host is execution-only. This is loopback evidence, not physical multi-host qualification.
- The maintained `headless-cli-evidence --independent-host-only` journey checks actual daemon/CLI
  direct and delegated process/model calls, two process entries, three external model requests,
  authentic origin coordinates, output imports and an unknown final response whose originating
  reservation survives restart. Its documented reproduction is in the
  [independent-host evidence lane](../../../verification-evidence.md#independent-host-execution).

**Consequential ambiguity, not a settled defect.**
[ADR 0041](../../../../decisions/0041-published-method-invocation.md) says execution-only hosts can
consume remote publications. The source supports a narrower immediate statement: remote adapters
inherit `CapabilityAdapter::accepts_direct_inputs() == false`, and
`PeerService::catalog_entries_in_scope(..., true)` excludes them from direct discovery. Final
remote execution requires workflow provenance. Thus a direct user connected only to C cannot
currently discover B's peer-mapped method in C's direct catalogue and use C as an ordinary direct
relay. This does **not** disprove direct client→B invocation or a workflow-delegated relay with
genuine foreign provenance. The ADR may mean those cases, or it may overstate an intended route.
The observed two-daemon publication test configures both sides as workflow-enabled. No inspected
test establishes the broader execution-only consumption interpretation.

**Investigation recommendation.** Specify the desired caller/origin/transport separately, then
exercise the exact C→B case in isolation. Do not mandate publication for ordinary remote tools,
and do not infer arbitrary direct relaying from the ADR sentence. Resolving the desired outcome
belongs to 02; any implementation route belongs to the later design loop.

## 3. Reusing a definition is different from invoking a private service

**Source findings.** Three current operations have different consumers and consequences:

| Operation | Current source path | What the consumer receives |
| --- | --- | --- |
| Pinned subworkflow | `blueprint::PinnedSubworkflow`; `runtime::engine::structured::subworkflow::{ensure_child_created, advance_child_lifecycle, observe_child_terminal}` | An exact child revision/interface, inputs and a real child run with inherited authority/account/agreement. The definition is in the runtime's revision store. |
| Independent copy | `apps/daemon/src/host/commands/authoring.rs::copy` and public `Command::CopyBlueprint` | A new workflow identity and immutable source provenance, with no execution-state or authority copy. An identity-bound governing agreement refuses this convenience operation. |
| Published method | `control::PublishedWorkflowService::{prepare_publication,publish,restore,retire}` and host continuation port | A public callable generation with declared input/output contract, private starting method, governing agreement and configured service identity. Invocation does not grant internal inspection. |

`ensure_child_created` loads the pinned revision, materializes declared inputs and verifies an
existing child against its immutable creation event rather than only its current pin. It retains
the parent's exact execution basis and accepted agreement. The current mechanism does not imply
a cross-owner editable subworkflow reference merely because B's capabilities are visible at A.

`PublishedWorkflowService::prepare_publication` requires the capability's exact configured service
grant and a governed saved revision. It produces a reviewable method document; publication is
separate. `DaemonConfig::validate` requires workflow mode and enabled controller accounting for
configured publication services. The service uses a weak owner reference through the host's
`PublishedWorkflowContinuation` port; it owns no additional invocation ledger or scheduler.

The accepted public operation stores a planned internal run and canonical create/start association
before child creation. Local callers retain the association with the runtime attempt; direct or
incoming peer callers retain it in their serving record. Control uses ordinary runtime commands
to create, bind and start that exact child. Lost replies recover the same association. A pending
call releases the execution worker while durable pins and observations remain. Public success,
internal completion, agreement satisfaction, cancellation acknowledgement and a surviving deployed
service are separate facts.

**Test assertions located.**

- `apps/daemon/tests/control_plane/reuse.rs::independent_copy_is_editable_and_retains_source_without_run_state`
  checks source authorization, stale guards, separate briefs/pins, later copied edits, restart and
  governing-agreement refusal.
- `apps/daemon/tests/control_plane/published.rs::invoke_only_published_outputs_replay_retirement_and_restart`
  checks an invoke-only public result, denied internal access, replay, retirement and reopen.
- `apps/daemon/tests/two_daemon_peer/published.rs::peer_publication_creates_one_run_and_transfers_only_the_accepted_result`
  checks the ordinary remote publication route and output boundary.
- `crates/control/tests/control_service/published/recovery.rs` tests
  `each_create_bind_start_commit_boundary_recovers_one_child_after_reopen`,
  `cancellation_after_lost_create_reply_settles_without_starting_the_child`,
  `cancellation_after_child_start_has_one_linked_control_action_and_no_work`, and
  `lost_public_terminal_commit_does_not_repeat_a_settled_internal_method`.
- `published/constraints.rs` tests exact active-call pinning across new generations, account
  settlement across reopen, self-call refusal and inherited ancestor depth ceilings.
- `published/retirement.rs::queued_serving_acceptance_survives_retirement_and_reopen_before_entry`
  distinguishes new admission refusal from preserving an already accepted queued call.

**Inference.** A collapsed visual call can represent these operations, but the client still needs
their actual owner, public rights and recovery identity. Conversely, source names do not prove
they all need distinct pages or visible node types. Neither conclusion is selected here.

## 4. Installation lifetime and permission to edit are independent

**Source finding.** [ManagedResources](../../../../../crates/capability-host/src/managed.rs)
owns lifecycle orchestration over `ManagedResourceStore` and a mechanism-specific `ManagedPlatform`.
`execute` uses a trusted authenticated caller and actor-scoped exact request. Durable intent and
closed admission precede platform work; `drive` records each observed step against its transition.
`recover_startup` resumes saved identities rather than creating a same-name replacement.

The persistent resource path lives in
[redb managed uses](../../../../../adapters/redb-store/src/managed/uses.rs) and
[managed execution transactions](../../../../../adapters/redb-store/src/managed/execution.rs).
An accepted invocation acquires exact generation holds; a physical writer additionally needs an
editing claim. `transfer` requires exact accepted parent/child association, claim generations and
current authority. It requires parent quiescence before transfer and child quiescence before return.
`release` refuses release without physical stop evidence and settled child ownership.
Runtime terminal status alone does not prove a process stopped.

The composed success path matters: publication wrappers have no external writer and can record
`NoExternalEntry` evidence; their exact accepted physical children receive editing in turn while
the wrapper keeps lifetime protection. They do not consume the execution worker needed by that
child. The failure path preserves an entered child's hold after cancellation/restart when physical
stop is unknown. Authorized resolution fences the exact use; it does not turn its old effect into
a known success or known failure.

**Test assertions located.**

- `adapters/redb-store/tests/contracts/managed.rs::journal_acceptance_and_resource_hold_commit_or_refuse_together`
  checks atomic acceptance/hold acquisition, conflicting writer/maintenance refusal, and retained
  use even after a successful runtime terminal lacking stop proof.
- `exact_child_handoff_restart_fencing_and_authorized_return_never_have_two_editors` covers both
  ordinary and cancelled cases with exact claims.
- `crates/control/tests/control_service/published/managed.rs::publication_hands_editing_to_exact_internal_writers_and_releases_every_hold`
  observes two legitimate physical child entries, editing return between them and no final leak.
- `lost_child_stop_proof_survives_cancel_and_reopen_without_returning_editing` checks retained
  suspended parent/entered child, refused unsafe parent resumption, then authorized resolution.
- `adapters/managed-linux/tests/lifecycle.rs::every_platform_boundary_recovers_the_original_intent_without_duplicate_service`
  and `intent_and_each_result_commit_survive_before_after_commit_faults` locate deterministic
  intent/result-loss coverage. They are not new physical Podman results.

**Public-route gap for S25.** `ManagedAction` currently offers prepare/apply/inspect/start/stop/
update/preserve/remove/recover/evaluate/publish/evidence/handoff/return/resolve. No detach or
release-management operation appears in that closed enum. `DataDisposition::Preserve` retains
data when removal removes owned service/configuration. It does not promise that an active owned
service becomes independently administered. An externally attached service already has another
owner; that is a different case. A future detach outcome therefore needs explicit definition and
obligation handling, or an explicit statement that it is not selected. Container survival while
Milkdrift is absent cannot establish that route.

## 5. Authority, context, prospective adaptation and retained history

The same route serves humans and service/AI actors because authentication supplies actor/grant
facts and ordinary control commands carry intent. This says nothing about arbitrary agent quality.
Model output cannot directly execute a mutation by being persuasive prose.

| Boundary | Inspected owner and meaningful consequence | Discriminating test source |
| --- | --- | --- |
| Starting/adopting work | `runtime::engine::authority::{bind_execution_authority,validate_revision_authority}` freezes the start basis and walks reachable task/reducer/subworkflow/repeat requirements. Adoption revalidates under that basis. | `crates/runtime/tests/durable_runtime/authority.rs`: revocation before resolution, after claim, and after adapter entry; the last preserves earlier success while blocking later work. |
| Proposed revision | `ControlService::{submit,decide_proposal,apply_proposal}` constructs and validates a candidate, evaluates delta/risk and delegates accepted changes to runtime. | `apps/daemon/tests/control_plane/repair.rs::final_model_review_repair_preserves_history_context_and_approval_guards` exercises the public convenience path; it is narrower than arbitrary graph repair. |
| Future adoption | `runtime::engine::reconciliation::plan_revision_adoption` requires an active non-draining run, checks inherited agreement/adoption allowance, compares old/new definitions and records a prospective plan. | `structured_runtime/reconciliation.rs::prospective_revision_adoption_is_persisted_actionable_and_stale_safe`; `reconciliation/retired_history.rs::compacted_retry_history_still_blocks_retrospective_side_effect_rewrite`. |
| Causal context | `runtime::context`, `source/discovery`, `selection` and `retained` freeze identity, source boundary, provenance, selection/omission and bounds. Materialization loads only selected references. Direct inputs use the separate explicit selection above. | `structured_runtime/causal_context_production.rs::reviewer_receives_frozen_causal_evidence_without_private_sibling_transcript`; `context_enforcement.rs` tests required evidence and model-session agreement before claims/recovery. |
| Agreement protection | `blueprint::validate_agreement_adoption` and runtime's inherited-agreement check prevent the editable region from changing its governing contract or pinned child obligations. | `crates/blueprint/tests/agreements.rs::{useful_investigation_and_repair_preserve_the_agreement_and_roundtrip,protected_nodes_dependencies_terminal_and_scope_cannot_change,indirect_data_rewiring_and_new_capability_requirements_refuse}`. |
| Protected effect | `capability-host::managed::protected::{evaluate_candidate,publication_candidate}` records an incomplete exact candidate before calling the configured verifier, then requires retained trusted evidence for exact target/generation/agreement/configuration/bytes. An uploaded report cannot become this private evidence. | `adapters/managed-linux/tests/lifecycle/protected.rs::{failed_forged_stale_and_revoked_evidence_cannot_publish_and_replay_is_exact,policy_change_at_final_entry_and_lost_effect_response_survive_reopen,evaluation_commit_loss_retains_unknown_or_exact_result_without_rerunning}`. |
| History/replay | Runtime journal facts and receipts persist; projections compact active state. Daemon historical reads use occurrence/revision anchors rather than treating the latest revision as the meaning of older attempts. | `apps/daemon/src/host/attempts/history/tests.rs`; runtime `reconciliation/retired_history.rs`; `control_plane/durability.rs::daemon_command_idempotency_restart_and_stale_conflict`. |

**Inference.** A diagram with a verification node is weaker than an effect prerequisite enforced
at publication, and artifact integrity is weaker than trusted verification. Current source has
both mechanisms for distinct reasons. Any later unification must explain which finite verifier
claim is being preserved; generic correctness/security is not established by the finite checks.

Current ordinary model authoring is deliberately narrower than the runtime's generic graph.
`NodeKind` includes generic capability `Task`, branch, fork, join, reducer, repeat, wait,
signal-wait, pinned subworkflow and terminal forms. The public convenience editor covers its model
workflow subset; it is not evidence that all structured meanings already have a human editor.
Raw ordinary blueprint/proposal routes retain richer executable forms. S05/S06/S28/S29 need an
interaction account beyond proving that these types deserialize.

## 6. The actual browser boundary

**Source findings.**

- `DaemonConfig::validate` in
  [config/compile.rs](../../../../../apps/daemon/src/config/compile.rs) refuses every non-loopback
  bind. `main` binds a Tokio TCP listener and `http::serve` uses `axum::serve`; no daemon TLS
  listener or static frontend route was observed. Non-loopback HTTPS clients therefore depend on
  an external deployment mechanism; this source does not provision it.
- [http::router](../../../../../apps/daemon/src/http.rs) explicitly omits CORS. Its layers are body
  limit, panic handling and tracing; there is no CORS/preflight configuration. Every ordinary
  route, including version/readiness, uses bearer authentication.
- [response::bearer_header](../../../../../apps/daemon/src/http/response.rs) reads only
  `Authorization: Bearer …`; cookie or URL token fallback was not observed. `AuthRegistry` rereads
  explicit secret references per request, compares digests in constant time and refuses ambiguous
  matches. It derives cursor keys from the credential. JSON does not supply actor identity.
- [ControlClient](../../../../../crates/control-client/src/lib.rs) is a native Rust `reqwest`
  client. It disables redirects, refuses non-loopback HTTP, bounds safe-query retries, and sends
  mutations once. Its artifact reader checks bounded `Content-Range` and requires complete-byte
  digest verification by the caller. Those headers would also need browser-readable exposure in
  a future cross-origin route.
- `VersionRequest`/control protocol enforce the exact current version, presently 2.20.
  Negotiation is authenticated. A page or client cannot infer compatibility from successful TCP.
- [SSE handlers](../../../../../apps/daemon/src/http/streams.rs) reauthenticate and reauthorize
  every bounded poll. Run cursors continue against durable run history. Health/capability feeds
  bind the actual router's random incarnation; after restart or another feed instance they emit
  `ResyncRequired`. Capability observations are complete replacement snapshots, including empty
  sets, not additions to an eternal catalogue.
- `ControlClient::subscribe` decodes bounded frames, suppresses old positions and reconnects with
  its last decoded cursor. It stops on resync/closing/nonretryable failure. Consumers own the
  overall reconnect/deadline policy. Its resume point means decoded observation, not durable
  application acknowledgement. The query cursor and SSE event ID are not a credential.

**Primary platform evidence retrieved 2026-10-10.** The
[WHATWG Fetch Standard](https://fetch.spec.whatwg.org/#http-cors-protocol), shown as updated
2026-10-06, defines cross-origin response sharing and preflight. Authorization is not a
CORS-safelisted request header. The
[EventSource interface](https://html.spec.whatwg.org/multipage/server-sent-events.html#the-eventsource-interface)
accepts a URL and `withCredentials`; it offers no arbitrary Authorization-header option.
These are normative API constraints, not evidence about an exercised browser/version here.

**Inference.** A standalone app at another origin cannot simply use today's raw daemon as if it
were the native client: bearer requests need successful cross-origin authorization/preflight, and
native EventSource cannot directly supply the required header. A fetch-based SSE reader is a
candidate transport, but still needs origin/deployment support and bounded protocol handling.
Same-origin operator proxying and explicit daemon CORS are viable alternatives to investigate;
neither is selected here. Private-network permissions, HTTPS/mixed-content behavior, credential
storage, proxy custody, server-side rendering isolation and several independent remote origins
remain untested. A Tauri wrapper would require its own bounded transport/provisioning design.

**Existing regression locations, not browser proof:**

- `control_plane/independent_client.rs::json_client_authors_runs_recovers_downloads_and_copies_without_private_builders`
  drives a real daemon binary through public JSON, but its transport is `reqwest`.
- `control_plane/streams.rs::capability_streams_isolate_authority_in_both_subscription_orders`,
  `capability_stream_closes_when_credential_moves_to_another_actor`, and
  `process_local_stream_cursors_resynchronize_after_restart_but_run_cursor_resumes` exercise the
  current Rust client and daemon protocol.
- `host/tests/live_streams.rs::capability_stream_removal_converges_with_fresh_and_reconnected_clients`
  and `capability_cursor_cannot_attach_to_another_http_feed_on_the_same_host` cover complete removal
  and actual feed-instance binding.
- `control_plane/inputs_cli.rs::actual_cli_lost_start_reply_and_wait_deadline_recover_without_reexecution`
  distinguishes lost local observation from daemon work lifetime.

**Investigation recommendation.** A bounded browser-origin probe should first establish a normal
authenticated read, current raw preflight refusal and authenticated streaming under one declared
deployment arrangement. Record browser/OS, origins, proxy configuration if any, network requests,
resync and a second independent owner. Do not count native JSON success as this proof or solve the
gap by putting bearer secrets in URLs.

## 7. Consequential disagreements and alternative explanations

| Question for 02 or later factual work | Established ground | Plausible explanations still open | Dependent claim that must remain conditional |
| --- | --- | --- | --- |
| Does standalone mean arbitrary browser-to-host origins at first delivery? | Raw daemon is loopback-only, bearer-only and has no CORS/TLS/static UI composition. | Intended operator-provisioned same-origin deployment; missing direct-origin daemon capability; need for a separately bounded broker. | S01/S07/S18 browser reachability and credential UX. A native CLI test cannot decide the product scope. |
| What does execution-only consumption of a remote method mean? | Remote adapter is workflow-only for direct discovery; two-daemon publication test uses workflow mode. | ADR means direct connection to the remote owner; legitimate workflow-delegated relay; overly broad wording for a missing route. | C-as-relay claims and any design that treats a peer as an interchangeable user connection. |
| Must an editable reusable workflow be remotely discoverable without publication? | Peer registry exposes capability generations; pinned subworkflow loads a local exact revision. | Desired UI connects directly to each workflow owner; explicit import/copy is intended; a remote method-reference operation is missing. | S07/S09 ownership and cross-owner editing/reuse. Current naming does not choose the answer. |
| What does operator independence require for managed work? | Persistent services can survive operations; action enum has preserve/remove but no detach. | Persistence alone serves the need; external attachments serve it; active release from management is a missing valuable outcome. | S25 handoff/de-management and lifetime/credential migration. |
| Is a general graph UI already supplied by headless authoring? | Runtime supports richer generic/structured forms; model convenience authoring is intentionally narrow. | A general public import/proposal interface is sufficient backend; additional public edit operations are needed for usable thin clients. | S05/S06/S23/S28/S29 editor scope and machine-authoring equivalence. |
| Can strong existing refusal tests establish combined useful progress? | Publication/managed tests show both progress and lost-stop handling; many external/platform outcomes remain separately qualified. | Current boundaries compose for selected cases; uncovered three-owner, authority-change and frontend cases expose new constraints. | S21/S26 cannot be declared generally solved from isolated successes. |

The history suggests narrower causes than “the architecture is broken.” `0038` added independently
useful hosting while retaining local workflow ownership. `0041` added explicit service authority
and a recoverable child rather than using publication as every remote transport. Later live-feed
changes (`3059745`, `4e3c1db`, `e3adddf`, `18db4b2`) repaired authority isolation, restart invalidation,
complete removal and feed-instance binding. `c016cd3` records the integrated result. This chain
explains why a frontend that merely appends capability rows or reuses process cursors after restart
would contradict the current contract. It does not prove why any original product goal was chosen.

## 8. Scenario coverage and next evidence

This is a source-coverage contribution to the coordinator's system dossier, not a second scenario
authority. “Current source path” below does not imply this session executed the scenario.

| Scenarios | Current source coverage in this note | Unresolved part |
| --- | --- | --- |
| S01–S02 | Public JSON author/save/input/run/replay/results and stable identity; independent client test. | Zero-connection UI and browser transport; human use observation. |
| S03–S06 | Prospective repair/authority/context, generic and structured graph owners. | General human/agent authoring UX, advanced diagram round trip and all combined edits. |
| S07–S08 | Distinct roles, owners, peer catalogue/delegation and direct calls. | Three independent client connections; exact relay interpretations; non-loopback browser proof. |
| S09–S10 | Pinned child, copy, published service, invoke-only results and accepted-call retirement/recovery. | Cross-owner editable reuse meaning and complete UI disclosure. |
| S11–S13 | Current auth/cursor isolation, direct/managed lifetimes and uncertainty. | Browser cache cleanup and a real multi-origin permission transition. |
| S14–S17 | Agreement/effect guards, repair, context/omission/history; learning is identified in canonical docs but lightly inspected here. | New learning value claims require learning-owner evidence; no independent full evaluation trace in this note. |
| S18–S20 | HTTP/browser gap, exact-current versions, independent edit/invoke/read permissions. | Supported initial deployment choice and migration of a future redesign. |
| S21–S24 | Exact publication/managed handoff, authority recheck, causal isolation and owner-qualified recovery facts. | Three-owner composed interruption, full same-name UI case and transfer-specific privacy review. |
| S25–S27 | No detach action, cumulative ownership, explicit cancel versus disconnected observation. | Desired handoff semantics, continuous-project interaction and browser logout/cache behavior. |
| S28–S30 | Existing proposals/causal artifacts/recovery and immutable definitions are usable ingredients. | This planning workload, notation round trip and a real supported upgrade program remain unproved. |

Useful focused checks were sent to the coordinator. Build real binaries before tests that launch
them; serialize Cargo with other work. Recommended existing filters are:

```sh
cargo test --locked -p milkdrift-daemon --test control_plane independent_client::json_client_authors_runs_recovers_downloads_and_copies_without_private_builders
cargo test --locked -p milkdrift-daemon --test control_plane inputs_cli::actual_cli_lost_start_reply_and_wait_deadline_recover_without_reexecution
cargo test --locked -p milkdrift-daemon --test control_plane direct::direct_process_upload_replay_and_restart_have_no_workflow_records
cargo test --locked -p milkdrift-daemon --test control_plane published::invoke_only_published_outputs_replay_retirement_and_restart
cargo test --locked -p milkdrift-daemon --test two_daemon_peer published::peer_publication_creates_one_run_and_transfers_only_the_accepted_result
cargo test --locked -p milkdrift-control --test control_service --all-features published::recovery::
cargo test --locked -p milkdrift-control --test control_service --all-features published::managed::
```

The maintained actual-binary independent-host command is:

```sh
target/debug/headless-cli-evidence --independent-host-only \
  --daemon target/debug/milkdrift-daemon --cli target/debug/milkdrift
```

It needs freshly identified daemon, CLI and evidence binaries. An old executable under `target`
is not automatically the inspected HEAD. Preserve source/build identity and report the raw artifact
location with any new results. These focused checks discriminate useful progress, refusal and
recovery; they are not the full gate or a renewed hardware/browser qualification.

Documentation verification for this contribution: source/consumer/test consistency review,
Markdown review, existence checks for every local Markdown link, and
`git diff --no-index --check /dev/null` against each newly written contribution passed. The
coordinator retains responsibility for the repository documentation contract target and
integrated `git diff --check`. No production files, shared index or commits were changed by this
session.
