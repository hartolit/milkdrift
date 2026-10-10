# The proposed executable workflow and diagram language

Profile **NP4**, selected planning recommendation under
[N1/r4](../../whiteboard/discussions/implementation/executable-notation.md), revises NP3 after
[U19](../intent-source-excerpts.md#u19--preserve-legitimate-adaptability-while-choosing-migration).
Structured Program is the default authored source. Versioned GraphNative construction, import,
copy, editing and prospective repair remain supported through the **same blueprint/control owner**
and checked execution plan. Conversion is explicit and optional; inability to convert an unrelated
part of a definition cannot by itself prohibit a lawful native repair. This is proposed architecture,
not implemented behavior, standards conformance or implementation authorization.

[Delta's comparison](../working/structure-comparison.md#u19-reopens-representation-and-removes-the-conversion-prerequisite-for-repair)
records actual current graphs, complete corrected-graph and structured alternatives, scoped runtime
behavior, five migration states, probe outcomes and dissent. NP3's exclusive new-region writer and
conversion-gated legacy editing policy are withdrawn. Earlier decisions remain historical in N1
and the comparison. No existing source or accepted execution is silently rewritten.

## One authoritative source per revision, one execution engine

`WorkflowSource::{Program, GraphNative}` is a versioned immutable source family. Each revision has
exactly one authoritative source variant. Blueprint owns pure source reading, validation, native
editing and lowering; control owns authorized application operations; runtime owns execution and
prospective reconciliation. Protocol, CLI, independent JSON, agents and Svelte use that same boundary.
There is no independently persisted editable execution graph beside a Program revision.

Program has stable element/region identities, Sequence, ordered Conditional clauses with matching
result mappings, Parallel with explicit completion policy and result exposure, bounded Repeat,
pinned Call, Await, Return and ordinary capability tasks. Array position is not durable identity.
Every external composition uses ordinary task requirement/context/authority; bounded collection
bindings supply selected results rather than another executor kind. Control order comes from
structure; typed value sources own data dependencies. The compiler derives readiness/causal indexes.
The person does not maintain both a value binding and a second matching data edge. Diagram and
accessible outline edit that same Program through complete structural/value operations; layout
remains outside revision identity.

GraphNative retains its versioned graph semantics and native mutation operations, including new
construction/import and existing ungoverned copy rules. It is not an opaque region leaf in Program,
an eligibility list of grandfathered IDs, or a hidden alternate daemon compiler. A source-aware
application owner dispatches to the appropriate pure editor and reports exact source version.
Current model convenience/importer commands retain their deterministic versioned compilation for
exact retries, including an underlying revision commit before the outer receipt was retained.
New default commands may emit Program; changing that default cannot change the old command's meaning.

Both source variants lower to one checked `ExecutionPlan`, which can represent their distinct
activation rules. It is never a separately editable persisted source. Runtime occurrences, attempts,
workspace values, child acceptance, authority and physical obligations remain their own facts.
Existing graph execution requires no tree decomposition. Pinned reuse may reference either source
version without making a new scope or grant merely because an editor renders a region.

## Standards principles with executable consequences

Program's Conditional uses substantive ideas from UML 2.5.1 §16.11.3.3: clauses, predecessor ordering
and each clause's mapping to the conditional's result pins. Milkdrift restricts tests to pure bounded
conditions and a total explicit order. Missing required results fail; optional absence is explicit.
This avoids unspecified selection among enabled alternatives or implicit null results. Structured
Sequence/Loop ideas also inform Program, while physical cancellation remains an observed durable
lifecycle. These are declared departures from general UML, not conformance.
[UML normative specification](https://www.omg.org/spec/UML/2.5.1/PDF).

BPMN's ordered exclusive routing and pass-through merge were evaluated as execution semantics.
An unrestricted merge can forward multiple arrivals; it does not imply one owned choice result.
Parallel token synchronization also needs explicit error routing for failed attempt outcomes.
Together with prospective adoption and scoped data/authority extensions, this makes full BPMN
execution a separate choice; no interchange or hidden second token engine is claimed.
[BPMN normative specification, §§10.6.2, 13.4.1–2](https://www.omg.org/spec/BPMN/2.0.2/PDF).
[Standards evidence](../working/standards-evidence.md) records inspected clauses/PDF identities.

## What a person sees and can predict

| Visual or authoring action | Saved meaning and outcome |
| --- | --- |
| Task with typed inputs/outputs | One full capability requirement/context declaration. Instructions cannot grant authority or replace the operation contract. |
| Ordered Program choice | First matching clause in explicit semantic order, distinct default, compatible selected results and one continuation. Renaming IDs does not alter priority. |
| Parallel with named branches | Isolated concurrent work, explicit All settled / Any completion / First success / Quorum policy, result exposure and loser treatment. Logical completion does not certify useful results or stopped effects. |
| Result composition | Deterministic selection/collection or an ordinary external Task receiving exact outcome/value references. Declared result order differs from earliest completion. |
| Pinned call | Exact owner/revision/interface and linked execution, for either source version. Collapse/authorized expansion changes no pin, grant or lifetime. |
| Wait | Declared deadline or event type/correlation/delivery/occurrence target. Browser timers do not execute work. Current limited SignalWait is not relabeled as an already implemented arbitrary correlated catch. |
| Bounded repeat | Explicit body, condition timing, iterations/budgets and limit disposition. Child/iteration allowance does not reset. |
| Return | Explicit output/outcome for that Program boundary. Resource/account uncertainty can remain visible after a useful result exists. |
| GraphNative method | Exact graph semantics, source version and native edit operations. Optional Program conversion is a separate reviewed action; it is not a prerequisite for native repair/copy/reuse. |
| Edit future work | Immutable successor with explicit selected base, actual affected frontier, applicable authority and reconciliation decisions. Ancestry alone, especially sorted merge parents, cannot identify the chosen adoption base. |

Choice identity/order remains accessible at every selectable zoom; compact views aggregate explicitly
rather than showing indistinguishable diamonds. Parallel completion and composition have distinct
statuses. All meaningful edits have keyboard/outline/form equivalents, with a source-aware native
path for graph methods. A private service keeps its public invocation breadcrumb; lost internal
read rights clear the private view. A lane projects stated ownership/placement and cannot relocate
work or mint authority.

## Results, prospective repair and recovery

A selected Program clause produces only its declared result mapping. Unselected data cannot become
a required continuation input through a hidden bypass; common ancestor inputs remain valid. Runtime
records selected clause, producing occurrences and result references atomically with the logical
completion boundary. Replay uses accepted facts, not today's inputs, and cannot repeat continuation
because a reply was lost.

If investigation is complete and repair pending inside the selected clause, an authorized successor
can change pending repair while retaining the accepted decision and investigation. Stable descendant
identity and actual dependencies matter; changing one descendant must not force replacement of its
entire active enclosing region. A started occurrence retains its original governing plan. Its future
configuration may nevertheless change where existing policy permits: current `ChangedActive` with
`FinishCurrentThenAdopt` uses `UseNewOnNextInvocation`. Immutable history does not require permanently
freezing every later use of that definition element. Changed completed none/read-only work may also
have a new future definition without rewriting its earlier result.

Unsafe dependencies affecting started descendants, retargeted current ownership, incompatible pins,
uncertain side effects and protected boundaries retain their actual refusal/authority/remediation
rules. The new source must not impose a blanket active-node-change refusal. Nor may it change an
accepted selected route or already consumed value retroactively. Candidate validation and exact
frontier/sequence reconciliation remain different checks.

A nested first-success result may be usable while a losing writer remains uncertain. The region
retains that writer, its hold and reservation. Entry checks still control later work. A restricted
published call may withhold public completion until its own obligations settle. Logical continuation,
workflow completion and physical quiescence remain separate inspector facts.

## Verified conversion is distinct from an intentionally changed successor

`PrepareProgramConversion` proves equivalent control/data/scopes/results/ownership for an exact
GraphNative definition and creates a new immutable Program revision with origin correspondence.
Old lexical branch priority becomes explicit clause order. Unsupported translation refuses with
precise relationships and leaves native execution/edit/repair/reuse intact. No lossy save, copied
fingerprint or inferred grant fills the gap.

A deliberately changed Program successor has a different proof: validate the proposed future source,
retain selected ancestry/identities, map actual scoped history and apply ordinary reconciliation.
It need not be equivalent to the entire old definition. For example, while a prefix Wait is active,
a new successor can preserve that occurrence and intentionally fix an entirely unentered crossed
fork/join portion. That is prospective repair, not an equivalent conversion of the defective graph.
Old history need not be embedded as an executable graph leaf in the new Program to remain inspectable.

Native governed graph successors use the original v1 agreement validator directly. Conversion into
Program under an old agreement additionally uses blueprint's pure `agreement::legacy_v1` producer
of `LegacyAgreementView`: current checked source/plan, exact origin and complete checked source-to-
legacy correspondence reconstruct the original protected node/edge/config/interface shape. The old
fingerprint is not only activation equivalence. Every relevant entry/link must be accounted for;
copying old protected facts while ignoring changed current source is forbidden. Apply unchanged
v1 Task-scope/count/envelope rules and accepted digest. The view is ephemeral compatibility data,
not another persisted graph. Native Program agreements have versioned stable-element obligations;
accepted old agreements are not silently upgraded.

The selected native `ProgramAgreement` v2 names one or more bounded stable Sequence regions whose
direct bodies may contain only ordinary Tasks. Its scope fixes exact allowed capability requirement
envelopes, maximum total task count and cumulative revision limit. Seal the enclosing source skeleton,
scope identities, immutable scope input bindings and input/output names/schemas/requiredness; replace
each permitted body with an opaque scope slot when computing that protected fingerprint. Verifier,
acceptance, effect, control structures and pinned calls stay outside editable bodies. Compiler-generated
plan-node IDs are not new authority facts.

Within a permitted body, task configuration, ordering, additions/removals, internal value references
and mappings from internal values to the fixed scope results may change prospectively. External
consumers bind scope results; direct external references into the body and undeclared body reads
outside its input boundary are forbidden. Output signatures remain fixed while the implementation
producing them can improve. Already published results/entered occurrences retain their old facts;
reconciliation rejects or otherwise handles unsafe reordering/dependencies under the existing policy.
The accepted v2 agreement cannot be replaced during a run. A converted v1 prefix may span several
graph fragments and continues using LegacyAgreementView; forcing it into a narrower native scope
is not a migration requirement or authorization to change its agreement.

An old protected root with completed `repair.begin` and pending `repair.end` must retain permitted
Task-only investigation/repair through the native path. Certified conversion cases must also support
that positive repair under the same agreement. Changing protected verifier/effect/boundary facts still
refuses. Inherited child adaptation remains separately refused when its accepted enclosing agreement
pins it. A converter cannot authorize that change; cancellation/new work is not relabeled repair.

| State | Required preservation |
| --- | --- |
| Closed historical work | Original source/event/receipt identity and inspection/replay; explicit native successor or lawful copy/reuse never edits history. |
| Reusable inactive definitions | Native create/import/edit, existing copy/qualification and exact run/call remain supported. Conversion is optional; inactivity does not justify losing future repair. |
| Active with unentered work | Native future edits use current classification/actions without a whole-conversion prerequisite. Program-native or intentionally cross-source successors use the same guarded engine. |
| Started/uncertain work | Occurrence pins, evidence, reservations and holds remain owned; applicable future-change/remediation rules decide safety. Representation alone adds no refusal. |
| Governed/protected work | Original agreement and native root Task repair remain; optional conversion proves the additional representation correspondence. No authority or protected-boundary relaxation. |

## Actual non-nested cases and the boundary of R

The [production diagnostic](../working/compatibility-runtime-evidence.md) proves the validator accepts
crossed fork/join ownership. Proper nested All succeeds; crossed All remains Running after 32 ticks;
both proper nested Any and crossed Any raise an `InvalidHistory` cancellation-owner error. These are current defect/stall
observations, not useful demonstrated extra graph expressiveness. The same diagnostic successfully
applies a pending Task change before that unchanged crossed graph while preserving an active Wait.
It directly disproves whole-definition conversion as a necessary condition for that repair.

The error arose while `PlanTransition::push_event` applies a prospective event to its projection,
before adding that event to the eventual commit. This observation does not establish that corrupt
history was persisted; the error text alone must not be used to claim that.

Fork branches ending at direct terminals without a Join are accepted and succeed in the probe and
existing tests. Their unjoined outcomes and old terminal output-selection rules remain GraphNative
behavior; an ordinary joined Parallel is not declared equivalent automatically. No special Program
DirectTerminals mode is added merely to claim total conversion. New work asking to await all parallel
branches, fail on any failed outcome and select a particular result can express that explicitly with
Parallel outcome handling and Return. It does not inherit an incidental historical last-terminal rule.

Retain original GraphNative validation; there is no blanket stricter admission or eligibility
registry blocking unrelated native repair. Program validates its own structured source. Crossed-All
stalled topology remains a precise diagnostic and may be deliberately repaired while unentered;
accepted scopes cannot be relocated by inference. The Any failure is a separate ordinary nested-
cancellation defect: the driver sees cancelling ancestors while projection's current predicate
checks only the immediate branch. P01 must give both one private bounded ancestor-cancellation
evidence predicate over accepted scope/fact ownership, including timer/wait/retry consumers. Preserve
existing nonbranch cancellation sources; unrelated scopes must refuse. No invented historical
cancellation/Join events or loading ban replaces this correction. No production fix was made here.

## Cost, retirement and required proof

R as default plus GraphNative keeps a genuine version-specific source validator/editor/compiler and
its tests. It avoids duplicate runtime/authority/persistence ownership, but that maintenance cost is
real. M, the corrected canonical graph with complete compound authoring, remains the strongest
alternative: less conversion risk and one source editor, at the cost of stored graph plumbing and
reconstructed ownership. Neither option has measured human/model authoring superiority. Valuable
new work must not be shunted increasingly into a legacy-only category; such a finding reopens R/M.

Native authoring retirement is not scheduled. A future reviewed change must first prove equivalent
supported construction/import/copy/edit and legitimate live repair for affected active/reusable work,
with exact agreements and recovery, and announce the supported writer/client transition. Historical
reading/replay/execution is a separate lifecycle. There is no automatic expiry or ID registry;
where the proof fails, retain the path or reconsider M rather than silently remove adaptability.

P01 must run D1–D8, the specialist future change, overlapping predicates with renamed IDs, failed/
missing results, uncertain losers, pinned reuse, pending and permitted active future changes,
restart, waits and exhaustion through real public/runtime owners. Preserve native model-editor,
prompt-sequence, protected-root repair and parallel/call cases. Test certified conversions positively
and refusals without losing native repair, plus native create/import/copy and cross-source deliberate
successors. A blanket conversion refusal does not satisfy promised positive conversion cases, and
a successful finite corpus is not a whole-language proof. Unknown source variants remain read-only;
supported GraphNative is not classified unknown. Exact old command retries preserve their compiler
version and selected base. No frontend/model usability, paid comparison or standards interchange
was performed for this profile.
