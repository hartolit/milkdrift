# Agent-directed work: independent reconstruction

Elin's reopened investigation at `ff818e5`. This is a recommendation under review, not approved
implementation. It uses the supplied reopening request, original selected
[intent excerpts](../intent-source-excerpts.md), current source and history. No other fresh
reviewer's recommendation was read before this initial position was recorded. Source inspection
is evidence of current behavior; the proposed operations below are modeled, not executed.

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

**Selection: B, with an explicit finite boundary.** Milkdrift should own a complete assistance
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
