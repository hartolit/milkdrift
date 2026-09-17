# 00 — Adopt the direction throughout the project

## Assignment

Execute the first assignment of the adaptive-hosts sprint. You own direction adoption and sprint
coordination for this assignment. The user has explicitly requested these changes and the subsequent
implementation sprint. Do not turn this into another open-ended debate, request repeated permission,
or begin a new unrelated roadmap. Resolve the design sufficiently for 01–05 to implement it.

The outcome is one coherent account of Milkdrift's product, ownership, supported behavior, and
remaining implementation—not another competing design document beside unchanged canonical files.
This assignment changes the project's accepted direction and instructions. It does not claim that
editing prose implements independent hosting, managed environments, or adaptive-method protection.

Read [the sprint README](README.md) and [the complete discussion](discussed-direction.md). Follow
[AGENTS.md](../../../../AGENTS.md) in its required order. Select and apply both
[implementation practice](../../practices/implementation.md), because you are making ownership and
compatibility decisions, and [documentation practice](../../practices/documentation.md). Use the
[current workflow](../../workflow.md) and [office procedure](../README.md).

## Establish the actual starting point

Record the current commit and working-tree state. The planning baseline is
`855f8ecbb1007baa2a91006384fa322af40ebef9`; a newer checkout must be assessed, not reset. Preserve
other contributors' changes. Earlier downloadable design patches may or may not be present.

Read the existing whiteboard overview and managed-execution discussion, the architecture and product
owners, relevant ADRs, and the implementation routes named in the discussion's source index. Trace
at least: daemon startup, local workflow prepared entry, peer acceptance/worker recovery, adapter
context and output publication, control proposal classification, acceptance gates, controller
account admission, and client/CLI routing. Read the corresponding tests, not just their names.

Search the workspace for assumptions that every execution has a local run, every context is a
workflow manifest, every durable producer is run-shaped, and every meaningful revision requires
unconditional approval. Also locate the existing feature freeze and statements that imply workflow
publication, isolation, or learning already exists. Distinguish intentional current limitations
from stale product intent. A compilation dependency alone does not establish a runtime ownership
requirement.

## Adopt these decisions

Make the product's central purpose explicit: agents develop, adapt, and reuse methods of work;
independent hosts and managed tools support that purpose. Adopt the discussion's relationships:

1. One workflow system supports differently scoped freedom. A blueprint defines a reusable method;
   revisions are immutable; runs record actual prospective adaptation. There is no global static
   versus dynamic engine split and no unaccountable rogue mode.
2. A dependable method binds responsibilities, process/result obligations, effect prerequisites,
   and permitted adaptation. Repairing a run, improving a reusable method, and changing a published
   agreement are distinct authorized actions. Permission to adapt cannot rewrite its own limits.
3. A published version binds an exact starting blueprint and adaptation policy, not necessarily
   one frozen graph for the entire invocation. Accepted calls retain their version and actual
   revision lineage. New promotion changes future selection only.
4. The independent host supports real direct calls without fabricated workflow identities. Origin
   and transport differ. Shared prepared execution must preserve local runtime-owned history and
   accounting, and reuse/generalize serving-side durability without another local attempt ledger.
5. Managed resources outlive operations. Durable identity, use holds, approved configuration,
   intended changes, recovery, and deletion policy are required. Rootless Podman plus systemd/Quadlet
   is the initial Linux mechanism, not a universal execution condition or a custom supervisor.
6. Requirements are enforced both during permitted adaptation and at the consequential effect.
   Direct invocation cannot bypass publication prerequisites, inherited grants, or applicable budgets.
   Verifier trust and candidate/configuration identity are distinct from an artifact checksum.
7. History, reusable methods, consistent acceptance, reproducible setup, and identical output are
   different claims. Learning proposes and evaluates a method; it does not turn a successful trace
   into an automatically promoted universal recipe.

Keep native trusted execution, external endpoints, ordinary files, exact artifacts, scoped knowledge,
local-first operation, shared administration, and future cross-platform clients in the product.
Do not replace these requirements with a mandatory container platform or inference implementation.

## Resolve the operating rules before implementation

Write accepted ownership decisions, not an inventory of unanswered questions. Use concrete traces
for the following cases and record why each owner is necessary:

- Direct model/process call versus local workflow attempt versus a workflow request served remotely:
  identify who owns acceptance, preparation, final entry, result artifacts, cancellation, and restart.
- A model request submitted directly has explicit authorized inputs; a delegated workflow request
  retains its governing context and account. Explain how credentials prevent reclassification.
- A resource is created successfully, but its result commit fails; a resource update races with
  active work; a removed resource's operation detail is later compacted. Identify retained facts,
  expected versions, transaction boundaries, and evidence needed to resume or preserve uncertainty.
- A method adapts to repair a failed candidate. Removing a check, changing a verifier, replacing the
  target, or calling the raw effect must not authorize publication. Explain where useful automatic
  edits are allowed and which agreement cannot be weakened by those edits.
- An invoke-only caller uses a workflow-backed deployment capability without receiving production
  secrets or broad internal rights. Bind an explicit service-execution identity and narrowing policy
  to the public operation; do not require every caller to hold the unrestricted internal grant union.
  Conversely, do not let a method proxy untrusted arbitrary targets or reset a caller's account.
- A published invocation survives a crash between acceptance and internal run creation; a worker
  with a single available execution slot calls a method that itself needs workers. Resolve durable
  linkage and nonblocking continuation without a second scheduler.
- A proposed lesson becomes a candidate blueprint, is evaluated on separate inputs, and is promoted
  for future calls. Identify who owns proposal, evidence, selection, and each variant's mutable data.

### Define one shared application and evaluation case

Before handing off implementation, select and name the demonstration application that connects
managed setup, failed deployment, adaptive repair, method learning, and product variations. Record
its specification in one maintained example location, with a reference from `handoffs/00.md`.
Do not let 02–06 invent separate applications or independent success demonstrations for these steps.

Settle these concrete facts during 00:

- The application's purpose, controlled deployment target, original input, and initial candidate's
  observable failure. Explain how unclear planning or contradictory requirements can produce that
  failure; an unrelated planted defect does not establish a planning lesson. Label seeded fixture
  behavior separately from behavior actually observed in a model-driven run.
- The required verifier and its trusted owner, exact checks, candidate/configuration binding, and
  evidence required before publication. Identify which requirements and verifier settings the
  repair and learning agents cannot change.
- The editable part of the method and the authority available to repair this run or propose a
  reusable improvement. Specify the problem and permitted changes, not the model's answer. The
  suggestion in 05 to introduce a planning responsibility is a hypothesis, not a mandated graph
  change, prescribed prompt, or required node name.
- The evidence connecting the source run's failure and repair to the proposed method revision.
  The proposal must cite that run's relevant decisions, contradictions, and verification results.
  A repaired application alone does not prove that the method improved.
- Concrete reuse inputs separate from the source case, including at least two meaningful product
  variations. Evaluate the baseline and candidate methods on the same declared evaluation inputs
  with isolated working state and comparable declared tools and limits. Keep those inputs and
  expected answers out of the proposal agent's source context; candidate methods receive their
  task inputs normally when evaluated.
- An observable improvement criterion, its measurement, and its threshold, fixed before candidate
  selection. For example, reduced contradiction-driven repair rounds while still satisfying every
  unchanged required check could count; a renamed stage or an extra planning artifact would not.
  Define failure and inconclusive outcomes too. Do not require or fabricate a positive learning
  result when the evidence does not meet the criterion.

The trace must remain inspectable: original run evidence → candidate method proposal → separate
baseline/candidate evaluations → promotion or rejection → independently recorded variations.
Later agents must use the same example and traceable identities. Do not retune evaluation cases
until a preferred candidate wins or generalize a small demonstration into a universal quality claim.
This is an example and acceptance decision, not authorization for another learning subsystem.

### Resolve nested resource ownership as well as worker availability

Extend the single-worker published-call case to a parent and its authorized child using the same
managed working area. Releasing a worker slot is insufficient when the parent still holds the
exclusive editing access the child needs.

Distinguish two responsibilities: protecting the exact resource generation from removal or
replacement while accepted work depends on it, and deciding which operation may currently mutate
its files. A waiting parent may retain the first without retaining an editing claim that blocks
its own authorized child. Choose an explicit ownership handoff or narrowly delegated child-use rule
within 02's resource owner and 04's durable invocation/run linkage; do not add another scheduler or
an unrestricted reentrant lock keyed only by a shared actor or workspace name.

Bind child access to the exact accepted parent/child relationship, resource generation, and
inherited authority. Parent writes must be suspended or refused while the child owns mutation;
there must still be at most one active mutator. Resume parent writes only after child use is safely
settled and editing authority has been reacquired or returned under the chosen rule. A wait, timeout,
lease expiry, or cancellation acknowledgement alone does not establish that the child has stopped.

Specify one integrated acceptance test, implemented across 02 and 04 and rerun in 06:

1. With one execution worker, a parent starts a workflow-backed call whose child must edit the
   same managed working area. The child makes observable progress while the parent waits.
2. During child mutation, a parent write and an unrelated conflicting write cannot enter, and
   removal/replacement remains controlled by the retained resource-lifetime protection. Unrelated
   work on other resources can still proceed.
3. After proven child completion, the parent can regain editing access and continue without
   simultaneous writers, duplicate work, or a leaked hold.
4. Interrupt at the ownership handoff and while the child is active. Restart and cancellation must
   preserve the exact lineage and holds; uncertain child use must not permit unsafe parent writes
   or deletion. Define the existing inspection and authorized resolution path for blocked work.

Record the owning transition/transaction, recovery evidence, and test allocation in `handoffs/00.md`.
These are required design and acceptance clarifications, not a claim that the current implementation
already contains a deadlock. Do not change the rule into one operation per host or environment.

Prefer explicit protected scopes and effect checks over arbitrary program-equivalence claims. A
new concept earns a representation only when it owns a real invariant. State necessary module or
crate moves with their dependency reason; do not promise a small change or require a crate per noun.

For every changed durable or public boundary, decide current ownership, affected producers/consumers,
versioning, preservation/read behavior, and which assignment implements it. Cover invocation origin,
context and artifact ownership, execution records, authority/account delegation, resources, adaptive
agreements, and published invocation linkage. Use the existing compatibility policy; do not assign
future numeric versions blindly or invent a migration that has not been designed. Record the concise
impact map in this assignment's handoff, with durable decisions in appropriate ADRs.

## Update the canonical owners

Make integrated edits in the existing files, following their actual responsibilities:

| Owner | Required change |
| --- | --- |
| `docs/product/vision.md` | Explain adaptable reusable methods, scoped freedom, protected commitments, independent hosts, managed setup, publication, and learning in one product narrative. Preserve valid existing intent. |
| `docs/architecture.md` | Integrate agreed ownership, direct/workflow origin, prepared entry, resource lifecycle, adaptation/effect enforcement, publication linkage, and dependency direction. Clearly distinguish intended additions from current composition. |
| `docs/product/roadmap.md` | Replace the blanket freeze with this explicitly authorized ordered sprint and its finite exclusions. Preserve restrictions on GUI, inference, and unrelated expansion. |
| `docs/product/status.md` | Keep implemented and qualified facts accurate. Add the specific unimplemented boundaries as limitations where useful; do not mark planned work complete or copy the roadmap. |
| `docs/decisions/` and index | Add only durable boundary/security/compatibility decisions that need rationale. Use the next available identifiers. Explicitly relate to earlier ADRs rather than silently contradicting them. Proposed compatibility is not a new supported reader. |
| `AGENTS.md`, contributor/development entry points | Remove conflicting scope instructions and clarify shared durable principles only where necessary. Preserve reading order and existing quality rules; do not duplicate the vision here. |
| Relevant package READMEs, guides, source documentation, examples | Correct explanations that would misdirect the implementation, without describing future symbols or behavior as existing. Change no serialized examples unless actually supported and tested. |
| Virtual-office and whiteboard | Register this sprint, mark the relevant topic assigned to it, preserve unrelated discussions, and reconcile the existing synthesis instead of adding another competing topic. |

Use subtraction as well as addition. Do not leave old and new definitions of a blueprint, host,
workspace, publication, or authority equally normative. Historical ADR text can remain with an
explicit supersession note; implementation limitations must remain visible. Keep future GUI and
platform ambitions without making them prerequisites for this sprint.

Do not rename functioning source APIs solely to match prose or add unused future interfaces. If a
necessary correction changes executable code, follow the full gate; otherwise keep this assignment
an honest design/documentation adoption. All substantive execution changes belong to their numbered
implementation owners, not untested scaffolding committed here.

## Verification and acceptance

Review each relevant rule against source, consumers, and tests. Recheck the full diff for duplicate
truth, misleading current-support claims, broken links, and stale freeze language. Confirm that the
remaining prompts reflect the adopted boundaries; clarify their wording when needed without
silently removing requested features or inventing another sprint. In particular, align 05's learning
example with the shared application without prescribing the proposed solution, and carry the nested
resource-ownership test into 02, 04, and 06. These are clarifications within the existing assignments,
not new phases or a reason to regenerate the sprint.

For prose/planning changes run:

```sh
cargo test -p milkdrift-evidence --test repository_contracts --all-features documentation::
git diff --check
```

Also run the checks required by the actual changed content under the workflow policy: comment-only
Rust changes need formatting, affected doctests and warning-denying rustdoc; executable/schema/example
changes require their stronger checks. Do not report source inspection as test execution.

Read the result as a newcomer: can they distinguish the method, agreement, host operation, installed
resource, and evidence; explain useful adaptation and a refused bypass; and locate the single owner
of each rule? Can an implementing agent begin 01 without rereading chat or deciding again whether
these features are authorized? A list of terms without these relationships is not sufficient.

## Completion and handoff

Update `handoffs/00.md` with adopted decisions, concise affected-boundary/compatibility map, canonical
locations, exact checks, and any unresolved conflict that actually prevents execution. Include the
shared application's specification location, failure/verifier/editable-method decisions, separate
reuse inputs, fixed improvement criterion, and source-to-evaluation evidence links to be produced.
Also include the parent/child resource-ownership rule, interruption/resolution behavior, and which
assignments own each integrated test. Do not mark either case as executed during this design task.
Record source and resulting commit identities. Update the sprint assignment table accurately. The
stop condition is an adopted, internally consistent direction and executable assignment sequence,
including these two settled decisions—not merely a new whiteboard essay and not the implementation
of 01 itself.
