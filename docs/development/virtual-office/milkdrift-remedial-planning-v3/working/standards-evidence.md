# Ada: normative evidence and candidate mappings

Revision: evidence 1, 2026-10-10. This is research for phase 05, prepared before permission to
select a notation profile. It does not select a new runtime, authorize conversion code, claim
standard conformance, or establish that models author better workflows with a particular notation.
Repository inspection began at `908e7893f5dadb84d12712573c8daaa946829e39`.

## Sources actually inspected

I opened the official [UML 2.5.1 landing page](https://www.omg.org/spec/UML/2.5.1/) and
[BPMN 2.0.2 landing page](https://www.omg.org/spec/BPMN/2.0.2/) and followed their normative PDF
links. The web reader rejected the UML PDF because of its size; I downloaded both official PDFs
and extracted text with Poppler. I visually inspected UML PDF page 433, printed page 391,
figures 15.27–15.31, and BPMN PDF page 323, printed page 293, figures 10.110–10.111. The browser
reader also opened the BPMN PDF and its call-activity and gateway page images. Downloaded source
copies remain outside the repository in `/tmp/milkdrift-standards-ada`; they are not deliverables.

| Normative document | Identity and inspected clauses |
| --- | --- |
| [UML 2.5.1 PDF](https://www.omg.org/spec/UML/2.5.1/PDF) | OMG formal/17-12-05; 796 PDF pages. SHA-256 `416b57e1933780eb48bd60fe513e031da220c28a521bdd334a366bebc78a463e`. Read 15.1; 15.2.3.1–4; 15.3.3.1–6; 15.3.4; 15.6.3.1–2 and 15.6.4.1; 16.2.3.1; 16.3.3.1; 16.10.3.1–3; 16.11.3.4 and 16.11.4. Relevant printed pages 373–377, 387–391, 406–408, 435–436, 450–452, 472–474 and 479–480. |
| [BPMN 2.0.2 PDF](https://www.omg.org/spec/BPMN/2.0.2/PDF) | OMG formal/2013-12-09; 532 PDF pages. SHA-256 `d3d9258a70bf808c11ac52becea4ce645540a8b881117535f60a56fbf13e22f2`. Read 9.2–9.3; 10.3.6; 10.3.8 standard loop; 10.4.1 data associations; 10.5.1; 10.5.6 boundary handling; 10.6.4–5; 13.3.5–6; 13.4.1–2. Relevant printed pages 110–111, 182–185, 189–190, 220–222, 233–234, 276–279, 292–295, 431–435. |

The exact section references are research anchors, not a claim to have reviewed either entire
specification or its machine-readable metamodel. I did not inspect an executable standards engine
or test import/export. Current version claims and general compliance questions are out of scope.

## Compact normative grounds

UML distinguishes control tokens, object values, and execution. Forks copy offers; joins synchronize
using a condition, defaulting to all inputs; merges do not synchronize. Decisions select at most
one outgoing path, with unspecified choice when alternatives compete. Actions have typed pins;
calls may be synchronous or asynchronous. Acceptance waits use an execution context's event
dispatch. Loop nodes define setup, test and body, but have no prescribed specific notation.
Partitions organize modeled responsibilities and can constrain targets. Activity-final and
flow-final behavior differ. These grounds are in the inspected UML clauses above.

BPMN distinguishes sequence flow, participant messages and data associations. Data associations
do not carry control tokens, although unavailable source data can delay their execution. Parallel
gateways synchronize incoming paths; exclusive merges pass arrivals through. Complex gateways
have activation and reset behavior. Call activities reference reusable work. Standard loops can
specify a maximum. Messages support correlation; signals use broadcast communication. Interrupting
boundaries terminate the modeled activity. Ad-hoc subprocesses permit constrained flexible ordering.
These grounds are in the inspected BPMN clauses above.

These statements supply comparison grounds only. The mappings below are my proposed interpretations
of Milkdrift's needs and source, rather than additional promises made by either standard.

## Current meaning and candidate correspondence

Current node definitions are owned by
[node.rs](../../../../../crates/blueprint/src/model/node.rs), configuration by
[structured.rs](../../../../../crates/blueprint/src/model/structured.rs), and control/data edges by
[graph.rs](../../../../../crates/blueprint/src/model/graph.rs). Runtime details below come from
[the structured driver](../../../../../crates/runtime/src/engine/structured.rs),
[join completion](../../../../../crates/runtime/src/engine/completion.rs), and
[reducer execution](../../../../../crates/runtime/src/engine/structured/reducer.rs).
These are source findings, not newly executed tests.

| Milkdrift concern | UML-oriented candidate | BPMN-oriented candidate | Obligation before selection |
| --- | --- | --- | --- |
| Capability `Task` | Action/call with visible typed ports. | Task with explicit operation contract. | Keep capability generation, request schema, effect class and authority in daemon-owned data. A rounded rectangle cannot mean only arbitrary text. |
| `Branch` | Decision shape with a clearly defined priority rule. | Exclusive decision with explicit arm order. | Current runtime takes the first true arm in port-key order; renaming a port can therefore change selection. Either make this ordering visible and stable or propose a semantic migration. Do not silently depict an unordered mathematical choice. |
| `Fork` | Fork bar with named child branches. | Parallel gateway with a declared structured region. | Preserve branch identities, isolated mutable scopes and exact join ownership; test whether ownership is a necessary rule or merely current representation. A drawn split cannot promise shared writable state. |
| `Join(All)` | Labeled join bar. | Parallel gateway if the supported activation trace matches. | Current rule waits for branch termination, including failed outcomes. Separately show which results are acceptable; completion is not proof that all branches succeeded. |
| `Join(Any/FirstSuccess/Quorum)` | Join annotation plus an explicit loser policy. | Explicit specialized synchronization, compared against complex-gateway behavior. | Current source requests cancellation of active losers after its condition; a standard-looking gateway must not conceal this effect or pretend cancellation has completed. Do not equate quorum with an arbitrary resettable gateway. |
| `Reducer` | Separate transformation action after synchronization. | Separate task after synchronization. | Retain collection, deterministic first-by-branch-order, and capability-backed composition as different configured operations. “First” here is not first completion. A combined compositor interaction must expose both wait and transformation settings. |
| `Wait` | Time acceptance action. | Catching timer event. | Show durable due time and restart behavior derived from daemon records. This does not grant recurrence or arbitrary boundary interruption. |
| `SignalWait` | Event acceptance with scope and matching rule. | Correlated receive/catch form only where its contract matches. | Inspect current one-shot and broadcast modes separately. Display delivery mode and recipient scope; do not use a broadcast triangle merely because the Rust type contains “signal.” |
| `Repeat` | Explicit bounded structured loop region. | Loop activity with additional displayed bounds. | Preserve pinned body, iteration/time/cost limits, condition timing and limit disposition. A loop icon alone is insufficient. Current accepted body pins must survive later edits. |
| `Subworkflow` | Synchronous behavior call, with visible exact pin. | Call activity, with visible exact pin. | Collapse/expand must retain the same revision, bindings, execution and authority. An independently editable copy must be a different authoring action. |
| `Terminal` | Explicit result/end presentation after lifecycle review. | Explicit result/end presentation after lifecycle review. | Verify scope and active descendants before choosing a standard end glyph. Failure, cancellation request, completed cancellation and uncertain effects must remain distinct. |
| Control/data | Distinct control edge and typed object-port connection. | Separate sequence and data association layers. | Browser gestures must produce the same canonical bindings/dependencies as the public daemon authoring operation. Implicit readiness differences require a stated transform, not visual substitution. |
| Inter-owner service or peer | Call plus explicit identified owner and accepted-operation link. | Participant communication plus a separately identified call outcome. | A remote process is not automatically a workflow publication. Invocation-only callers see public contracts/results; no layout action confers graph access. |
| Ownership/placement | Labeled partition only for its declared dimension. | Pool/lane only for its declared dimension. | Keep workflow owner, execution host, principal, role and grant separate. The frontend may offer alternate projections, but dragging between regions cannot mint authority or move accepted work. |
| Human decision | Explicit authorized decision action. | Human task or catch form after contract comparison. | The daemon records who may decide, the exact subject/version, expiry/revocation and outcome. A human icon cannot substitute for those rules. |
| Artifacts/context/resources | Typed data view plus provenance and lifetime inspection. | Data view plus provenance and lifetime inspection. | Neither candidate mapping establishes branch privacy, artifact authenticity, retained unknown usage or permission to delete a resource. Keep those obligations with current owners or specify a complete replacement. |
| Live revision | Definition/history/proposal layers over a stable graph view. | Definition/history/proposal layers over a stable graph view. | I found no Milkdrift prospective-adoption contract in the inspected clauses. Do not infer impossibility in the standards; model the required agreement explicitly and test it. Moving a shape remains layout; replacing future work requires daemon validation and adoption. |

No major current node kind is deleted by this evidence account. Whether each remains an exposed
authoring construct, becomes a compound interaction, or is replaced remains a product-model
decision requiring loss/gain and migration evidence. The existing enum is a comparison input.

### Source refinement: alternative-path convergence

The complete validator trace narrows the initial concern. In
[validation.rs](../../../../../crates/blueprint/src/validation.rs), `validate_control_topology`
rejects more than one incoming control edge on a non-Join node; Join is bound to a Fork.
Thus directly converging exclusive alternatives is an authoring refusal, not established evidence
of an accepted graph that stalls. The runtime's all-predecessor rule is consistent with that refusal.
Current branch tests use separate terminals. An isolated production-library construction probe was
prepared for coordinator execution outside the repository; its expected positive control uses
separate terminals, while the shared-terminal variation is expected to refuse. No execution result
is claimed here.

A possible existing representation wraps the exclusive choice and separate terminals in a pinned
child, then continues in its parent. That must be tested as a complete alternative: its child
identity, interface, context visibility and prospective edit boundary are consequential. It is not
a free graphical merge. The eventual notation decision must compare this construction with a
scoped explicit merge extension; it must not freeze the gap as a harmless drawing limitation.

## Credible foundations to compare after the gate

**A selected UML Activity basis** could make typed inputs, explicit dependencies, calls and
structured synchronization the dominant explanation. Its strength for this product is that these
are directly relevant operator decisions. The cost is a visible policy for Milkdrift-specific
branch ordering, bounded repetition, durable effects, pins and live changes. It is not a claim
that the current scheduler is a UML execution engine.

**A selected BPMN basis** could foreground collaboration, callable work and human/event waiting.
Its strength is a disciplined vocabulary for interactions among owners. The cost is proving each
dependency, cancellation, join and data mapping rather than assuming similar diagrams execute
the same way. An executable BPMN subset may be appropriate if interchange becomes a concrete
reviewed need, but that would require a separately complete compatibility and execution program.

**Retain the current executable core with a standard-grounded notation profile** could preserve
working authority/recovery boundaries while exposing behavior through selected familiar forms.
Its strength is a smaller immediate semantic migration. Its risk is rationalizing every existing
restriction as a required extension and leaving humans with a proprietary language decorated
with familiar symbols. Each extension needs a real outcome, a contrary case and a readable label.

A hybrid is not automatically a compromise: it introduces its own consistency obligation. For
example, a canvas cannot use an unlabeled UML-style data arrow while relying on a different
readiness convention in another view. Select one executable interpretation per accepted operation;
multiple views may project it only with explicit, testable correspondences.

## Cases that can falsify a mapping

These are prospective modeled cases, not runtime executions or model benchmarks:

1. Two branch predicates are simultaneously true. Record the expected chosen arm before drawing
   the diagram, then rename an unrelated display label. If the chosen arm changes, the label was
   semantic and must be edited/reviewed as such; it cannot be a layout-only gesture.
2. Two branches terminate: one fails first, one succeeds later. Compare Any, FirstSuccess and
   All; then add a still-running external writer. The notation must predict when downstream work
   starts and why the writer may remain uncertain after a cancellation request.
3. Two alternative routes converge. A synchronization join is an invalid substitute for a merge
   if one route will never activate. Current support or a refusal must be demonstrated; do not
   draw a successful path that the daemon cannot author.
4. A reducer selects branch A by deterministic order although B completed first. A fresh reader
   must predict A, or the chosen label is misleading. A capability-backed judge is a new
   invocation with its own effect/usage/provenance, not an invisible gateway calculation.
5. An authorized targeted one-shot signal and a broadcast signal share a type name. Their wait
   recipients and durable delivery histories must differ correctly. An ordinary approval does
   not become a cross-owner broadcast by adopting a familiar glyph.
6. A child call is collapsed, then expanded after the viewer gains internal read permission.
   The accepted identity and service authority stay fixed. Expansion has no effect on runtime
   behavior, and read permission alone creates no edit/publication permission.
7. A run has completed A, is running B, and has pending C. A revision replaces C and adds D.
   Show old facts, the proposal and accepted future separately. A refused adoption must preserve
   the current run; an unknown effect from B must remain unresolved under either notation.
8. Export a diagram containing a nonstandard join or prospective-adoption annotation. A future
   interchange tool must preserve the declared meaning or refuse/clearly disclose unsupported
   content; silently dropping the extension is not an acceptable round trip.

## Evidence limits and next gate

The first account and [cross-examination](trial-ada.md#cross-examination-contribution--ada-2026-10-10)
remain separate from these standards findings. I have not selected a production visual grammar,
changed runtime semantics or evaluated models. The next step requires reviewed phase-02 outcomes,
phase-03 meanings and phase-04 owner/browser decisions. A fresh critic then needs to interpret
the proposed diagrams without an oral explanation; their disagreement should revise the actual
profile and dependent frontend/adoption work. No standards compliance, interchange compatibility,
human usability or model-performance conclusion follows from this research alone.
