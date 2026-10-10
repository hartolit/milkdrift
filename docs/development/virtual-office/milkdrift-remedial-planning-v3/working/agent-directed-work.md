# Agent-directed work: independent reconstruction

Elin's reopened investigation at `ff818e5`. This is a recommendation under review, not approved
implementation. It uses the supplied reopening request, original selected
[intent excerpts](../intent-source-excerpts.md), current source and history. No other fresh
reviewer's recommendation was read before this initial position was recorded. Source inspection
is evidence of current behavior; the proposed operations below are modeled, not executed.

**Current disposition after review:** U17/U18 supersede the initial finite ceiling and overly
restrictive evaluation assumptions preserved below. Automatic bounded ongoing work and honestly
labeled human knowledge assessment are approved product directions, not production authorization.
The recommendation is canonical structured method source plus a continuous `WorkCommitment`
coordination owner within the existing control/controller responsibility. The complete C design
and its final refinements below own that recommendation; “session” is its earlier working name.
Knowledge need not be an executable method. No positive evaluation itself adopts or publishes it.

## Initial position before comparative review

Milkdrift has a strong admission and durable execution substrate, but the current product does not
own the complete conversion from a goal or criticism to an agent-produced useful revision. The
controller lifecycle checks limits around a caller-supplied repeat body; it does not decide what
that body should inspect or how its model response becomes a proposal. Current evidence drivers
provide that orchestration. Calling those demonstrations a product goal-to-plan path overstates
what was implemented.

My initial recommendation is a bounded assistance operation in control, compiled to ordinary
runtime work and using the existing proposal/reconciliation boundary. Its packet preparation,
response authentication and binding must become product responsibilities. It must not require an
external driver to manufacture authenticated provenance or reconstruct interrupted orchestration.
I will compare this route against the complete existing external-client composition and a
separate durable assistance session before settling the recommendation.

The learning implementation also has a material product restriction: it compares repair counts
for protected managed-publication methods. It is not a general evaluator of research or planning
methods. Whether a lesson belongs to that finite profile must be explicit; successful Slotbook
evidence cannot establish S28's broader learning outcome.

## Coverage correction and examined purpose

The prior dossier named these families and inherited tests, but did not compare their ownership
or complete producer/consumer relationships. The reopened review now has the following depth.
This is not approval of packages outside the stated paths.

| Package or subsystem | Valued outcome; necessary rule | Current mechanism and consumers | Review depth now; remaining limit |
| --- | --- | --- | --- |
| `control`: proposals and controller | An agent changes future work under explicit authority; a model's claim is not an accepted decision | `ControlService::submit`, `WorkflowProposalDocument`, `WorkflowControlAdapter`; daemon proposal commands, runtime lifecycle callback, evidence drivers | Source, caller search, history, hostile-input and lifecycle test expectations examined; three complete placements compared below. No current product planner found. |
| `runtime`: context | A fresh worker receives relevant evidence without acquiring a sibling's private material; exact prior selection survives retry | `DurableContextCandidateSource`, `CausalContextBuilder`, retained manifests; host materialization and model adapters consume them | Discovery, explicit artifacts, join/subworkflow exposure and selection test expectations examined; compared with global session memory and explicit packet-only inputs. Scheduler as a whole belongs to the other review. |
| `workspace` | Files and accepted evidence can outlive one process without losing their producer; mutable branch state has one editing owner | `WorkspaceValueReference`, `ScopeLineage`, `ArtifactOwner`, `ArtifactProvenance`, `CandidateEvaluation`; runtime, host, persistence and managed publication | Logical state/artifact ownership and candidate contracts examined. Managed filesystem transitions mapped through their consumers, not independently requalified on hardware. |
| `model` | Models remain interchangeable external workers receiving exact permitted evidence | `ModelTaskRequestDocument`, `ContextManifestDocument`, `ModelResponseDocument`, explicit continuation companion | Relevant contracts and proposal-reader integration inspected. Provider framing/performance remain inherited evidence, not a new interoperability review. |
| `control::learning` plus daemon learning orchestration | Retain a proposed reusable improvement, compare independently, and preserve a negative result | `LearningRequest`, `LearningDeclaration`, journal-derived comparison and separate publication | Types, daemon declaration/candidate verification, pure comparison test assertions and historical introduction examined. The existing managed-publication restriction is established, not inferred from the example. |
| `tools/evidence` | Reproduce the advertised route with independent observations | Headless controller and Slotbook source/learning Rust drivers | Orchestration and model-response consumers traced in depth; shared binary/cleanup harness mapped. No new execution, model-quality or hardware claim. |

The source entry points are [control](../../../../../crates/control/src/lib.rs),
[workspace](../../../../../crates/workspace/src/lib.rs),
[context discovery](../../../../../crates/runtime/src/context/source/discovery.rs),
[model](../../../../../crates/model/src/lib.rs),
[learning requests](../../../../../crates/control/src/learning/request.rs), and
[evidence composition](../../../../../tools/evidence/README.md). Their boundaries are not
justified by their names. The comparisons below state which obligations would move if a boundary
were removed.

## What actually turns agent output into control today

There are three distinct implemented relationships.

1. A caller constructs `WorkflowProposalDocument` and calls `ControlService::execute` with
   `SubmitProposal`. Control validates the exact base revision/digest and optional run sequence,
   privately constructs a complete candidate, checks authority and risk, and delegates live
   acceptance/application to runtime. A proposal cannot create a genesis revision: `submit`
   requires an existing base. Ordinary daemon authoring can construct/import that base.
2. `WorkflowControlAdapter` exposes those same commands inside work, but its request is a complete
   **inline** `ControlCommandDocument`. It reconstructs actor/grant from frozen invocation authority
   and rejects disagreement. It does not take a model-response artifact, extract a mutation,
   discover target state, or synthesize authentic model provenance.
3. `ControllerLifecycleOwner` assesses whether a previously supplied repeat body may enter another
   cycle. `build_controller_blueprint` compiles one `Repeat` plus a terminal and policy metadata.
   The body, continuation condition, limits and author are its inputs. Progress/account decisions
   are durable, but this owner neither writes a plan nor chooses evidence for a planning model.

These statements follow the actual
[adapter input](../../../../../crates/control/src/adapter.rs),
[proposal submission](../../../../../crates/control/src/service.rs), and
[wrapper builder](../../../../../crates/control/src/controller/policy.rs), rather than the
word “controller.” The closed [control command enum](../../../../../crates/control/src/command.rs)
has inspection, proposal, approval, application, pause/resume, run creation and retained-work
resolution; it has no goal-to-method request.

A whole-workspace Rust search for `from_model_response` and
`workflow_proposal_structured_output` resolves the possible hidden consumer. Apart from parser
tests, the structured producer/consumer is the
[Slotbook learning driver](../../../../../tools/evidence/src/bin/slotbook-evidence/learning/author/exercise/proposal.rs).
The one production call to `from_model_response` is
[daemon learning verification](../../../../../apps/daemon/src/host/commands/learning/proposal.rs).
It authenticates a candidate **already supplied by the caller** against the model's retained
response, manifest and declared run. It does not launch or continue the model operation. Source
inspection found no production consumer that closes that missing orchestration.

The demonstrations make the gap concrete. The
[headless controller driver](../../../../../tools/evidence/src/bin/headless-cli-evidence/controller.rs)
constructs the repair mutation, installs a decision node, submits it, obtains separate approval,
applies it and releases waiting work. Its
[model review prompt](../../../../../tools/evidence/src/bin/headless-cli-evidence/controller/review.rs)
tells the model that the intended fix changes `answer.txt` from 41 to 42 and asks for
`approved_repair`. This proves useful execution/accounting and refusal relationships; it does not
prove discovery of a method or independent orchestration. The
[Slotbook source driver](../../../../../tools/evidence/src/bin/slotbook-evidence/develop/proposal.rs)
selects prior observations, builds/imports a model workflow, starts and polls it, downloads its
response, maps source code into a known editable region and attaches authenticated provenance.
The learning driver gives the full baseline and schema, then similarly constructs the workflow,
downloads/parses output, binds provenance and submits the candidate. The drivers are real Rust
clients; their development-only location is not the problem. The problem is that the advertised
general product route depends on their workload-specific coordinator knowledge.

## Why the boundaries arose, and what that does not justify

Git `8238706` introduced the control application layer and the model-response parser together.
[ADR 0013](../../../../decisions/0013-immutable-proposal-revisions.md) answers a real race: a producer
may finish after the run has advanced, so a private candidate plus guarded prospective adoption
must separate intended changes from accepted history. That remains necessary for S28. The commit
and ADR do not establish that an external caller should permanently own model invocation and
response binding.

Git `09a5b86` introduced durable controller lifecycle enforcement after controller limits existed
only as a pure assessment contract. [ADR 0026](../../../../decisions/0026-durable-bounded-controller-lifecycle.md)
explicitly rejected a daemon polling scheduler and privileged AI node, because they would duplicate
runtime entry and authority. Later accounting (`709de9b`, `d537059`) and activation (`26f56a5`)
closed concrete admission gaps. Those decisions justify retaining one scheduler and account
transition, not claiming that a supplied repeat body is an implemented agent supervisor.

Production causal context entered in `307a90a`; later changes strengthened exact session and
retained-manifest enforcement (`a24671a`, `0fb775a`).
[ADR 0031](../../../../decisions/0031-context-enforcement-and-retained-evidence.md) and
[ADR 0036](../../../../decisions/0036-explicit-model-continuation.md) explain why a retry cannot
silently receive today's broader context. The current source preserves distinct source selection
and host byte loading. No reviewed history proves the present automatic selection order optimal
for every research task; it is deterministic infrastructure, not a model-quality result.

Protected adaptation (`cb6697b`) and evaluated learning (`17aab7d`) grew around a real consequence:
work must not pass merely by deleting its check or publishing different bytes. The narrow profile
is visible in [ADR 0040](../../../../decisions/0040-protected-adaptive-methods.md) and executable
learning types. Its origin is established as protected application-method evaluation. It is not
evidence that every future research lesson needs a managed service, publication generation and
repair-count metric. The original need behind every chosen numeric bound was not reconstructed;
this review makes no such claim.

## The demanding case and independent expected outcomes

Use [S28](../scenario-corpus.md) with these concrete inputs for all three placements. These are
modeled packets and expected results, not a performed model experiment.

| Input | Exact content/selection and owner |
| --- | --- |
| Goal `G0` | Human artifact: improve the repository's methods of configuration validation, preserve accepted public behavior, investigate before implementation, obtain independent criticism, stop before publication without its separate permission. |
| Repository `R0` | Exact source snapshot/artifact or explicitly authorized managed working area generation. The mutable checkout is not silently treated as immutable evidence. |
| Existing work | For creation, no method. For revision, target revision `V7`, run `W`, observed sequence 120; investigation `I1` and critic `C1` are complete; implementation `X` is pending. |
| Bad premise | `I1` records that configuration is interpreted only at startup. `C1` points to a permitted independent test showing a later adapter re-read. Criticism is data with producer identity; it is not a new authority grant. |
| Capabilities | Permitted exact descriptors: read-only source analysis, local test process, selected remote test host, restricted deployment method, configured model endpoint. Describe requirements and side effects; do not send credentials. |
| Permission | Planning actor may read selected evidence and propose ordinary future revisions. It may not replace the protected deployment agreement, become its verifier, change the target, or give itself publication authority. |
| Bounds | One assistance request has an explicit model/process/byte/time/proposal allowance. Existing target work retains its own cumulative account. A follow-up may not silently mint another allowance. |
| Held-out evaluation | Independent evaluator chooses new cases before a lesson-generating request; proposal actor gets source evidence and criterion, not private test inputs. |

The expected revised plan adds investigation of the second interpretation site and a regression
check before `X`; it preserves `I1` as evidence of the rejected premise rather than rewriting it.
The critic's correction does not by itself establish application correctness. A fresh context must
be able to reconstruct the exact request, source versions, selected/omitted context, model output,
candidate and decision without a remembered conversation. A capability change, revoked authority,
missing required source, malformed output or stale affected future must produce a useful retained
refusal. An uncertain remote test remains uncertain and is not repeated merely because a planner
wrote a replacement node. An accepted restricted-service result exposes only its declared output.
Any lesson is a finite hypothesis whose supporting and contradicting cases remain inspectable.

## Three complete placements on that same case

| Boundary | A: external-client composition, complete today | B: product-owned assistance round, compiled to ordinary work | C: durable general work session |
| --- | --- | --- | --- |
| Public entry | Human/agent client authors/imports base, uploads selected inputs, starts model workflow, polls, downloads output, constructs/submits proposal, approves/applies/resumes separately | `PrepareAssistance` freezes a reviewable packet; `StartAssistance` accepts it and starts a bounded ordinary run; existing proposal/approval/apply operations finish adoption | `CreateWorkSession` accepts goal, authority, budget and plan state; `AddCriticism`/`Answer`/`Continue` are session commands |
| Coordination state | Client must persist stage, exact request keys, target snapshot, artifacts, pending clarification and budgets; daemon retains accepted individual operations | Accepted request receipt plus immutable packet and normal run/attempt/artifact/proposal facts. Assistance read state is a projection of these references, not another mutable execution record | Durable session record owns pending action, round, target run references, messages and next transition; it submits ordinary runtime work and tracks completions |
| New goal | Client invents a legal genesis/base and model contract before asking for mutations | Product creates an explicit empty draft base for proposal authorship, compiles prepare/model/admit steps, returns validated candidate for inspection/start | Session owner asks planner, admits candidate and starts execution under session policy |
| Criticism | Client pauses appropriate future work, selects `I1/C1`, asks model and binds correct base/sequence | Explicit revision request freezes `V7/120` and sources after authorized pause; model proposes; admission authenticates its output and submits against that target | Session marks plan invalidation, schedules investigation/critic/plan transitions and decides when to resume according to policy |
| Clarification | Client persists question and chooses how to ask again | Structured clarification is retained as a round result. Answer references it and starts the next explicitly authorized round; no hidden assumption or automatic new allowance | Session waits for an authenticated answer then advances a durable phase under retained overall limits |
| Invalid output | Client retains bytes, records diagnostic and decides retry | Admission emits typed failure plus raw response reference; no partial proposal. Explicit correction round selects diagnostic and sources | Session can schedule bounded correction attempts, then stops at its configured ceiling |
| Restart/fresh context | Individual daemon facts survive, but a new client needs the external coordinator's checkpoint file to know what remains | Query by accepted request returns run, packet, refusal/proposal/clarification and exact result references; a fresh client resumes observation/adoption | Session resumes its pending transition from its own durable record |
| Permissions and accounts | Ordinary operations enforce their permissions; client has to coordinate scope and multiple allowances | Same enforcement plus packet read checks and authenticated response binding. One round's ordinary controller account covers its descendants. Separate target allowance stays explicit | A new durable session authority/account origin must be added or represented by one root runtime account; otherwise new sub-runs could reset its claimed overall budget |
| Remote/service/resource use | Client knows which invocation result can be used, owns editing handoff and context export | Packet includes only authorized declared results/resource references; ordinary run/host paths own remote selection, holds and uncertainty | Session must learn and persist all those relationships or derive them from existing owners without mirroring them |
| S28 autonomy | External agent does the orchestration that the product is meant to offer | Product completes the bounded semantic action “propose a useful method/revision.” Human initiates consequential criticism/answer rounds; authored workflows still execute analysts, critics and implementation | Product can coordinate open-ended rounds under an explicitly authored session policy, including automatic reconsideration |
| Long-term cost | New authoring contract requires fixes in every independent client/driver; clients tend to duplicate provenance and recovery | Add a new admitted plan-result case in one control owner, compiler, protocol projection and fixtures; runtime execution semantics are reused | Every new wait, target relation, interruption and cancellation must preserve both session transition and runtime lifecycle; removal of runtime ownership would become a larger replacement |

All three can obey accepted-history and scoped-authority rules. A is not rejected for being
external; it is rejected as the only ordinary product route because users must recreate an
orchestrator for the central action. C is the strongest alternative when automatic cross-run
reconsideration under one project budget is the product promise. It is more than a conversational
UI: it needs an owner for session transition receipts, lifetime, budget inheritance and in-flight
reconciliation. Simply adding a mutable “current plan” table is not that design.

**Initial selection: B, superseded by U18 and the C review below.** Milkdrift should own a complete assistance
round now; it should not advertise that as a general autonomous project session. It delivers
the required goal and criticism actions without introducing a second scheduler or requiring the
client to construct model provenance. S28's human can initiate “reconsider after this criticism”
and decide when altered obligations justify a new agreement. Investigation, criticism, execution,
verification and prospective changes then remain inspectable ordinary work. This interpretation
does not require every consequential action to be autonomous, consistent with the reopening
request. If the product instead promises automatic multi-run orchestration with one session-wide
allowance independent of authored workflow structure, C is a real additional owner and must be
selected openly; B does not establish that promise.

## Selected route: exact contract and construction

The following are proposed Rust names, not current APIs. They define the boundary sufficiently
for implementers to build it without searching again for a consumer.

`control::assistance` owns `AssistanceIntent::{CreateMethod, ReviseFuture}`,
`AssistancePacket`, `AssistanceResult::{Clarification, Candidate, Refused}` and the compiler.
`PrepareAssistance` takes the goal/criticism artifact identities, exact target if any, explicitly
selected sources, capability requirement, permitted action policy and finite allowance. Daemon
adapts authenticated input; the control owner reads through narrow revision, projection, artifact
and authority ports. It does not accept client-supplied grant facts, artifact checksums, current
run state or model provenance. `StartAssistance` binds the exact packet digest and command key.
`InspectAssistance` returns the accepted receipt and derived ordinary run/proposal references.

The immutable packet contains:

- Goal or criticism artifact; source snapshot/managed generation; selected evidence references
  and source run/sequence anchors; explicit omissions and required-source policy.
- For a new method, workflow identity and the product-created draft base; for repair, exact
  target run/base revision/digest/sequence and a bounded classified frontier. A draft base is a
  nonexecuted starting definition, not evidence that any work happened.
- Allowed capability descriptors/requirements, remote placement options and declared service
  interfaces visible to this actor; relevant read/write/effect restrictions without secrets.
- Existing agreement/editable-region restrictions, allowed proposed action, remaining applicable
  target allowance and the separately authorized assistance allowance.
- Versioned proposal/schema guidance, desired result obligations and provenance to the request,
  actor/grant decision, source artifacts and exact capability catalogue observation.

The compiler builds an ordinary bounded assistance run with three task responsibilities:

1. Prepare/materialize the authorized packet into selected input artifacts. The packet must remain
   within context/request limits before any model entry. Required missing inputs stop here.
2. Invoke the selected external model through the existing `ModelTaskRequestDocument`, defaulting
   to a fresh context whose `ContextManifestDocument` names the packet and exact source artifacts.
   A fresh model is not expected to remember the target or infer any granted authority.
3. Admit the response through a new typed workflow-control operation taking the packet reference
   and the exact model-response artifact. It authenticates the producing invocation, profile,
   frozen context and response bytes, parses the closed result, reconstructs trusted proposer
   identity and submits an ordinary proposal. Model-supplied authority, risk, producer identity
   and success claims are never trusted. The result is an immutable clarification, candidate or
   refusal artifact, with a normal run outcome and accepted command reference.

This compiler consumes the general blueprint/authoring owner. It must not call the private
daemon `ModelWorkflow` recognizer or duplicate prompt-sequence compilation. Its use of a
controller account requires the existing explicit controller activation; preparation reports an
inactive host and start refuses before model entry until the authorized daemon setup supports
the requested bounded operation. It does not silently turn on a disabled controller lifecycle.

The new response contract should let the model supply proposed mutations, rationale, assumptions,
risks, cited selected evidence and clarification. It should not require it to predict the future
response artifact ID or copy authentication fields. The product binds these facts once. The
existing complete `WorkflowProposalDocument` remains the canonical admission contract for all
producers; the new authoring result is untrusted input to it, not a competing revision format.

Preparation and admission are control capabilities inside ordinary runtime work. Their outputs
flow through declared artifact/data bindings. They must not run an HTTP polling loop inside the
daemon or perform inference in control. The compiler and adapter are shared by CLI, standalone
Svelte and explicitly configured agent callers. The daemon adds wire adaptation/receipt discovery,
not a second authoring policy.

Two subtle constraints are part of this design, not deferred implementation details:

- The assistant must not propose against its own advancing run sequence. Its own model/response
  events would make the packet stale before admission. A revision round targets a separately
  named paused run; any concurrent target observation still makes its exact guard stale. Admission
  preserves that refusal and the valid offline candidate. It does not silently rebase or replace
  the model's observed frontier with the newest one.
- This operation's account covers the accepted round and descendants, including retries.
  Existing target work keeps its original account. An answer/correction creates a new round only
  with explicit authority and allowance; an automatic retry may consume only the existing round's
  remaining allowance. This is not a project-wide budget. The interface must show that scope.

After successful admission, ordinary `ProposeOnly`, risk/approval, apply and requested run action
govern whether the target resumes. A new method is inspectable before `CreateRun`/start. There is
no implicit authority to start the proposed method merely because model planning succeeded.
An explicitly authorized start/apply policy can compose these existing commands using retained
request identities. Lost replies recover their exact results; they do not call the model again.

This route makes S28 useful without claiming all open-ended coordination is solved. The initial
ordinary method can declare independent analyst branches, critic inputs, implementation and
verification. A bad premise is surfaced by their explicit artifact outputs or a human; the human
requests revision with those artifacts. That is a product action, not source-level client
orchestration. Humans still choose the goal, permitted authority, required independence and
consequential agreement changes. Models propose interpretations and methods. Milkdrift owns
packet construction, execution, admission, retained evidence and permitted prospective change.

## Structural adaptation, reuse and context must agree

S28's investigation/coordination method should use ordinary exploratory revision semantics. It
must not be placed inside a current protected task-only region and then represented as if it
could arbitrarily replace forks, critics or nested methods. Current protected adaptation admits
ordinary tasks in named editable regions; protected structure and edges remain fixed. A child
inherits the original agreement and cannot escape it through direct adoption. Changing the
agreement requires a separate authorized agreement/run, preserving the earlier run's evidence.
Those are different operations from adding a permitted repair task.

Ordinary nested reuse keeps an exact method revision/interface. Remote execution selects a host
capability; it does not force publication of the whole method. A restricted service has a distinct
service authority and public-output boundary, so an assistance packet sees its declared outputs,
not private internal investigation merely because it has permission to invoke the service.
Managed working areas remain mutable resources with generation/hold/editing ownership. They
cannot be made immutable by calling them “context”; freeze/export selected bytes for an evidence
packet, and keep physical editing handoff with the resource owner.

Retain runtime causal selection and host materialization as separate owners. Combining them in a
global session memory store would have to reimplement branch exposure, authority, omissions and
retry identity. Using only explicit packet inputs would simplify a narrow planner but would
remove useful causal selection from ordinary workflows. The selected route instead uses an
explicit packet for this external-target operation and the existing causal manifest for its
actual model invocation. There is no automatic cross-run global memory search.

Retain workspace's distinction between immutable artifacts, versioned logical values and mutable
working areas. A single file tree loses versioned provenance and safe branch references; making
every file version an artifact cannot own editing permissions or an externally running service.
Artifact metadata and a context manifest are not duplicate history: one says who produced the
bytes; the other says exactly why this attempt selected or omitted them. The context producer
must derive those facts from their owners rather than copy editable authority into a new packet.

## Evaluated lessons: avoid turning Slotbook into the general product

This initial scope comparison preceded U17. Its diagnosis of current restrictions remains; the
final U17 disposition below replaces its narrower assumptions about reviewer independence,
predeclaration and knowledge being an executable method.

The implemented [learning declaration](../../../../../crates/control/src/learning.rs) requires
agreement/policy/verifier digests, exact protected check names, separate managed workspaces and
targets per pair, accepted published invocations, a returned candidate artifact and a positive
`minimum_repair_reduction`. [Daemon declaration](../../../../../apps/daemon/src/host/commands/learning.rs)
checks the resources are prepared at the declared generations and verifies their protection.
The comparison counts failed candidate submissions, rejects any paired regression and preserves
missing evidence as inconclusive. This is valuable for a protected application method. It rejects
a legitimate research method that writes a decision brief and changes no managed deployment.

Three choices are materially different:

| Choice | S28 lesson and cost |
| --- | --- |
| Keep only current protected-publication evaluation | Complete and defensible for a lesson about implementation/repair before publication; a planning lesson can be retained as unvalidated guidance but cannot honestly be called evaluated by this mechanism. No new evaluator semantics. |
| Add one finite ordinary-work evaluation profile | An independent evaluator fixes source/input pairs and a comparison criterion before proposal; candidate outputs are exact ordinary run artifacts; recorded per-case judgments support a bounded lesson without provisioning fake services. The product gains a real result-evaluation contract. |
| General metrics/plugin framework | Arbitrary scorers could define quality, cost and many domains, but require extensible trust, metric versioning, missingness, comparison and promotion rules. No established multi-owner contract justifies that machinery yet. |

Recommend the second choice, explicitly keeping the existing profile. The new closed profile
`PairedResultJudgment` should bind exact baseline/candidate revision, paired immutable inputs,
predeclared run/attempt identities, required output fields, evaluator actor/grant, a rubric
artifact, finite time/usage limits and an exact criterion such as no worse case plus at least one
improved case. `RecordJudgment` is a separately authorized command binding both output artifact
digests to `Better`, `Equivalent`, `Worse` or `Inconclusive` plus evidence. A proposal author cannot
be its own required independent evaluator. A human can make this judgment; an AI evaluator is an
explicitly configured actor/capability, not inferred to be trusted because it is a model.

`Compare` checks accepted run inputs/outputs/authority/account facts and exact evaluator receipts,
then applies the predeclared criterion. It does not interpret prose as a verified score. Missing
judgment or output, stale rubric or a different input remains inconclusive/refused. This qualifies
only the declared finite comparison, never general superiority. The old
`ProtectedPublicationRepairReduction` profile retains private verifier/target checks and exact
returned-product protection; splitting the criterion must not make those fields optional inside
that profile.

Approval for later reusable guidance should become a distinct `ApproveLesson` receipt binding the
comparison, method revision and selected guidance artifact. Selecting that approved lesson for a
later packet does not need a published capability. Existing publication promotion remains a
separate operation consuming an eligible comparison under publication authority. This removes
the accidental requirement that knowledge approval be a service-publication promotion while
preserving the original protected route. The source selector should name an optional exact
managed resource relationship rather than require a managed installation for ordinary artifacts;
resource-associated selections retain their current generation/inspection checks. No second
knowledge database or generic scoring registry is needed.

This recommendation expands the product from its present finite profile, rather than asserting
the broader feature already exists. It follows S28 and the user's stated reuse ambition; a
requirement that every learned method be automatically judged without a human would be a further
product decision, and is not assumed here.

## Concrete reconstruction and adoption units

Retain `ControlService` proposal validation, blueprint revision construction, runtime reconciliation,
controller final-entry/account transitions, workspace provenance, model documents and serving/resource
owners. Their alternatives lose a required distinction or recreate it elsewhere. Consolidate
authenticated model-response admission now implemented specifically in daemon learning into
control's assistance/admission owner through narrow artifact/provenance ports. Keep the
learning-only held-out/source-selection rules as an additional check; do not weaken them into a
generic “model response verified” flag. Move general knowledge/evaluation orchestration from the
daemon command handler into that same control-owned application boundary while daemon retains
authentication, wire DTO adaptation and command receipts.

Remove workload-independent construction, output parsing, provenance rewriting and orchestration
from the evidence drivers once the public assistance route covers them. Retain workload prompts,
fixtures, independent counters, fault injection and result assertions as test inputs/observations.
Do not delete exact canonical proposal readers, ordinary external-client proposal submission or
historical records: independent authorized clients remain useful producers. The ordinary frontend
must consume the same assistance operations rather than recreate the removed driver.

1. Add immutable assistance packet/result contracts and response-authentication owner. Migrate
   learning's common producer checks and keep its extra held-out checks. Demonstrate a complete
   explicit request against controlled model output, no new autonomous loop.
2. Add compiler and public prepare/start/inspect operations; use existing run, receipt and artifact
   owners for recovery. Ship CLI and independent JSON-client coverage before the frontend. This
   intermediate state is usable for new-method generation and separately paused-run revision.
3. Adopt that route in evidence drivers, delete duplicate common orchestration and expose the
   finite action in Svelte. Inspect clarification, invalid output, stale target and lost-reply
   recovery during early human use, not only successful graph display.
4. Separate finite learning profiles and independent lesson approval from publication. Migrate
   current producers/consumers to the protected profile explicitly; add ordinary-work paired
   judgment with independent receipts and preserve negative/inconclusive cases.

Old revisions, runs, proposals, manifest bytes and receipts remain unchanged. New assistance
records have their own versioned artifact contracts and derived read projection. No active or
uncertain work is adopted into an assistance operation implicitly. A target must be explicitly
selected with its current guard; uncertain effects keep the existing resolution path. For learning
declaration evolution, retain exact v1 receipts as read-only historical records mapped to the
protected profile without changing their canonical digest or replay result; new writers emit the
new closed profile. The selected implementation is additive new declarations with unchanged v1
replay, because the old profile's meaning is unambiguous. There is no rewrite of accepted
declarations or conversion of old comparisons into the new ordinary-work profile.

## Tests must challenge the choice

Existing tests inspected here have useful independently justified assertions, but they are not
whole-product proof. [Proposal contracts](../../../../../crates/control/tests/proposal_contracts.rs)
reject malformed structured output. [Causal context](../../../../../crates/runtime/tests/causal_context.rs)
permutes candidate order, refuses required losses even after selection stops, redacts omissions,
isolates siblings and commits a manifest before dispatch. The first of those proves deterministic
selection, not that the selected evidence is the best evidence for reasoning. A legitimate
variation is a decisive old critique reachable through an explicit artifact while much later
irrelevant work exists; another is a join exposing one output while the sibling's private failed
analysis must remain hidden.

[Learning tests](../../../../../crates/control/tests/learning.rs) independently vary one paired
regression, zero baseline rework, missing history/usage, different returned product, verifier and
shared resources. They justify the existing finite profile. Their requirement for managed targets
does not justify rejecting an ordinary brief-review method; that is the variation motivating the
new profile. [Daemon learning verification](../../../../../apps/daemon/src/host/commands/learning/proposal.rs)
checks the actual response against the submitted mutation and rejects held-out context; this must
remain on the production path after consolidation.

Required new discriminating observations are: start assistance through public JSON without a
driver-built model workflow; restart after model entry and before admission without a second call;
retain a malformed response and accept only an explicitly authorized correction; ask clarification
and resume with a different fresh client; revoke a source's read authority before preparation;
change the target while the model runs and preserve stale refusal; expose only a restricted service's
declared result; hit the combined descendant allowance without resetting it through retry; retain
an uncertain remote test; and revise S28's future while both the bad premise and its independent
counterexample remain inspectable. A separate observation must verify that a new authorized round
is shown as a new allowance, not masqueraded as continuing the old one.

For ordinary-work learning, use a fixed evaluator rubric and independently supplied judgments,
then mutate one input, output digest, evaluator actor, criterion or missing case. The comparison
must not stay eligible. Preserve the same protected-publication refusal suite unchanged for its
profile. These are proposed acceptance tests, not newly executed evidence. No Cargo commands,
paid calls, production changes or prototype were performed by this reviewer.

## Elin's cross-review of Delta's representation selection

This section was added after the independent position above and after reading
[Delta's complete comparison](structure-comparison.md). Delta initially called the corrected graph
M; this cross-review calls it G to distinguish it from the method being authored. I agree with the
semantic corrections: ordered choice independent of IDs, an owned selected result, exact
join-outcome inputs, full ordinary Task configuration for an external judge, and protected
agreement boundaries. These corrections are valuable under either G or canonical regions R.

I do **not** find the original positive case for G stronger than R's for agent-directed authoring.
Delta's concrete R already allows stable task IDs, ancestor value references, explicit parallel
exports and pending-descendant repair inside a selected region. Consequently “data dependencies
can cross sequential control portions” does not by itself favor G. In the supplied packet, the
candidate flows from Conditional to two review calls and the judge, while the brief also reaches
the judge. R represents both without copying bytes or inventing another run. Its optional
specialist variation similarly has a Conditional inside Parallel returning `ReviewOrSkipped`,
followed by one judge. These are the decisions the user/model actually needs to make.

G's complete `ChoiceDraft` improves the first authoring action, but criticism must reopen a
durable method, locate its selected branch and replace pending work. With graph-only canonical
source, the product reconstructs structured membership from ports/edges and stored boundary
configuration before presenting that edit. R stores that membership directly. G's proposed
no-bypass/no-foreign-entry/no-crossing restrictions already make this largely structured control;
unrestricted graph flexibility is not established as a user benefit by D1–D8. A succession of
complete Choice/Fork/Repeat/Await constructors risks becoming an implicit region language whose
canonical document remains its compiled output. The source burden matters for fresh agents as
well as human diagrams.

The strongest R adoption therefore keeps a canonical `MethodDefinition` with a closed region
enum and stable element IDs, and lowers it through blueprint into one `ValidatedExecutionPlan`.
Runtime, reconciliation and context consume that checked plan plus source identity. The graph is
a computed view; authored regions and generated edges do not become competing mutable objects.
The assistance model supplies region edits/result signatures or corresponding blueprint compound
operations; it does not emit control-port plumbing. Historical graph documents retain versioned
readers/lowering with exact old identities and receipts. New authorship emits only R. Conversion
of a live old graph requires a proved element/occurrence mapping and unchanged external
obligations; otherwise the accepted old run continues and new work starts explicitly with selected
evidence. This supported-data consequence must be budgeted, not dismissed as cosmetic migration.

For these examined cases I favor R as the **new canonical authoring source**. That does not imply a
new task executor, memory database or authority owner. It is compatible with assistance and the
existing effect/runtime owners. I would retain G instead if a concrete valued current graph or
required future edit needs R to add an escape hatch, extra run or materially different
scheduling/data semantics. Delta accepted this objection and is revising the actual selected
representation to canonical structured source; this is a change to the recommendation following
criticism, not merely agreement that the previous documents were consistent.

No author reliability experiment was performed. Neither knowing UML/BPMN nor producing fewer
fields proves better model choices. What is established by inspection is narrower: R puts
conditional result totality/containment directly in the definition, while G needs compound
construction plus whole-graph validation and a derived ownership index. Both still require
capability schemas, explicit context, preserved evidence and a fresh-context packet. Assistance
must provide a canonical editable **method** view; generated plumbing is not a sufficient
goal-to-plan interface.

## C developed fully: continuous bounded work, without another task scheduler

The coordinator challenged B's finite scope; the user subsequently approved continuous authorized
coordination in U18. The initial selection above remains as a record of that narrower
interpretation. This section supplies the stronger C as an implementable alternative, including
why “just compile it into the existing root” is not yet a complete answer. It is a modeled design,
not a claim that the current product has these operations.

The useful additional outcome is concrete: after the independent critic reports the bad premise,
Milkdrift pauses affected future work, starts a fresh planning context, admits a revision, resumes
permitted work and later evaluates the lesson, without a human manually starting each new round.
The human still owns the goal, authority, cumulative allowance, required independence, acceptance
obligations and any changed agreement. These choices are fixed in an immutable
`WorkSessionPolicy`, not repeatedly reinterpreted from a chat transcript.

### Compare the two plausible session state placements

| Placement | Complete mechanism | Consequence |
| --- | --- | --- |
| Ordinary root orchestration run | A root Repeat/Parallel method owns the account; planner/critic/work children inherit it. Dynamic admitted-method selection must bind an exact child revision before creation. Explicit loop-carried state and nonterminal checkpoint/monitor behavior let a coordinator act while a work child is paused. | One runtime history is attractive, but those last operations do not exist in today's pinned, terminal-returning Call contract. A new dynamic Call plus asynchronous child-monitor/ownership rule would be real additional runtime semantics. It is not enough to draw a Repeat around existing tasks. |
| Control-owned session with exact runtime associations | Control owns accepted next-work decisions and bounded pending actions. Runtime owns every accepted work/model run and all task scheduling, attempts, effects and recovery. Persistence atomically validates account associations when a session action creates a run. | Adds a real application coordination record but does not duplicate task lifecycle. Can react to a paused or running work run using public owner facts; avoids forcing an artificial never-ending workflow around the user's methods. |

The limitation of the first route is source-backed. Current
[subworkflow driving](../../../../../crates/runtime/src/engine/structured/subworkflow.rs) starts
an exact pinned child from a parent execution and imports its outputs on terminal observation.
A paused child remains active; it does not return a next-round result. Current
[repeat driving](../../../../../crates/runtime/src/engine/structured/repeat.rs) consumes a fixed
pinned body and its completion/condition. A planner revising its own root also observes a sequence
that its own output then advances. A carefully designed future root method can solve these, but
it needs exact new semantics; claiming it uses only current primitives hides the design work.

For C, recommend the **control-owned session**. This is not a general competing scheduler:
`SessionOwner` decides which already-authorized method/analysis should be requested next; runtime
alone decides when each method's nodes and external operations enter. There is a new coordination
state machine, and it must earn its cost by eliminating external-driver stage tracking and by
giving continuous intent, interruption and a total allowance one durable owner. It should replace
evidence-driver orchestration, not coexist with an undocumented client supervisor.

### Types, public operations and ownership

`control::session` owns `WorkSessionPolicy`, `SessionIntent`, `SessionDecision`, packet preparation,
response admission and a pure `plan_session_transition`. The policy fixes goal/input artifacts,
allowed methods/capability envelopes, mutable target set, permitted proposal/application risk,
critic independence rule, result obligations, model selection, maximum rounds/rejections/time,
maximum concurrent work, cumulative resource budget, interruption policy and stop/escalation
rules. It permits a narrowed successor request; changing the accepted goal/obligations or widening
authority/budget requires a separate authorized policy decision with its own provenance.

Public operations are `CreateWorkSession`, `SubmitCriticism`, `AnswerClarification`,
`PauseWorkSession`, `ResumeWorkSession`, `CancelWorkSession` and `InspectWorkSession`. Candidate
approval/application and uncertain-work resolution retain their ordinary operations; session
actions reference those receipts. `CreateWorkSession` takes the reviewable prepared intent/policy
and exact command identity. The response returns session, policy, account and accepted action
references; it never treats an absent model result as an empty plan. `SubmitCriticism` and
`AnswerClarification` bind exact selected artifacts and an expected session version. An answer
also binds the outstanding question identity, so answering one round cannot release another.

Persistence owns `SessionActionRecord` and a `SessionExecutionAssociation` contract. Session
actions are append-only accepted facts, grouped by session sequence, with ordinary application
command receipts for exact replay. A bounded projection/index may retain current pending actions
and question; it is rebuildable from those records. It stores artifact/run/command references,
not copies of run events, capability status, model text or execution outcomes. `SessionOwner`
uses a narrow session journal port implemented by redb; daemon authenticates requests and
composes a bounded continuation driver in its existing owner queue.

The projected phases are `NeedsDecision`, `Planning`, `Working`, `AwaitingAnswer`,
`AwaitingApproval`, `Paused`, `Stopping`, `Completed` and `Stopped`. `Working` contains a bounded
set of accepted work associations and pending criticism references; their actual lifecycle is
read from runtime. Phase changes are not evidence that an invocation succeeded. `Stopping`
remains distinct from `Stopped` while cancellation, account reservations or uncertain effects
remain unresolved. The policy can explicitly allow further authorized resolution operations,
without reopening ordinary new work.

The model returns one bounded `SessionDecision`:

- `Clarify` with a question and the exact ambiguity blocking the next action;
- `ProposeMethod` or `ProposeRevision` with canonical structured method edits, selected citations,
  rationale and declared affected work;
- `RequestWork` or `RequestCriticism` naming an admitted method/revision and exact input mapping;
- `Conclude` with result/evidence references, or `Escalate` with the unmet authority/obligation.

These are requests to control, not executable commands. The owner binds actor, model invocation,
profile, response and context manifest; validates citations against the selected packet; validates
method edits through blueprint; and evaluates the corresponding existing authority operations.
It can reject a forbidden decision without asking another model to “explain why it was safe.”
Critic identity is selected by the accepted independence policy, never by a candidate's assertion
that its own output is independent. Result obligations are checked against accepted output and
verification facts before `Conclude` can become completion.

### One cumulative account, with a real origin

Do not manufacture a terminal-only controller run merely to obtain a session account. Current
[account declarations](../../../../../crates/persistence/src/controller_account.rs) already
distinguish controller occurrences and published invocations, but retain a mandatory controller
run and optional origin fields. Replace the new-write origin shape with a closed
`ExecutionAccountOrigin::{ControllerOccurrence, PublishedInvocation, WorkSession}`. A session
origin binds session ID and immutable policy digest, with no invented run identity. The existing
resource totals, reservation IDs, admission/settlement/byte-charge transitions and conservative
unknown-use behavior remain one implementation. The name can become `ExecutionAccount` as all
consumers migrate; a public alias must not preserve two editable owners.

Every planning, criticism, target and evaluation run admitted by this session binds that account
and a frozen authority basis before it can start. Descendant workflows inherit both as they do
now. A independently published restricted method still uses its own service authority and
internal allowance; the caller's session retains its invocation reservation and attributable
outer usage under the existing serving contract. Do not claim the caller can inspect or directly
control its internal account. Unknown charges remain reserved; retries, fresh contexts, a new
method revision and daemon restart cannot establish another session allowance.

`SessionExecutionAssociation` names session/action/policy/account, child run and exact revision,
canonical inputs, caller authority decision and derived create/start command IDs. It is created
only from an accepted session action. A caller cannot bind an arbitrary run by supplying an
account ID or `is_child` flag. Before runtime accepts creation, the journal transaction rechecks
that exact accepted action, unconsumed association, account origin, authority containment and
session cancellation/deadline. It commits run creation and `BindRun` together. The session's
active-run association is the accepted action reference; no copied run status is committed there.

### Atomic handshakes and recovery

| Boundary | Accepted state and crash behavior |
| --- | --- |
| Create session | One application transaction commits canonical command receipt, immutable policy, account establishment and first pending action. If account establishment cannot commit atomically, no session is accepted. A lost reply replays this result. |
| Decide to invoke model/work | A session transition records the complete packet/input digest, exact run identity/revision and deterministic create/start keys before any runtime call. This is intent, not proof of external entry. Capacity/round bounds include pending actions. |
| Create/start run | Runtime creation and account binding commit under the exact accepted association. Start uses its stable ordinary command key and existing authority/requirement checks. Crash between create and start leaves a recoverable unstarted run, never an unaccounted model call. |
| Observe result | Driver reads the accepted output/attempt/manifest from its owning run. It records one consumption keyed by action and output digest. Missing or conflicting terminal evidence leaves the action unresolved; it does not issue another model call. |
| Submit proposal | Session first records the exact proposal command/digest. Ordinary control/runtime receipt is authoritative. If proposal commits and response/result publication fails, recovery queries/replays that same command and derives the same session result. It never substitutes a newly generated proposal. |
| Apply/resume | Separate accepted action references exact proposal, plan, approval and apply/resume command keys. Current revocation and target guards are rechecked. A stale target produces retained refusal and a newly budgeted planning round only if policy permits. |
| Criticism during work | Authenticated operation or a declared critic output records the criticism artifact once. Preauthorized pause is an ordinary target command; only after its result and current target frontier are known is a new packet prepared. Already entered work remains owned and may become uncertain. |
| Clarification | A durable question stops dependent new work. An exact authenticated answer appends a new fact. Restart neither fabricates an answer nor times out into permission. A deadline can stop the session under policy. |
| Stop/cancel | Session closes new action admission before requesting ordinary run cancellation. It retains active work references and reservations until their owners establish settlement or an authorized resolution. Inspect distinguishes stopped coordination from unsettled external work. |

The driver processes bounded pages of pending session actions on the existing daemon maintenance/
continuation path; it never runs provider calls inline or keeps an unbounded callback queue.
Waiting/notification loss changes observation latency, not accepted transition identity. Source
selection and current read authority are checked anew for a new planning packet. A model retry
for the same entered attempt uses the frozen selection; a new round with new evidence has a new
packet and consumes the same remaining session allowance.

### S28 through this C

Create records `G0/R0`, obligations, independent critic actor and total budget. The first planning
run returns a canonical structured investigation method. Control admits it; runtime starts the
associated work run under the session account. Independent analysts use branch-isolated context;
the configured critic receives only explicitly exported results. Its declared criticism output
or `SubmitCriticism` triggers the policy's pause action. The session records that the startup-only
premise was challenged, selects `I1/C1` and current pending/active target facts, and starts a fresh
planning run. The model proposes a future probe and regression check. Control validates the
structured source and ordinary proposal, requires approval where policy demands it, then applies
and resumes through the exact target commands. Old evidence remains unchanged.

If the requested edit crosses a protected agreement boundary, the session enters escalation and
cannot authorize itself to change the agreement. If a remote operation is uncertain, it requests
or waits for ordinary authorized resolution instead of creating a replacement call. Resource
editing holds remain with the actual active work. After accepted implementation/verification,
an independent evaluation declaration reserves role slots before lesson generation; it does not
pretend to know the eventual candidate revision or attempt IDs. Later accepted bindings fill those
slots with exact candidate/run/attempt facts. Evaluation results and any lesson approval then
become selected artifacts for a later session under its own read authority.

### Adoption and tests specific to C

Implement C's session contract, account-origin transition and association handshake together before
allowing any session-driven external entry. First expose a usable explicitly stepped session that
retains state through public commands; then enable the same bounded continuation driver, without
adding a second execution path. Migrate generic assistance-round preparation/admission into session
actions, so B becomes a one-round policy of C rather than a permanently competing orchestration
implementation. Remove evidence-driver phase/provenance/retry bookkeeping once the product owns
it; retain independent counters, faults and assertions.

Historical controller/publication accounts keep exact old identity/digest/replay readers and lower
to the corresponding closed origin; their accepted declarations are not rewritten. New sessions
start only under the new contract. Existing unrelated or uncertain runs are not automatically
attached to a session; observing their evidence is permitted separately, while taking control and
accounting ownership needs an explicit supported association decision. The initial scope should
refuse adoption of already-entered independent work into a fresh allowance, because doing so could
drop its unknown liabilities. A session may still propose a revision to such a target under its
own authority, clearly retaining the target's separate account.

Fault tests must stop after each table row's commit and before the next, reopen the daemon and
assert one account, one accepted model/work run, one proposal and unchanged external invocation
count. Exercise criticism while a child is running, criticism while paused, lost proposal reporting,
revoked source/actor authority, duplicate/conflicting answers, exhausted round and account bounds,
unknown remote usage, and cancellation with a retained editing hold. Tests must independently
inspect the actual runtime/account/serving owners instead of trusting a session phase string.

C is now concrete enough to compare. U18 approves its continuous-work outcome; the following
disposition selects C's concrete ownership for the revised planning recommendation, subject to
final user review. It does not authorize production implementation or require automatic acceptance
of every proposal, generic statistical evaluation or privilege bypass.

## U18 disposition: one continuous work commitment in the existing controller owner

U18 resolves the product scope: manual reinitiation remains an operating choice, but cannot be
Milkdrift's capability ceiling. Rename C's public concept and operations to `WorkCommitment`,
`WorkCommitmentPolicy`, `CreateWorkCommitment`, `InspectWorkCommitment`, and corresponding
pause/resume/cancel/criticism/answer commands. Locate the coordinator at
`control::controller::commitment`, with packet/admission helpers in the same control owner.
Do not add a parallel generic `control::session` framework. The earlier C names describe the
same candidate, not a second implementation to retain.

The distinct durable fact is the accepted ongoing goal, permitted next-action policy and total
allowance **before any method exists and across several method revisions/runs**. It cannot be
identified with one immutable method, one execution run, one attempt or one published service.
It may be represented by existing runtime ownership if the necessary asynchronous supervision
semantics are added; the product distinction alone does not prove a new database table necessary.
The selected physical representation is a narrow persistence commitment journal and account
origin, because the strongest runtime-root alternative changes more core lifetime semantics.

Delta's cross-review identified the complete root alternative, rather than declaring it impossible:
`StartOwnedChild` freezes an admitted method/account and returns a handle while the root retains
the child's lifetime; `Observe/AwaitOwnedChild` exposes checkpoint/terminal facts; a parallel
supervisor consumes criticism with explicit loop-carried evidence; root cancellation/drain keeps
the child owned after the start action has returned. This shares runtime and could make the
supervisory algorithm an ordinary reusable method. It also introduces asynchronous child escape,
checkpoint visibility and cancellation ownership into the core structured-concurrency model.
Dynamic `CallTarget::AdmittedPlanResult` alone addresses only the pin, not those relationships.

The selected commitment owner instead names accepted actions and exact associated runs, while
runtime retains all task/result/lifetime truth. Its advantage is the locus of change: a new
criticism policy changes the control-owned next-action contract and planning method, without
teaching every Call/Parallel/Repeat consumer a new asynchronous lifetime. The cost is that the
coordination policy is not itself an arbitrary editable workflow. Its planner/critic/evaluation
methods are ordinary reusable methods; the closed coordination policy defines when and under what
authority those methods may be requested. If users need arbitrary user-authored asynchronous
supervision itself, the root alternative becomes valuable and should be reconsidered openly.

`ControllerLifecycleOwner` remains the bounded-repeat/account assessment adapter. Existing
controller-wrapper definitions continue with their recorded semantics and ordinary authored
repeat use. New product continuous-work requests use the commitment coordinator; do not silently
install another wrapper loop around it. One-round/manual assistance is a policy of this owner,
not a second orchestrator. Shared account admission/settlement stays with persistence/runtime.
The new closed origin is `ExecutionAccountOrigin::WorkCommitment`; old controller/publication
origins preserve exact readers and historical digests.

Criticism triggers **assessment**, not automatic acceptance or necessarily pause/repair. The
policy may continue unaffected work while assessing a critique, or pause a named potentially
affected frontier before the model reads it. The assessed outcomes include retaining the original
conclusion with reasons, further investigation, prospective revision, escalation and stopping.
An equivalent canonical method/plan plus no new selected evidence counts toward a finite
`maximum_no_progress_rounds`; rejecting a critique with a new reason is retained evidence, not
proof of progress. The owner checks a declared progress rule against accepted artifact/revision/
outcome identities, never a model's “I made progress” Boolean. All attempts consume the same
remaining commitment allowance.

The control-owned no-progress comparator uses a bounded structural normal form: preserve ordered
sequence/clauses, task configuration, typed value references and exact external pins; alpha-normalize
generated internal identities consistently and omit layout, timestamps and explanatory reason text.
Evidence novelty uses the policy's finite accepted roles plus content/accepted-observation outcome
keys, not a new artifact/request ID. Duplicate observations or freshly phrased reasoning alone do
not reset the counter. This is a declared mechanical proxy, not a proof that arbitrary methods are
semantically equivalent or that new evidence is true. Original absolute round/time/allowance limits
apply regardless of the proxy's result. Test identifier-only rewrites, timestamp-only evidence and
new reasoning with unchanged accepted observations against the unchanged-work counter.

Faris's source review adds three necessary account/authority details. Current account
`Establish` requires `bind_run == controller_run`; therefore add an explicit commitment-origin
establishment transition with no fabricated run, not just another enum field. A new
`WorkCommitmentAuthorityBasis` freezes actor, grant, policy and revocation facts; the existing
`ExecutionAuthorityBasis` remains per actual associated workflow/run. Each association obtains
the ordinary fresh start decision and cannot gain read/control of arbitrary runs merely by
holding a commitment handle. Authority gains explicit commitment scopes/operations. Old grant
schemas have no such scope and cannot silently authorize new commitment mutation through a
historical wildcard; new grants must opt into the new versioned scope.

Current `ControllerLifecycleOwner::assess` specially accepts an inherited published-invocation
account only when its whole budget fits the inner marked controller's limits. Extend that exact
rule to commitment-origin accounts. Otherwise refuse local association before start; do not
pretend a new inner marker creates an independent allowance. The existing published-service route
may use its separately declared internal allowance and caller reservation under its own contract.
Generated commitment analysis/work methods use ordinary methods, not duplicate controller markers.
General hierarchical budget slicing is not introduced by this plan.

Two corrections make C's recovery/enforcement contract complete. First, every pending action
retains the **exact canonical internal create/start/proposal/apply command material**, including
generated source, inputs, expected guards and authority, as bounded immutable artifacts or record
fields before effects. IDs and packet digests alone are insufficient: a changed compiler after
restart must not reconstruct a different request under the old identity. Runtime receipt replay
still owns any command-specific delivery-time normalization; commitment recovery does not invent
a new one. This follows the [application receipt comparison](application-reconstruction.md).

Second, commitment pause/cancellation/deadline is enforced at owner-local final entry, not only
by a driver that may be asleep. A persistence-owned admission gate bound to the account origin records
`Open`, `Paused` or `Closed` plus immutable deadline/policy facts. Commitment transition and gate
change commit together. Runtime/host final-entry transactions read this gate and trusted boundary
time alongside the existing reservation transition. Paused/closed/expired commitments cannot
admit a new local capability entry governed by that commitment. The entry transaction traverses
actual accepted local publication/child ancestry; a separate immediate publication account must
not hide a local controlling commitment. A durably admitted effect may physically begin after
closure and remains subject to ordinary cancellation/uncertainty. A remote or published invocation already
accepted by another owner B has its own admission/account authority: closing A's commitment does
not synchronously fence B. If cancellation is delayed, B may still admit its child until B's own
cancellation, deadline or current-authority rule prevents entry. A retains the attributable outer
reservation and uncertainty; no new remote fence protocol is implied. Terminal reporting, artifact
settlement and authorized uncertainty resolution continue. Resume requires current authority and
remaining allowance; cancellation does not reopen. Pure gate decisions live beside existing
account transitions, while redb reads and checks the actual durable association/gate in the same
transaction. No caller-supplied Boolean can substitute for these facts. Pausing an affected target
run for criticism is a separate operation and can leave the commitment's planning admission open.

`AmendCommitmentPolicy` takes exact expected commitment sequence and policy version, an immutable
successor policy, and bounded explicit carry-forward action IDs/digests with their current frontier
evidence. Its authenticated authorization is separate from planner-result admission; the closed
planner decision enum cannot amend its own policy. Acceptance atomically pauses owner-local
admission at a new gate generation and retains the successor and affected-action disposition.
Each carried action preserves its original exact command, authority basis, obligations and target;
the successor adds a current restriction/authorization check, not rewritten history. Actions whose
unentered future work is affected remain held or receive an exact ordinary cancellation/proposal;
they need a newly accepted action before replanning or continuation. Already-entered effects keep
their actual obligations and uncertainty. A changed protected agreement needs a separately accepted
agreement and new run; this operation cannot weaken it in place.

Before reopening admission, control validates every listed carry-forward against current rights,
the successor policy and whole-allowance containment. It cannot carry an old run into broader
authority. A newly permitted scope can authorize a fresh future run with its own ordinary execution
basis under the same commitment account, leaving prior runs and liabilities intact. No automatic
rebase of a saved command or target follows from a policy change. An answer received for an earlier
question/version remains attached to that question and is not silently accepted for its successor.

The commitment's original aggregate cost, entry/round and time ceilings remain fixed for its
lifetime. Successor policy can impose a stricter future stop without refunding settled or reserved
usage; it cannot raise the original ceilings, reset elapsed time/rounds or discard unknown usage.
Budget top-up is excluded from this implementation contract. After exhaustion an authorized person
may explicitly create distinct new work and allowance, with the old stopped commitment and its
unsettled liabilities shown; the product must not present that as continuation on the original
budget. This fixed-ceiling choice meets U18's prospective requirements changes without inventing an
account-amendment transition or a hidden reset.

The public continuation/account view must distinguish pending decisions, eligible future work,
active associated runs, outstanding questions/approvals, exhausted/no-progress stops and unresolved
effects. It may derive these views from their owners. It must not persist another mutable copy of
each run's status. A fresh context can therefore resume the commitment from owned facts rather
than reconstructing a remembered supervisor conversation.

## U17 disposition: evaluated knowledge without invented executable methods

The earlier paired-result proposal was too restrictive. U17 explicitly permits author self-review
when labeled, retrospective assessment when labeled, and useful knowledge that has no executable
method. It requires reasoning, limitations and counterevidence. This changes the representation;
making `LearningDeclaration`'s existing method/resource fields nullable would obscure the distinct
finite claims and weaken protected publication.

The actual current selection owner confirms that this is more than renaming a comparison.
`select` in [daemon learning](../../../../../apps/daemon/src/host/commands/learning.rs) requires
one to eight exact run pages, resolves `selection.method`, requires an existing nonremoved managed
installation and authorizes `resource.inspect` through capability administration. Supersession
must preserve the workspace; changing the applicable method requires a retained `Promotion`
receipt. Approval also accepts only `Promotion` with an eligible method comparison.
[Promotion handling](../../../../../apps/daemon/src/host/commands/learning/promotion.rs) is actual
publication through `PublishedWorkflowService`, with an exact service template/generation and
separate publication authorization. Thus a legitimate research artifact with no run pages or
managed installation cannot enter this current knowledge selection, and a knowledge-only approval
cannot be represented by its current approval field. These are source-established restrictions,
not assumptions based on the Slotbook example. Keep their protected-profile meanings while
replacing their claim to be the universal knowledge contract.

Keep `control::learning` as the one evidence-selection/comparison/adoption application owner.
Use ordinary immutable artifacts and existing authenticated application receipts. Introduce a
closed `LearningSubject::{Knowledge, ExecutableMethod}` in new declaration/assessment documents:

- `Knowledge` binds exact artifact/version references for a lesson, explanation, decision or
  method description, its source inputs and applicability. Baseline/candidate outputs may be
  compared, or one result assessed against a stated criterion. No workflow identity, managed
  resource, deployment target or service generation is fabricated.
- `ExecutableMethod` binds exact canonical method revisions and evaluated invocation/run/output
  facts. The existing protected-publication profile retains required agreement, verifier, target,
  candidate-byte, resource isolation and repair-count obligations as a closed subvariant.

A `KnowledgeItemDocument` is an immutable artifact containing the bounded claim, applicability,
reasoning/source references, limitations and counterevidence. It is not a new mutable knowledge
database or a replacement for ordinary files. An item may point to a selected managed working
area/export when that is its real origin; unmanaged research artifacts remain first-class inputs.
`SelectSources` resolves authorized artifact identities and exact source pages for either subject.
It no longer requires `method` and `workspace` merely to select nonexecutable knowledge. Actual
resource associations retain their generation/read checks. Source selection remains separate from
an assertion that a lesson is correct or approved.

New `AssessmentDeclaration` names subject/role slots, criterion/rubric artifact, allowed reviewer
policy, evidence kind and comparison rule. Its basis is closed:
`Controlled { accepted_declaration, reserved_slots }` or
`Retrospective { assessed_sources, criteria_recorded_at }`. A controlled claim binds the criterion
before comparison; a held-out executable experiment additionally retains its existing stronger
pre-generation isolation requirements. Role slots, not unknown future candidate/attempt IDs, are
reserved before generation. Later authorized binding records exact candidate/version/output facts.
A retrospective record never acquires controlled status merely because its author calls it a test.

`RecordAssessment` binds exact inputs, compared versions/outputs, criterion, authenticated reviewer
and grant, judgment, reasoning, limitations and counterevidence. Reviewer relationship records
authorship and known participation/source relationships; unknown independence remains unknown.
Self-review is allowed unless the declaration explicitly requires a distinct/independent reviewer.
Agreement between several reviewers does not prove independence. The owner derives known author
relationships from retained provenance and retains declared/unknown relationships separately; it
must not fabricate stronger assurance than those facts establish.

The reopened transfer read found that “one result assessed against a criterion” did not yet choose
its judgment shape. Close that handoff explicitly: `AssessmentShape::SingleSubject` names one
exact subject/result role and accepts `MeetsCriterion`, `DoesNotMeetCriterion` or `Inconclusive`;
`AssessmentShape::Paired` names exact baseline/candidate and input roles and accepts `Better`,
`Equivalent`, `Worse` or `Inconclusive`. These are authenticated reviewer judgments under the
stated criterion, not assertions of objective verification. Each declaration selects the finite
rule `NamedReviewer` or `PairedNoRegression`, fixing its required reviewer/case slots. The latter
requires no worse declared case and at least one better case; it makes no statistical claim.
Missing/unknown required evidence yields `Inconclusive`; incompatible bindings are refused before
comparison. Contradictory required judgments produce `Disputed`, retaining the exact differences
rather than manufacturing consensus or picking the latest favorable reply. A declaration whose
required reviewer slots are not compatible with its finite rule is rejected at declaration time.

Evidence kinds remain `HumanJudgment`, `AgentJudgment` and `AutomatedVerification` with their
actual producer contracts. A favorable human judgment means that reviewer preferred/accepted the
exact result under the stated criteria. It is not a machine-check pass, statistical superiority or
universal improvement. The existing private trusted-verifier journal remains the owner of protected
effect checks; uploaded assessments cannot impersonate it. Mandatory technical checks remain
mandatory even if every reviewer prefers the candidate.
`ActorRef` itself is only an authenticated identity, not a human/AI classification
([authority identity](../../../../../crates/authority/src/identity.rs)). Human-review attribution
is an explicit authenticated reviewer attestation. Configured model-review attribution links the
verified producing invocation, profile and context; protected automated verification links the
trusted verifier journal. Do not imply a bearer token or an uploaded label proves a human's nature
or a model's actual execution. Known producer evidence and declared/unknown relationships remain
separate in the public view.

`Compare` derives the declared finite conclusion from exact assessment/run/verifier receipts. It
retains disagreement, failure, missing evidence and inconclusive outcomes. A simple declared
judgment rule can recognize an authorized preference or paired no-regression criterion; the owner
does not parse free prose into a score. `ApproveKnowledgeForReuse` is a separate authorized receipt
binding an exact item/version, supporting assessments, applicability and limitations. It makes
that item eligible for explicit selected reuse; it does not mutate existing runs, replace the
current method or grant publication rights. Method selection/adoption and publication remain
their ordinary separate operations. Automated approval is not inferred from favorable evaluation.
An approval may preserve an explicitly bounded negative, disputed or retrospective finding as
useful guidance, caution or an open question, with its reuse purpose and rationale recorded. It
cannot relabel the underlying verdict as favorable or remove contrary evidence from its basis.

Extend existing `LearningRequest` with these closed subject/declaration/assessment/knowledge-
approval variants and keep the protected method operations under their explicit profile. Move
domain orchestration from daemon learning handlers into the control owner as already proposed;
daemon continues authentication/wire/receipt adaptation. Historical v1 declarations/comparisons
retain exact protected-profile meaning and replay bytes. New writers emit the new versioned forms;
no old negative result becomes a new positive knowledge approval during migration. The source
selector's old method/publication fields lower only for historical records, not as hidden defaults
for new knowledge items.

Acceptance cases now include: author self-review visibly labeled; a declaration requiring an
independent reviewer refusing that same actor; a retrospective preference staying retrospective;
two agreeing reviewers with shared sources retaining that relationship; a negative assessment
remaining discoverable; a favorable knowledge judgment failing to authorize a protected effect;
and a selected approved lesson entering a later commitment's fresh-context packet with its
limitations and counterevidence still attached. These test the user's actual evidence semantics,
not an invented requirement that all knowledge become executable or statistically qualified.
