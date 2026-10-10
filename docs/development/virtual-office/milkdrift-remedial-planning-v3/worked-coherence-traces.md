# Worked coherence traces — examples of the review method

These are **constructed review cases**, not reports of new defects or prescribed architectures. They illustrate missing intermediate meaning that a smooth A-to-D description can conceal. Establish actual current behavior in 01; compare alternative designs using the same facts.

## Case A — “Call this reusable workflow over there”

**User outcome:** use a known method owned by another team without rebuilding it or obtaining its private deployment privileges. View the result and understand an interruption.

A tempting simplification says: collapse any workflow to one node, attach a host name, then retry when the host is offline. That leaves several unanswered questions.

| Point in the path | Fact the next step needs | Question that can disprove the simple story |
| --- | --- | --- |
| Select method | The exact callable interface and owning service are known. | Is this an editable definition, a pinned call, or a published service? Can the caller see its internals? |
| Authorize request | The caller may invoke that public operation with these inputs. | Does invoking it accidentally require or grant raw deployment authority? |
| Accept request | The owner retains the exact request identity and accepted version. | Was a lost reply a refusal, or could the request already exist remotely? |
| Run internal work | Internal permissions, obligations and limits apply to this accepted call. | Can a nested call or revised plan reset the caller's limits? |
| Lose connection | Neither endpoint invents an outcome it cannot establish. | Would switching hosts repeat a possibly completed external effect? |
| Recover | The same accepted work is inspected or resumed where supported. | Are its outputs and internal graph subject to different read permissions? |
| Edit or retire library version | Future selection changes without silently repinning accepted work. | Which run, saved definition or service generation actually changes? |

Now change only one fact: the user instead wants an ordinary permitted build operation on an execution-only host. If the design forces service publication, identify why. If it has no reason beyond the visual metaphor, the metaphor is creating a false restriction.

A coherent design may present both as ordinary-looking callable nodes. It must still preserve the actual relationships where they matter. Neither “everything is a method” nor “every relation needs a separate menu” follows from this case.

**Independent checks:** one reviewer describes ownership and permissions; another describes exactly what the user can observe and recover. Compare their claims at each boundary. Require a positive successful call as well as refused input and lost-reply cases.

## Case B — a compositor that looks simple but cannot continue

**User outcome:** pursue two approaches, compare useful results, and continue with a justified selection while keeping the work inspectable.

Ask what the compositor promises: wait for both terminal states, both successes, first completion, first success, or a quality comparison? If branch B is deliberately not selected, did it never start, get cancelled, or remain unresolved? Those facts determine what can be awaited.

Then add a legitimate second feature: the parent workflow calls a method whose child edits the parent's managed checkout. The parent waits for its child but also retains exclusive editing access to the same files. Releasing an execution worker does not release that editing claim. Each component may pass its own tests while their composition cannot progress.

Finally, add a request to remove the checkout during uncertain child execution. Merely releasing every hold to cure the deadlock permits destructive removal. Retaining every hold forever prevents legitimate work. The design must distinguish lifetime protection, editing authority and evidence that use actually stopped.

These are distinct responsibilities, but not automatically three visible nodes or three new subsystems. Compare an explicit handoff, a different resource ownership model, or isolated child storage against the same successful, interrupted and concurrent paths.

**Changed-future variant:** while one branch has completed, revise the unstarted branch. Can the new join/reducer consume the preserved output without reinterpreting its meaning? Does an already accepted obligation still require the omitted work? The question is more precise than “support live editing.”

## Case C — a generic node versus a generic experience

**User outcome:** choose useful work, supply inputs, connect its results, and make the method respond to what happens.

A generic appearance does not imply that inputs are always text or that a model decides execution semantics. A tool can consume files; a wait can outlive a client; a signal can require exact correlation; approval can come from a different person. One `Node { instructions }` type may hide these distinctions rather than eliminate them.

The converse mistake is treating every engine variant as a separate human concept. A request to “run both and collect their results” may be one editing action that produces several well-defined elements. Analyze whether the user must manipulate each element, whether the daemon can build that arrangement, and how later edits preserve its meaning.

Compare the diagram, the non-graph accessible editor, the agent's authoring document, the daemon's validated definition and the execution history. Ask whether the same authorized edit has the same effect through each route. Visual collapse must not change execution; moving an item must not revise its semantics; a hidden implementation must not become editable merely because the UI can draw it.

The current ordinary editor is narrower than the full blueprint model. That is a source fact to inspect, not proof that the core is wrong or permission for Svelte to discard unsupported fields. The remedy might be a broader public authoring operation, a different core representation, or explicit viewing without editing. Choose on evidence and the required outcome.

## Case D — “learning works” but no reusable improvement has been established

**User outcome:** an agent's experience with a failed development method helps later work, without silently changing other callers' agreed methods.

The original run may fail because of unclear requirements, a missing API, unavailable hardware, poor model output, or a wrong acceptance check. Adding a planning node is not necessarily the remedy. Trace which evidence distinguishes these explanations.

A proposed method revision must cite the relevant failure/repair evidence and state what should improve. Hold the evaluation criteria fixed before selecting the winner. Separate source cases from evaluation inputs and retain failures, costs and missing measurements. A controlled fixture can test the comparison mechanism; it cannot establish that a live model discovered a useful improvement.

Positive results still need a scope: what changed in this run, what became a reusable definition, what was published for future calls, and which independent variants were created? An evaluated improvement must not automatically enlarge authority or rewrite past acceptance.

**Dogfood variant:** apply this to the present planning work. A critic finds a hidden ownership conflict; the coordinator revises a decision and downstream assignments; a fresh-context agent must explain and execute the revised scenario without the original author's help. This tests whether the planning method preserved the lesson, not whether everyone used the same terminology.

Do not require the current Milkdrift engine to run this review. Use the process as a demanding product scenario and identify actual capability/public-operation gaps. Do not build a separate review-management platform to satisfy the example.

## Case E — a tentative explanation becomes a requirement by repetition

**Constructed reasoning failure, not a reported Milkdrift defect:** a user suggests that a reusable workflow might bridge machines, then asks whether that restriction makes sense. An assistant calls it a method gateway. Architecture notes cite that phrase; a GUI planner concludes all remote work requires method publication; a critic cites both documents as independent confirmation.

Trace backward. Did the user require publication for all remote work, or seek reuse and controlled remote access while questioning the mechanism? What does the actual direct/peer path allow? Do the documents provide new evidence or repeat one interpretation? Label each claim's origin, present standing and support separately.

A candidate correction might separate the needs or unify their user interaction. Neither is decided by discovering the citation error. It must still preserve any useful restricted-service behavior, explain authority and interruption, and compete with alternatives on the reviewed goals.

Now suppose another proposal says “use a hybrid because BPMN cannot express our gateway.” Which exact standard rule and reviewed user behavior establish that mismatch? The outcome might be a justified extension, a corrected standards reading, or retirement of the gateway assumption. The word “hybrid” does not decide.

**Dependency check:** if the mandatory-gateway premise is withdrawn, recheck the method definition, remote-operation public API, notation, permission presentation, onboarding and implementation assignments that required it. Do not discard unrelated resource or recovery evidence. If the premise remains disputed, do not certify those dependent tasks ready.

**Review-method check:** ask a fresh reader to locate the original uncertainty and explain the best surviving proposal and objection without the author's oral corrections. In a separate labeled exercise, change only the caller's right to inspect internals and check which recommendations actually depend on it. Do not turn that hypothetical into a claim about current code.

## How to extend these examples

For each high-impact decision, create a new countercase not copied from these examples. Vary a meaningful assumption: owner, grant, input shape, nesting, ordering, completion evidence, writable resource, or definition version. Explain why the variation is within intended scope.

Trace both permitted progress and refusal. State what observations distinguish the candidate models. The purpose is to expose a wrong intermediate contract—not to prefer an alphabetic path or assume every extra step is waste.
