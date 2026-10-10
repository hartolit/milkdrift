# 05 — Test UML/BPMN foundations against reviewed product behavior

**Owner:** standards/notation reviewer with runtime and human/agent authoring reviewers. **Output:** one proposed semantic/notation profile, validated examples and explicit implementation implications. **Prerequisites:** 01 evidence, 02 goals, 03 product meanings and initial 04 ownership/browser analysis. Iterate with 04/06 rather than treating their choices as permanently fixed. **Stop:** production design and evidence, not a diagramming prototype.

Read the current dossier/alternatives, federation decisions, [scenarios](scenario-corpus.md), [deliberation protocol](deliberation-protocol.md), and [implementation](../../practices/implementation.md)/[documentation](../../practices/documentation.md) practices.

## Read primary semantics, not only symbol catalogues

Use the official OMG [UML 2.5.1 specification](https://www.omg.org/spec/UML/2.5.1/) and [BPMN 2.0.2 specification](https://www.omg.org/spec/BPMN/2.0.2/). Their official landing pages supply normative documents and machine-readable artifacts. Record exact sections and versions actually consulted. Do not claim complete compliance, executable import/export or preserved semantics from familiar shapes.

UML Activity and BPMN principles are the user's requested foundation. Study their relevant control/data/action/call/event semantics seriously. Do not start a new proprietary shape language by default. Also do not import unrelated standard machinery merely because it exists.

Separate three decisions:

1. **Notation:** which visible convention communicates which behavior?
2. **Execution semantics:** what actually starts, waits, combines, changes or ends, under which authority and lifetime?
3. **Interchange:** what external representation can be imported/exported, with what supported subset and meaning preservation?

These are related but not equivalent. A diagram library is not an execution engine; an XML schema check is not a behavioral proof. A canonical internal representation can support multiple views without having multiple semantic owners. If a transform changes execution meaning, the shared daemon path must own and validate it rather than the browser.

## Compare credible foundations on the same work

Use the best alternatives from 03, grounded in 02, not three freshly invented unrelated products. Compare a selected BPMN semantic foundation, a selected UML Activity foundation, and retention/revision of the current core with explicit mappings. State exactly where each fits, where Milkdrift adds meaning, and what existing value changes.

Before claiming that a standard cannot express a Milkdrift behavior, identify the precise normative semantics, the exact missing behavior, and the reviewed need behind it. Trace the need to original grounds and the current 02/03 decision. Consider three explanations: a real needed extension; an obsolete or wrong product premise; or a mistaken reading of the standard. Do not protect every historical invention by declaring the product special.

A hybrid must have precise correspondences and one executable interpretation. Combining two separately attractive choices is a new proposal requiring full review; neither “hybrid” nor “BPMN-inspired” resolves incompatible activation, ownership or interruption semantics. If a BPMN-looking symbol means something else, either correct the behavior, choose a different clearly explained symbol or document a visible extension. Familiarity cannot be used to hide incompatible expectations.

Include these relationships in the mapping:

- action/task/capability invocation, typed input/output and side-effect contract;
- control flow, data/object flow and inter-owner messages;
- conditional choice versus merge; parallel activation, synchronization and result transformation;
- all/any/first-success/quorum decisions and what happens to unfinished work;
- timers, external signals, correlation, human decisions, event triggers and interruption;
- bounded repetition, nested calls, recurrence and genuinely continuous authorized work;
- reused definitions, collapsed regions, callable private services and accepted version identity;
- pools/lanes/partitions, execution placement, ownership and permissions without equating them;
- artifact/context provenance, branch-isolated data and resource lifetimes;
- definition versions versus runtime occurrences; live-edit proposals and unchanged historical facts;
- success/failure/cancellation/uncertainty and result acceptance as appropriate to the reviewed model.

Map all current major node kinds to retained, changed or replaced behavior. A unified human compositor can hide mechanical detail but cannot erase the difference between waiting for results and transforming them. A generic task need not be text-only. A method can be visually collapsed without becoming a new authority scope merely because of its shape.

## Establish meaning using concrete traces

For the hard scenarios affected by a candidate, state expected activation, data visibility, ownership, completion and interruption in a small transition table. Use S03/S06/S08/S09/S13/S16/S23/S26/S29 and add discriminating variants. Not every candidate needs a full essay for every case; each proposed semantic difference needs a case that tests it.

Include examples where a visually plausible connection is invalid, where two similar symbols differ materially, and where collapsing/expanding the diagram must not change behavior. Identify when a live edit should be refused versus when a desired valid edit exposes a core limitation. Give the same case to another reviewer to interpret without the author's explanation.

Compare the diagram, machine-authoring representation, daemon-validated definition and runtime trace. Declare the supported equivalence and exclusions. If a proposed round trip cannot preserve an unknown construct, plan explicit refusal or loss disclosure rather than silent stripping.

No production conversion code is authorized here. Existing parsers/validators, modeled traces and isolated normative examples can establish different kinds of evidence. Mark a proposed mapping unimplemented where it is not executable yet; schedule the missing proof before its implementation claim.

## Test the claimed benefit for agents, not their familiarity alone

Prepare a reproducible authoring/review exercise using several meaningful tasks and held-out variants. Include ordinary composition, an adaptive failure case and a cross-owner reusable operation. Fix the required behavior independently of the candidate notation.

Give each candidate comparable semantic documentation and access to the required operation schemas. Retain prompts, model/tool identifiers, generation settings, correction attempts, documents produced and validator/reviewer outcomes when an authorized model is available. Examine semantic errors, missing constraints, invented operations, repair effort and whether an independent reader predicts the intended behavior.

Do not hand the model a solution graph and call accurate copying evidence of easier authoring. Do not make one candidate win by receiving hidden extra context. Do not demand a statistically meaningful ranking from a few examples. Without model access, deliver the exercise and label the advantage as a hypothesis; it cannot become “LLMs are trained experts, therefore this is correct.”

## Specify the production visual grammar

Provide allowed shapes/ports/edges/labels, meanings, ambiguous or forbidden combinations, and their validation owners. Explain collapsed calls, explicit private internals, data versus control connections, remote ownership, pending changes and per-run overlays. Movement/layout is not a semantic revision.

Describe normal editing, selection, connection, inspection and repair interactions for supported constructs, including an accessible keyboard/non-graph equivalent. State what the user sees without internal inspection rights. Do not create a distinct glyph for every Rust type or rely on color alone.

Define an agent-readable grammar through the selected public authoring path. Required identity, operation and permission facts cannot be replaced by opaque free-form prose. The browser must not mint authoritative semantic digests or become the only compiler capable of expressing the diagram.

Interchange is included only where a concrete outcome justifies it and its mapping can be tested. Choosing a BPMN-informed language does not automatically authorize a full standards-import platform. Conversely, reject dismissing useful interoperability solely to preserve proprietary convenience.

## Output and gate

Write `working/notation-profile.md`: chosen foundation and rejected alternatives; exact normative sources; mapping and extensions; concrete semantics/round-trip cases; authoring exercise and observed or missing evidence; visual grammar; API/code/migration implications; loss-and-gain account and unresolved decisions.

Have a critic attempt to misread the most consequential diagrams and a runtime reviewer explain resulting execution. Resolve differences in the actual profile, not in a separate oral explanation. Reopen the relevant 02/03/04 decision where needed and recheck affected frontend/adoption work. A changed premise invalidates dependent approval until rechecked, even if the diagram still looks the same.

Commit checked planning increments. **Gate:** the standard-informed notation is truthful, implementable and preserves the chosen product abilities. Unsupported claims are explicit; no second workflow semantics has slipped into the UI. Familiarity is treated as a hypothesis to test, not authority.
