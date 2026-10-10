# Bram — independent intent intake

This is Bram's first account, written before reading the v3 synthesis, phase prompts,
context-and-preservation file, proposals, decisions or another reviewer's account. It is an
interpretation to challenge, not an adopted product decision.

## Provenance and exposure

- Intake source HEAD: `908e7893f5dadb84d12712573c8daaa946829e39`.
- The worktree was clean when first inspected. This document is the only file I authored during
  this intake. No production change, Cargo command, commit, model call or deployment was made.
- User evidence available to this intake is the selected U01–U16 quotations in
  [intent-source-excerpts](../intent-source-excerpts.md). The containing context labels and
  compiler's notes were visible and are secondary interpretation. The selection does not provide
  an original conversation export or precise history location. I requested any available original
  history from the coordinator; none had been supplied when this first account was written.
- Before this account I read `AGENTS.md`, the excerpt file, the development practice index,
  documentation practice and workflow. I inspected canonical vision, architecture, status and
  roadmap as current repository evidence. Some broad command output was truncated; I then read
  the relevant terminology, daemon/GUI sections and current limitations explicitly. I do not
  claim a complete audit of those documents.
- Source inspected: blueprint `NodeKind` and `TaskConfig`, the beginning of its structured
  configuration owner, blueprint README, and the control-client README. These establish a narrow
  implementation observation, not successful execution or full consumer coverage.
- Filename inventories exposed names of the v3 files and external prompt-review files, but I did
  not open their contents. No v2 synthesis, v3 outcome synthesis, review protocol, scenarios,
  worked traces, other intake or coordinator-preferred product model informed this account.

## Requested outcomes I can support from user statements

The user wants a coherent, usable Milkdrift product whose real frontend exposes conceptual
mistakes that another abstract backend discussion could miss. U01 rejects a prototype as the
target; it does not require that implementation begin before the product questions are resolved.
U02 specifies Svelte as the first GUI and says frontend responsibility is a shell around daemon
operations. U03 adds that this frontend stands apart from any single daemon and must account for
authorization and capabilities. A concrete design should therefore let a person understand
which daemon and authority govern each action, including when more than one daemon is connected.
The excerpts do not determine a browser/desktop packaging choice or a federation mechanism.

The desired working experience combines an understandable flow diagram with live execution and
progress (U06). The user prefers established UML activity/BPMN principles (U08), both for human
coherence and in the hope that agents can author workflows reliably. Familiar notation is a
stated design preference; better model performance is a hypothesis. Neither implies a request
for full standards compliance, interchangeable file formats or every standardized symbol.

Reuse should spare people from reconstructing a workflow they already have (U07). The quoted
collapsed-node description is a useful user explanation of the intended interaction, with an
explicit warning that both the implementation and the explanation could be wrong. I infer a
need to inspect and reuse work at more than one level of detail. I do not infer that every reusable
workflow must first become a published service, that reuse must cross a network, or that a new
object named Method is necessary.

The system should permit evolving work and dependable protocols for results where reliability
matters (U15). The user questions their own static/dynamic split, so the strongest requirement is
the coexistence of these outcomes. It is not a commitment to two engines, two global modes or
identical generated content. U16 makes the current iterative investigation a representative use:
an evolving plan, retained objections and evidence, independent contributions, revision of
conclusions, and eventual action should be expressible without endless manual prompt shuffling.
That representative workload should include an outcome that remains disputed or inconclusive.

Finally, the user asks for a serious reassessment, including the coherence of the vision and the
review process itself (U04, U11, U13). U09 protects valuable ambition and work, but expressly does
not make simplicity the measure of success. U10 removes an arbitrary investigation-size ceiling;
it does not turn any imagined feature into approved implementation. U12 asks authors to expose
their derivations rather than write authority into existence. U14 requests regular coherent
commits and a final full-system test within an implementation sprint, with useful focused checks
along the way.

## Tentative explanations that must stay tentative

My working explanation is that the user is experiencing a gap between meaningful activities and
the implementation vocabulary presented to them. A diagram that is visually understandable may
still force the author to choose among many concepts whose necessity has not been explained.
That could be a product-model defect, an interaction defect, a documentation defect, or some
combination. U05 is evidence of this confusion, not proof that typed nodes caused it.

The frontend may serve as a demanding consumer of the product model: a person trying to reuse a
workflow, inspect a running child, or choose a remote capability should encounter one coherent
explanation of the operation. A failure to draw that interaction truthfully may reveal a missing
public operation or an unnecessary distinction. Conversely, a difficult operation does not become
unnecessary merely because a compact diagram cannot explain it.

The user's reliability concern may concern a stable agreement about acceptable results while the
method of reaching them adapts. It may also mean a workflow whose steps stay predictable across
runs. The excerpts support both readings and do not choose their exact balance. A design should
demonstrate where the user sets that balance before adopting a single interpretation as intent.

## Implementation assumptions to test rather than inherit

1. A frontend can remain a thin semantic client while providing substantial authoring assistance,
   navigation, local drafts and presentation state. “Shell” should not automatically be translated
   into an interface that exposes every storage or runtime type verbatim.
2. Structured concurrency and typed control can remain runtime concepts while ordinary work is
   authored through a smaller set of understandable interactions. This is a hypothesis, not a
   decision to hide advanced operations or delete typed nodes.
3. Reusing a local workflow and invoking a published method probably share some authoring
   concepts but differ in what authority and durable obligations must be established. The exact
   common representation needs evidence; it cannot be deduced from the word “method.”
4. Connecting one frontend to several daemon authorities need not mean merging their workflow
   ownership. That architecture could satisfy U03, but the user has not explicitly approved its
   details in these excerpts.
5. Immutable accepted history can coexist with continuous improvement if changes apply to future
   work. This is a strong current repository design with useful consequences, not a quotation
   establishing that the user has already settled every constraint on adaptation.

## Unresolved preferences and consequences

| Open preference | Why it changes the design |
| --- | --- |
| How much familiar notation is desired: visual vocabulary, executable subset, or interchange? | A standards-inspired UI and an interoperable model impose very different semantic obligations. |
| What should a collapsed reusable workflow expose by default? | A local reference, copied editable content, and a versioned callable contract imply different update and inspection behavior. |
| Which changes may a workflow perform without a new human decision? | Required evidence, effect authority, internal planning freedom and changing the reusable method cannot be conflated. |
| Does “standalone” require a browser-only deployment, a desktop application, or both? | Credential handling, reachability, installation and cross-origin behavior depend on this answer. |
| Should a multi-daemon interface allow one authoring action spanning domains? | A shared view is much weaker than a shared transaction or execution owner. |
| Which concrete reliability outcomes should be promised? | Stable output shape, retained evidence, reproducible steps and successful external effects are different claims. |

These are open choices, not reasons to suspend all planning. Alternatives can be carried through
concrete scenarios with their user consequences visible. Where no option dominates, the eventual
implementation program should retain a decision gate with bounded conditional work.

## Strongest tensions with the current repository

The strongest tension is procedural. The current vision says the first UI should be built only
after read models and commands are stable headlessly, while U01/U04/U11 ask this review to use a
real frontend design to discover conceptual defects. Treating current backend stability as proof
that the product model is settled would prevent the requested reassessment. Planning can resolve
this tension by designing real user operations now and using each operation to test backend fit,
without implementing a GUI under this assignment.

The blueprint implementation does not support the claim that every node is merely text input and
output. `NodeKind` includes tasks, branches, forks, joins, reducers, repeats, waits, signal waits,
subworkflows and terminals. `TaskConfig` selects a capability and context policy; structured
configurations own control and isolation rules. These are consequential distinctions. The review
should examine their authoring cost and alternatives, not use U05 either to erase them or to
silence the user's question because they already exist.

Current vocabulary identifies a blueprint as a reusable method, a pinned subworkflow as an exact
revision/interface call, and a published method as a callable version with an agreement and
constrained service identity. That may be justified implementation, but it is a substantial
distance from “reuse this workflow as a collapsed node.” A person should not need to learn
publication machinery to perform an ordinary reusable call if no publication boundary is crossed.
Whether current consumers actually impose that burden remains untested by this intake.

The daemon architecture assigns one workflow domain an authoritative owner. This need not conflict
with a standalone client connected to many daemons, yet the current GUI section mostly describes
one daemon and fixes three perspectives. Those perspectives are a current design proposal; their
presence in vision does not establish user approval or prove they cover cross-daemon navigation,
credentials, stale state or restricted inspection.

The current protected agreement model allows bounded task edits while preserving enclosing
structure and effect prerequisites. It can plausibly support dependable adaptation. It could also
constrain this review workload too narrowly if changing the method of deliberation requires
changing protected structure. U16 warrants a worked case in which reviewers discover that their
original process is poor. The design must say whether this revises the current run, creates a
new agreement, changes a future reusable workflow, or requires escalation. This is a tension to
exercise, not evidence that the existing mechanism is defective.

My initial position is to preserve the distinction between work definition, observed execution
and authority as a useful candidate foundation, while treating the present concept names,
publication path, frontend perspectives and adaptation boundary as challengeable. Neither
“generic nodes solve it” nor “the current invariants already settle it” follows from these sources.

## Intake verification and limits

I reread this account against U01–U16, checked its local source references and inspected Markdown
structure. The coordinator owns the integrated documentation contract run; no Cargo checks were
run by this worker as instructed. Source inspection establishes only the stated type and document
observations. It does not establish backend completeness, standards compliance, frontend usability,
physical multi-host behavior or model performance.
