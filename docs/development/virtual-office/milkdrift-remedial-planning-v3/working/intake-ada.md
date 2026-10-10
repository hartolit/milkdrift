# Ada: independent first account of user intent

This is my interpretation as an independent intake reviewer. It is evidence for discussion,
not an authoritative replacement for the user's words or a vote for the present implementation.
I recorded it before reading the v3 context, synthesis, scenario corpus, proposals, or other
reviewers' accounts. The repository HEAD I inspected was
`908e7893f5dadb84d12712573c8daaa946829e39`; the initial worktree was clean.

## Sources and limits

My primary user-intent source is the selected verbatim [U01–U16 packet](../intent-source-excerpts.md).
It says that the assistant selected these excerpts and that this is not a complete conversation
export. Its context labels and compiler's notes are secondary interpretation. The packet gives
no original conversation identifier, transcript path, or date through which I could independently
recover the surrounding messages. I requested such a reference from the coordinator. At this
first-account checkpoint I have not received or read the original full conversation. Consequently,
absence from these excerpts is not evidence that the user never asked for something.

I have also read the repository entry instructions, the practice guide and documentation
practice, the workflow's assignment and verification rules, and relevant parts of the current
vision, architecture, status and roadmap. One combined document read was output-truncated;
I followed it with focused reads, rather than treating the unseen remainder as inspected.
These are current repository statements, not independent confirmation of user intent. I inspected
the blueprint node and graph definitions, the blueprint package explanation, structured-definition
type locations and kernel-test names. I have not executed those tests, traced the whole runtime,
or checked normative UML/BPMN sources. I have read no v2 synthesis, v3 outcome synthesis,
review-method prescription or other reviewer's position before writing this account.

The delegated assignment reports that the user authorized planning phases 00–08 and stopping
before implementation. That bounds my work; it does not add a product requirement to U01–U16.

## Outcomes the user asks for

The strongest desired outcome is a coherent, usable system for designing and performing evolving
work, with enough dependable structure to repeat useful methods. U15 describes adaptability and
strict output obligations together, and immediately questions whether two modes are needed. My
reading is that degrees of freedom and reliability need to coexist. Neither a global mode switch
nor identical outputs from nondeterministic work follows from the wording.

The user wants a real frontend designed deeply enough to expose conceptual defects, with Svelte
as the first framework (U01–U02). It must remain a client of the daemon's capabilities and
authority, usable independently and with more than one daemon (U02–U03). A useful interface must
represent important work such as method reuse and permissions rather than merely decorate the
subset easiest to draw. This does not demonstrate that any particular browser deployment,
authentication mechanism, screen list or backend endpoint is already approved or implemented.

The user wants the work to read as a detailed live flow diagram, including execution and progress,
and prefers established activity/process notation principles (U06, U08). The request is substantive:
the notation should help people and models express and understand workflows. It is not yet a
request for full UML or BPMN conformance, interchange, every standard construct, or a particular
canvas library. U08's predicted benefit for models is a hypothesis to evaluate, not measured success.

The planning process must first develop a coherent account of purpose, then use it to judge
dependent design, and be willing to change both old representations and new proposals (U04,
U09–U13). It must preserve valuable capability without treating either maximum simplicity or
maximum feature retention as the goal. The review itself should work as a representative Milkdrift
workload (U16), so preserving disagreement, evidence, revisions and bounded authority matters to
the proposed user journey as well as to the planning method.

## Tentative explanations and implementation assumptions

| User wording | My classification and consequence |
| --- | --- |
| A blueprint is effectively a generic node formed by its input (U05). | A challenge and suggested explanation. Investigate what restrictions the current model imposes and what would be lost if control semantics moved into arbitrary instructions. Do not convert the suggestion into an instruction to erase all node variants. |
| A method workflow is a reusable workflow collapsed into a node (U07). | A strong account of the desired reuse experience, immediately qualified by uncertainty about the implementation and whether the idea is right. Preserve the desired ability to reuse work while testing several representations. |
| The method might bridge to another daemon (U07 context). | An explicitly doubted implementation relationship. Reuse and placement may be independent. Neither mandatory publication for local reuse nor mandatory method wrappers for every remote operation is authorized by this excerpt. |
| Splitter and compositor may be needed (U06). | Familiar visual ideas being reconsidered. Their names and grouping are negotiable; concurrent work, waiting and combining results still need intelligible behavior if supported. |
| Use UML/BPMN principles because models know them (U08). | A clear standards preference paired with an unproven mechanism for better model performance. Standards semantics and actual authoring success require separate evidence. |
| Vision, architecture and implementation discussion folders (U11 context). | A tentative organization example. It does not authorize elevating any folder, contributor or review stage into unquestionable product truth. |
| The sprint could be extremely long (U10). | Permission to investigate to adequate depth, not a requirement for a large document count or unlimited work outside the assignment. |

U14 asks for coherent regular commits and scheduling the integrated system test at the end of
an assigned sprint. My reading retains focused checks at intermediate boundaries and does not
permit carrying a known defect forward just to satisfy a schedule. It also does not make a
planning artifact into executable evidence.

## Preferences that remain unresolved

The excerpts do not settle how much of the graph a novice should draw directly, how much an AI
should generate, or whether those are separate entry experiences. They do not settle whether a
collapsed reusable workflow is a view, a pinned call, a published service, a template copy, or
several clearly distinguished actions. The smallest satisfactory choice must be tested against
editing one caller, updating future reuse, preserving an already running call, and executing the
same work on another host.

The excerpts also leave the first deployment packaging, multi-daemon trust bootstrap, offline
editing expectations, and the desired degree of standards interchange unresolved. A standalone
frontend does not by itself answer any of these. Planning can recommend bounded defaults and
show consequences, but cannot cite them as user-established preferences.

There is unresolved tension between a small vocabulary and explicit distinctions needed for
durability, authority and concurrency. The relevant question is which distinctions users must
understand to predict outcomes, not how many internal Rust variants can be deleted. Conversely,
existing runtime distinctions do not prove that every distinction belongs in the user's main
canvas or needs a separate product noun.

## Strongest tensions with the present repository

1. The vision already calls a blueprint a reusable method and prescribes canvas, timeline and
   inspector as three main perspectives. U04 and U11 ask for the vision itself to be questioned;
   U01 asks the real frontend design to uncover flaws. Those current statements should be tested
   as proposals with a rationale, rather than used to make the review converge automatically.
   See [vision sections 2 and 24](../../../../product/vision.md#2-the-product-thesis).
2. U05's text-input/output explanation is narrower than the implementation I inspected.
   [NodeKind](../../../../../crates/blueprint/src/model/node.rs) includes capability tasks,
   typed branching, fork, join, reducer, repeat, waits, subworkflow and terminal semantics;
   [EdgeKind](../../../../../crates/blueprint/src/model/graph.rs) separates control and typed
   data. These distinctions can be justified by observable behavior, but their existence is
   not that justification. Replacing them with generic text instructions would require a
   credible owner for each deterministic responsibility; retaining them requires an
   intelligible authoring model rather than an enum-shaped interface.
3. The architecture distinguishes pinned subworkflows, governed agreements and published
   methods, while the user describes uncomplicated reuse and doubts its relationship to remote
   execution. These mechanisms may serve different needs, yet their current names can conceal
   those needs. [Architecture terminology](../../../../architecture.md#terminology) and the
   [blueprint package](../../../../../crates/blueprint/README.md) establish current concepts;
   they do not establish that a user must learn all of them to reuse a workflow.
4. The status says there is no GUI or browser qualification, and that the ordinary model editor
   refuses richer definitions. A client shell cannot produce a complete live diagram authoring
   experience just by exposing that constrained editor. The plan must identify where existing
   public construction/read operations suffice and where a backend gap remains; claiming that
   thin-client ownership already delivers the desired frontend would be circular. See
   [current limitations](../../../../product/status.md#limitations-now).
5. The roadmap requires preserving host, managed-resource, publication, evaluation and client
   behavior. That is a valuable constraint against destroying working capability, but U09
   preserves valuable ambition rather than every old representation. A proposed improvement
   may replace an old representation if it explains how the valuable behavior survives and
   what migration or explicit refusal is needed. Neither “already implemented” nor “simpler”
   settles the choice by itself.

My provisional test for coherence is whether an operator can explain what is being defined,
what actually happened, what may change next, which authority permits that change, and how to
reuse the work without accidentally changing an existing execution. This is my derived test,
informed by user outcomes and repository constraints. It is not a quotation or a decision that
all current mechanisms satisfy it.

## Handoff evidence

This account was authored independently before synthesis exposure. No production files, indexes,
commits, Cargo commands, paid model calls or live deployments were changed or executed. The
coordinator owns integrated documentation-contract verification. Later discussion may revise my
conclusions; it should preserve this first account so readers can distinguish an independent
observation from a conclusion reached after convergence.
