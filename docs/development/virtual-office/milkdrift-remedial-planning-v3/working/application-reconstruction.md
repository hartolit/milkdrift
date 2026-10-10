# Application construction, clients and configuration

Rowan-20261010, coordinator `/root`, 2026-10-10. This is the application portion of the
reopened investigation, against production source unchanged from `908e789` through `94ec7c6`.
It distinguishes source inspection from the separately recorded runtime observations. It does
not approve implementation or claim every application path was exercised.

## Actual coverage and original pressure

| Owner/package | Purpose and important relationships | Previous depth demonstrated | Reopened examination and limit |
| --- | --- | --- | --- |
| `apps/daemon` | Authenticates network callers, composes configured owners, translates public commands and retains their exact results. Consumes control, host, runtime, storage and adapters. | Public journeys, browser refusal and selected commands; broad architectural retention was not independently justified. | Source/consumer/history comparison of authoring, receipts, configuration, service inspection and role composition. Learning/provenance and resource operations have the separate Elin/Faris traces. Transport fuzzing, every read projection and all configuration combinations were not newly audited. |
| `apps/cli` | Operator commands plus a client of the shared daemon; configuration/storage administration have deliberately local responsibilities. | Public CLI routes observed. | Inspected command dispatch and shared prompt-sequence use; no parallel workflow executor found in these routes. Full shell usability and platform behavior remain inherited evidence. |
| `crates/control-protocol` | Independent bounded wire documents for all clients; depends on contracts, not runtime. | Endpoint map and independent JSON client. | Authoring DTOs, envelope/version/bounds and command/result distinction examined against competing domain-type exposure. Whole decoder security qualification was not repeated. |
| `crates/control-client` | Typed HTTP/SSE transport and exact saved request preparation; CLI/evidence use it. | Lost-reply and direct invocation routes traced. | SavedRunRequest and serving preparation inspected, including differing grant checks. Transport retry/stream mechanisms mapped against browser requirements, not newly browser-qualified. |
| `crates/prompt-sequence` | Pure importer for a useful coding/verification/review dialect; both CLI and daemon consume one compiler. | Named as a construction dependency. | Compiler, acceptance helper use, import tests and consumers examined. Its domain-specific source is justified; its emitted canonical representation must change with the selected source model. |
| `crates/contracts` | Shared unambiguous bounded JSON/canonicalization and constructor-based deserialization; many independent schema owners consume it. | Package mostly inherited. | Read public implementation/embedded examples and selected checks. It owns bytes/structure, not workflow semantics. No new parser soundness claim. |
| `examples/adaptive-slotbook` | Concrete workload and independent result oracle, consumed by evidence drivers. | Real controlled evidence cited. | Joined with Elin's driver trace: application result/fixture remains valuable, workload-specific driver orchestration is not a general product planner. Not a production owner to retain merely because it exists. |

Source anchors are [authoring](../../../../../apps/daemon/src/host/commands/authoring.rs),
[private graph compiler](../../../../../apps/daemon/src/host/commands/authoring/graph.rs),
[repair](../../../../../apps/daemon/src/host/commands/authoring/repair.rs),
[wire authoring](../../../../../crates/control-protocol/src/authoring.rs),
[prompt-sequence](../../../../../crates/prompt-sequence/src/lib.rs),
[CLI](../../../../../apps/cli/src/command.rs),
[saved requests](../../../../../crates/control-client/src/saved_run.rs), and
[contracts](../../../../../crates/contracts/src/lib.rs). The package audit is joined in the
[system dossier](system-dossier.md); depth in one command is not package-wide approval.

The narrow model editor arose in `78a8f6a` and
[ADR 0044](../../../../decisions/0044-public-workflow-authoring.md). It deliberately supplied a
finite public convenience over existing mutations. Subsequent edge identity and output/input
corrections (`040b528`, `98b664f`, `aac1abd`, `4f3ef27`) show the cost of preserving generated
structure through edits. They do not show that a model-only linear method was the enduring user
need. The pure prompt-sequence importer has a different purpose: trusted-process stages, repository
profiles, verification and review. Keeping that input dialect need not keep a second semantic owner.

## Same authoring task, three complete placements

Use Delta's unchanged investigate-and-repair packet, including prioritized choice, two isolated
reviewers, failed review as evidence, prospective repair and a human wait. Independently expected:
a person chooses order, inputs, capabilities, output obligations and authority; they do not need to
invent paired edge identifiers or reproduce acceptance helpers. Changing one future probe must
preserve completed investigation and expose its revised result mapping. A second client must see
the same semantic method without access to browser memory.

**Current route.** Generic `BlueprintDraft` accepts a full mutation batch, so the public API is not
incapable of rich graphs. But `BlueprintEdit`'s convenient operations describe model steps. The
private `ModelWorkflow::read` recognizes a generated Fresh/direct-input graph, follows reserved
accept/gate/hold structure, rebuilds it and requires equality. An arbitrary valid workflow cannot
be edited by pretending it is that template. Generic clients must construct complete raw ports,
bindings, edges, child interfaces and outputs themselves. Repair further requires the particular
generated completeness hold. The ordinary product therefore exposes an unequal authoring boundary:
convenient finite model steps, or client-authored executable internals.

**Complete smaller correction.** Move a pure graph construction service into blueprint, with
compound choice/fork/join/call/wait operations; control owns authorized load/validate/save and exact
base guards. Protocol carries intent DTOs; CLI, agents and Svelte use one daemon route. The existing
model template and prompt sequence call this builder. It makes the packet practical and removes
private daemon ownership. Its cost is that each construct has an authoring form, a redundant stored
graph shape and a validation/ownership reconstruction. A promise that the frontend will wire the
parts is not this complete alternative.

**Structured source with one execution lowering.** Store stable-ID Sequence/Conditional/Parallel/
Repeat/Call/Task/Await/Return structure as the new immutable definition. Input references and region
results are semantic; layout is not. Pure complete edits and lowering belong to blueprint. Control
loads exact source, checks authority and saves; runtime consumes the checked plan and owns occurrences.
The authoring packet can express exactly the product choices above. The graph canvas becomes a
projection of that source, as does an equally complete outline. The model and prompt-sequence
compilers emit this source through the same validation, not through the daemon's private recognizer.

The last alternative is selected after Delta/Elin's actual cross-review. Cross-sequence data references
are possible in both proposed forms and therefore do not justify retaining edges as the authored
truth. Structured source avoids repeated inference of control ownership. The smaller correction's
strongest advantage is preserving a mature representation with less conversion risk; this remains
substantive dissent. Neither comparison establishes measured human or model authoring superiority.
The full legacy plan and conversion contract is in [Delta's comparison](structure-comparison.md).

This choice **reconstructs a core representation**, while retaining execution history, authority,
effect and value owners for positive reasons. It is neither a cosmetic editor over today's graph
nor replacement of every implementation because one representation was wrong.

## Concrete responsibilities and deletion

`blueprint` owns new immutable structured source, complete semantic editing, validation, stable
source-to-plan mapping and both version-specific lowerers. These are pure functions. `control`
owns authenticated application operations and model-response admission, reading current owner facts
through its existing/narrow ports. It may call the pure compiler; it must not reach into daemon
modules. Protocol owns external DTO/version/bounds. Daemon owns authentication, wire translation,
composition, application receipts and bounded projections. No new generic framework crate is needed.

Delete new-write dependence on daemon's `authoring/graph.rs::ModelWorkflow` and reserved `author.`
shape recognition when its consumers migrate. Retain versioned legacy read/conversion fixtures and
exact accepted old commands. Old model-only convenience commands can translate to the new complete
owner for representable successors; requests requiring a legacy edit that cannot be proved equivalent
refuse before mutation. Do not leave the old private compiler available as an alternative new writer.

Keep prompt-sequence's useful input dialect and pure crate. Its current acceptance builder already
uses control's `result_acceptance_task`/gate helpers; claiming two implementations of acceptance would
be false. Change its output to the new source and retain source-stage/element mapping. CLI offline
compile and daemon import continue using that one compiler. Its test assertions should preserve
verification/review consequences, not require obsolete port spellings or extra graph nodes.

Capability choice remains a control/host fact. Current `authoring/edits.rs::selection` checks the
filtered catalog, exact profile and locality/trust/effect requirements. Keep that narrowing in the
shared application boundary; Svelte presents the offered options and consequences. Neither the UI
nor a model response can create truthful current descriptor or grant facts.

## Receipts: actual duplication versus distinct promises

The daemon [receipt owner](../../../../../apps/daemon/src/host/receipts.rs) binds original actor,
exact grant and canonical external envelope, including guards, to a complete response/refusal.
Layout and receipt commit together. Definitions are immutable and preflight bounded result capacity
before retention. Runtime/control commands may commit before the application receipt; the code
explicitly returns uncertain if it cannot retain that result. This is not one global transaction.

For StartRun, [runs.rs](../../../../../apps/daemon/src/host/commands/runs.rs) creates, then starts.
`internal_control_id` derives stable actor/request/phase identities; control derives stable runtime
phase identities. Runtime [command receipts](../../../../../crates/persistence/src/journal/receipt.rs)
separate audit document from semantic retry intent, excluding delivery time and planning sequence
where the runtime owns replanning. The external receipt binds the complete external request. Thus
the two levels make different promises even when a particular response is only a run sequence.
Application layout/discovery effects also have no corresponding runtime run. Retain the external
receipt rather than copying runtime semantics into browser storage or dropping exact-envelope replay.

A credible unification would make every public operation a domain-neutral atomic command record,
with runtime, publication, layout and definition actions in one transaction protocol. That would
remove some response wrappers, but would couple external envelope compatibility to all owner
transactions and still need multi-step remote/child continuation. It does not eliminate independent
accepted facts. The inspected costs do not justify that new universal transaction owner.

This is not a claim that the existing gap is solved for every future compound operation. Assistance
start and response admission must save their exact internal command material and accepted associations
before effects, and recover a committed proposal after reporter loss. Merely choosing deterministic
IDs while reconstructing different command meaning is insufficient. Faris's cross-review states the
required fault boundaries; the chosen session design must use them explicitly.

Existing [durability tests](../../../../../apps/daemon/tests/control_plane/durability.rs) establish
retained accepted/rejected replay and restart for their fixtures. They do not independently simulate
every crash between underlying acceptance and external receipt. Required future assistance tests
fault each such boundary and inspect one accepted child/proposal, rather than just replaying a
request whose final receipt was already committed.

## Read contracts and avoiding a false finding

`ListMethods`/`InspectMethod` in
[publications.rs](../../../../../apps/daemon/src/host/commands/publications.rs) are administrator
commands with retained snapshots and `AdministerCapabilities`/`method.inspect` authority. List first
requires unqualified administration, then checks each record. These are not the invoke-only service
catalog. The prior R-A05 suggestion to universally filter this command conflated two contracts.

Retain those exact historical receipt semantics. The ordinary service chooser uses the existing
filtered invocation catalog and public input/output contract; it cannot expand private method
internals. Management uses a new bounded current publication-read projection checked per record under
current administration rights, with an opaque scoped cursor and truthful empty pages/continuation.
Unauthorized records do not become names or counts. This is a read projection of `PublishedMethodStore`,
not another publication owner. Existing administrative snapshot commands stay explicitly labeled as
snapshots for clients requiring a retained observation; new UI refresh uses the query. The distinction
is selected here, not left as an implementation question about which operation exists.

Similarly, saved ordinary StartRun requires exact saved `AuthorityRead` equality; direct serving has
its own original-grant/current-inspection replay rules. A generic browser 'retry under current token'
would merge different promises. Keep the previously reviewed operation-specific recovery contract.
Immutable revision save remains a branch: two valid siblings based on the same revision may both
save. A latest-head compare-and-swap must not be invented by the UI.

## Configuration and shared bytes: retention with reasons

The daemon's TOML config is operator input compiled once into an immutable validated `DaemonPlan`.
`config/compile.rs` checks version, roles, paths, activation and loopback policy; adapter preparation
later checks exact bytes/generations and live entry. These validate different facts at different
times. Making a general mutable registry the configuration truth would add live-reload transitions,
secret refresh, drain/rollback and generation races without a demonstrated product need. Retain the
startup plan; role changes still use supported shutdown/reopen with outstanding obligations intact.
Operator host setup is not an ordinary method editor's form. Browser endpoint/CORS additions need
the explicit bounded configuration contract already proposed, not an implicit deployment change.

Contracts' canonical JSON preserves array order while sorting object keys, rejects duplicate input
keys, and bounds shape before/after parse. These are shared mechanical rules with real independent
schema consumers. Moving them into capability would reverse the independent protocol dependency;
copying them would risk divergent fingerprints. Retain this narrow crate and its owner-supplied
limits. It must not grow into a generic workflow/common-policy module. Reusing a numeric document
bound does not establish a semantic ownership defect without a conflicting consumer need.

## Change-cost comparison and supported intermediate product

Future demand: add a specialist only for schema changes, pass an explicit missing-result outcome to
the judge, then amend the not-yet-started specialist after criticism. Current clients must coordinate
fork/child/port/output wiring and the private editor may refuse the definition. The complete graph
builder centralizes that work but still derives containment from wiring. Structured source changes
one Conditional child/result inside Parallel, one explicit reference and the compatible future
frontier. It still changes validation, lowering, runtime result/repair consumers and public views;
the recommendation does not claim these disappear. Their shared owner is decided, rather than
asking each client to infer it.

Implement complete source/read/lowering/repair/public operations before exposing the advanced edit.
An early real Svelte client may inspect old work and execute complete supported basic new work;
it must not offer advanced gestures that only store layout. The original source/byte identities,
accepted request results, running occurrence pins, unsettled effects and resource/account facts stay
authoritative through upgrade. In-place rewriting of old revisions is forbidden. Legacy source that
cannot convert equivalently remains inspectable/runnable with precise refusal for conversion. Under
U19 it also retains native graph edits/repair through the shared owner; explicit new work with selected
old evidence is not an equivalent substitute for that repair. No automatic copy erases an effect.

The closed adoption units are pure source/editor ownership, protocol/control authoring and response
admission, importer/client migration with deletion, current management reads, and the production
client. Each must list its actual source readers/writers and focused fault oracles in the implementation
program. This investigation selects these boundaries; function naming and local factoring remain
implementation choices. No production changes or new runtime acceptance are claimed here.

## U19 correction — retain supported source operations, remove private ownership

Rowan-20261010, 2026-10-10, coordinator agent pseudonym. The earlier R-only writer recommendation
confused a canonical owner with a single admitted representation. Existing `ConstructBlueprint`,
graph import, mutations and ungoverned independent copy are product operations; removing them merely
to force structured-source adoption has no demonstrated runtime necessity. The selected revision
keeps their versioned bounded contract. Each immutable revision has one source, GraphNative or
WorkflowProgram, and both lower through blueprint to the same checked execution plan.

The exact correction to the deletion list matters. Delete daemon-private `ModelWorkflow` ownership
of semantic construction and recognition, not every pure graph helper required to preserve old
AddModel/EditModel behavior. Move those helpers under the same blueprint source owner; shared control
admission dispatches against the actual source family. Existing exact shape refusals need not become
unbounded inference about arbitrary graphs; generic graph mutation remains available for rich graphs.
New convenience genesis defaults to Program while the explicit supported graph API remains. Old exact
command receipts retain bytes/results without rerunning either constructor.

No new graph-eligibility catalog, import exception or invented historical timestamp is justified.
Current authority decides graph creation/mutation. Copy still refuses identity-bound governed methods;
conversion cannot become a way to evade that refusal. Native source-family fingerprints remain stable
when runtime consumers move to a common plan, so normalization cannot accidentally classify every
unchanged legacy task as changed. Exact selected proposal base is carried explicitly; sorted merge
parents are ancestry, not proof of which base the proposer edited.

The consequence is real maintenance of two versioned source validators/lowerers and client adapters,
not two schedulers or independent runtime truth. Corrected graph M would avoid this cost; R earns its
default position through complete structural operations/results and future-change comparisons, not
a claim of universal graph convertibility. Retirement remains a later reviewed migration with active,
uncertain, reusable and external-client evidence, no automatic cutoff in P00–P09. See the substantive
U19 alternatives and state matrix in the agency/structure/execution reports and adoption plan.
