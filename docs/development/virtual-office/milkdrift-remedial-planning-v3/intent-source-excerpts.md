# User-source excerpts for independent intake

These are selected verbatim excerpts from the user messages supplied with this planning request. The labels are local source references, not requirement IDs, dates or evidence of implementation. The selection was compiled by the assistant; it is not a complete conversation export. Keep the original speaker and surrounding uncertainty when using an excerpt. Obtain additional original context where accessible; otherwise record the limitation rather than invent it.

The **notes** at the end are the compiler's interpretation, not user quotations. An intake reviewer should write an initial account from the excerpts and available original sources before consulting this package's outcome synthesis or the coordinator's preferred synthesis.

## U01 — Real frontend, not a prototype

> I also want to clarify that we should not try and build a prototype frontend, as this will make it less likely for us to catch real conceptual bugs. We have to design a real frontend.

Context: the user rejected the prior assistant's proposed mock/prototype frontend and requested a planning sprint that derives a real implementation sprint.

## U02 — Svelte and frontend responsibility

> Also our first target GUI framework is Svelte. There's been a few misunderstandings in the past of agents thinking our GUI was to be created with ICED, which isn't the case yet. Maybe in the future after we've established a working product. The frontend is just a shell or "wrapper" of our daemon afterall. So it's important it stays this way to avoid conflicts across future GUI's.

## U03 — Standalone, multiple daemons and permissions

> it seemed rather hopeless. It could establish a few key parts, but then it would just end up failing and get confused by some of our features like method workflows and omit important parts like that the frontend is a standalone and not attached to only one daemon or the authorization, capabilities ect.

Context: this is criticism of another design attempt, not evidence that the backend already supports every desired frontend action.

## U04 — Why reassessment is needed

> The entire codebase consists of me splatting out different and overlapping ideas for 2 months, while the entire vision of milkdrift and its purpose also has shifted dramatically. So it's not unreasonable to find non-sensical implementations or visions that doesn't exactly fit together with the current state of vision. And even then I'd say the current state is also rather poorly defined.

## U05 — Generic nodes are a question, not an established redesign

> Just take our blueprints as an example. What exactly are we defining? In reality it's just a node with instructions (input/output of text), but yet we've typed hardcoded functionality rather than letting it be what it is. Generic node, but formed by its input... By defining it, we're perhaps limiting the expression of a workflow and making it more complicated to define.

Context: the user explicitly questioned the implementation rather than reporting verified source behavior. The source reconstruction must examine the actual types and consequences separately.

## U06 — Live diagrams and questioned extra concepts

> Also the way I'm thinking of milkdrift is closer to UML. It's like a detailed dynamic flowdiagram, combined with a realtime workflow execution and progress... We should probably take use of the chart symbols and meanings to define the visuals, but then also include our own, like a splitter and compositor (if this even matters or is just yet another complexity that could've been simplified or done differently and better).

## U07 — Reuse and remote execution: preserve the surrounding doubt

> I also see a method workflow as just being a reusable workflow that's collapsed into a node. So that a user doesn't have to build this entire workflow up again.

> Though this is most likely not what's been implemented + the vision itself might also be faulty and too complex. Like should this really be the bridge for executing on an external machine?

> The entire scenario could be made up, as I've offloaded all the creations to AI. I do not know, the actual implementation.

Context: the user explored whether a method might act as a gateway into another daemon, then challenged both the premise and its purpose. Do not quote only the opening sentence to make mandatory publication, a new object type, or a network restriction appear user-approved.

## U08 — Standards foundation and a proposed benefit

> UML activity diagrams and BPMN sounds useful. I think we should base our design on these principles rather than build something from scratch. Especially since it's battle tested methodologies and has existed for so long that an AI is a trained expert in these domains, making designing of workflows more structured and easy to understand for an LLM.

Context: the preference for using established principles and the prediction about model performance are separate claims. The latter is not measured evidence.

## U09 — Preserve valuable ambition, not every old representation

> That's not to mean that every feature and implementation must be stripped away to it's core. We're not doing all of this just to start back at square one.

> We have built valuable work and valuable thoughts around what we should be, even if scattered, incoherent, bloated and conflicting.

> What I'm saying is that, defining coherency across this spectrum doesn't inherently mean simplifying it.

> We have to deal with what we've got and how we can change it for the better.

## U10 — Investigation depth is not a fixed prompt budget

> Our sprint creation shouldn't set any limits for what can and can't be done. The sprint could be 128 prompts reaching over 500k words/lines of instructions and semantic "keying." As long as we create coherent structures and discusses our work/issues to establish project coherency, then the prompts are worth executing.

Context: this requests sufficient depth and permits extensive planning. It does not supply credentials, consent to destructive operations, or approval to execute an unreviewed implementation program.

## U11 — Vision before dependent architecture, but folders are tentative

> This example made me think that we'd most likely want to review and discuss our vision's coherency first, rather than the actual implementations or project architecture. That way we'll be able to derive a backplate that we can compare and derive solutions from, rather than creating scrambled implementation ideas and discussions, that's based on incoherent visions.

> Though that's not to say this is the "right" structure. It's just a quick example I made without much thought about the definitions and structure of the sprint or how a problem may be discussed and solved.

Context: the second excerpt qualifies proposed vision/architecture/implementation discussion folders. V3's additional product-model layer is a review-process choice from the subsequent discussion, not a newly discovered user product feature.

## U12 — Contributors must not sound like unquestionable authority

> we could include safeguards by making the agent say that it's comments is it's own deriviation of the matter rather than implying that this is the only true solution.

> AI agents have a tendency to read text very literally and instantly read it as the source of truth.

## U13 — Review the process itself

> How does our workflow look like? How do we go from review, discussion, solutions to planning our execution sprint? Are we actually structured the way that promotes the best possible outcome? Or are we just hallucinating a structured approach?

## U14 — Commit and verification practice

> Also schedule in regular commits instead of having the agent do one big commit at each prompt. That way we can better backtrack our work and avoid a full re-implementation should things go south.

> Also push the full system test to the end of the sprint, so that we don't waste tokens doing a full test for each phase/prompt

Context: focused checks still accompany changes. Defects are not knowingly handed to the final tester, and configured CI is not disabled.

## U15 — Adaptability and dependable reuse

> I want milkdrift to be dynamic and continously evolving itself.

> I want to build reproducible workflows to ensure a dynamic, but strict output "protocol" used in areas where reliability matters.

> Though in reality the workflows are probably more or less the same. I just divided it into 2 modes to make sense of its distinction.

Context: these excerpts come from the static/dynamic discussion. The user later questioned having two modes; neither a global mode split nor identical generated outputs should be inferred as a fixed requirement.

## U16 — This review is itself a representative workload

> This iterative process is exactly what milkdrift wants to be used for.

## Compiler's notes — not additional primary evidence

The conversation contains many assistant prescriptions: first-class method objects, particular node cores, fixed screen perspectives, prototype recommendations and safeguards. They are candidate interpretations, even when later prose sounds authoritative. Cite them as assistant proposals rather than user requirements.

The supplied v2 package is a useful secondary synthesis and instruction baseline. The supplied repository is primary evidence of what its source implements at the archived commit, not of what the user still wants. Current normative standards are evidence of their defined semantics, not of Milkdrift's compliance or model proficiency.

Use these distinctions in the intent register. Do not conclude that the excerpts exhaust the user's goals, that repetition creates approval, or that the newest tentative idea automatically supersedes an earlier explicit instruction. When missing context affects a consequential choice, preserve both plausible interpretations and the conditional work rather than making up a preference.
# Additional product directions supplied during reopened investigation

The following two directions were supplied directly by the user on 2026-10-10 in response to
specific product-preference questions. They supersede narrower investigator interpretations.
They approve product behavior, not production work or a particular architecture. The wording below
is preserved as supplied, rather than treating an investigator's summary as primary evidence.

## U17 — human-reviewed knowledge and precise evidence

> I approve allowing bounded, human-reviewed comparisons to count as evaluated evidence, including for reusable lessons derived from research, planning, architectural decisions, and other knowledge work.
>
> However, this approval concerns the **meaning and scope of evidence**, not automatic promotion or equivalence with automated verification.
>
> The fundamental principle should be that **evaluation is not limited to mechanically verifiable outcomes**. A human can meaningfully assess whether a research method produced a better explanation, whether an architectural proposal addresses more constraints, or whether a revised planning approach improves a particular result.
>
> Establish the following distinctions:
>
> 1. **Evidence must retain its basis.** Record exact inputs, compared versions and outputs, evaluation criteria, reviewer identity and authority, judgment, reasoning, limitations, and relevant counterevidence. Preserve negative and inconclusive outcomes.
>
> 2. **Claims must remain bounded.** A favorable human review establishes that an authorized reviewer preferred or accepted a specific result under stated criteria. It does not prove universal improvement, statistical superiority, or correctness outside the evaluated scope.
>
> 3. **Evaluation criteria should be established before comparison when claiming a controlled evaluation.** Retrospective assessments are still useful evidence, but must be identified as such rather than presented as predeclared experiments.
>
> 4. **Human judgment and automated verification are different evidence types.** Neither should silently impersonate the other. Human review does not replace mandatory technical verification, protected-effect requirements, or security checks.
>
> 5. **Evaluation and adoption remain separate decisions.** A reviewed lesson may become eligible for reuse under its declared applicability, but it must not automatically replace an existing method, change future execution, or gain publication authority.
>
> 6. **Reviewer independence must be represented honestly.** A reviewer may also be the original author, but that relationship should be visible. Multiple agreeing reviewers are not automatically independent corroboration.
>
> Most importantly, distinguish **evaluating knowledge** from **qualifying an executable method**. A useful lesson about architectural design need not be an executable workflow or a protected managed-method repair.
>
> Investigate how this fits the existing knowledge selection, learning declarations, comparisons, approval receipts, and publication mechanisms. Prefer extending their shared evidence and authority principles rather than creating a parallel learning engine.
>
> You have discretion to propose a better representation if these distinctions expose weaknesses in the current design.
>
> The intended outcome is to allow Milkdrift to learn from meaningful human-reviewed experience while preserving precise evidence, honest uncertainty, and explicit authority.
>
> Treat this as an approved product direction. Continue investigating the actual architectural implications, challenge unnecessary restrictions, and document your recommendations through the whiteboard.

## U18 — bounded automatic reconsideration within ongoing work

> I approve **bounded automatic reconsideration within explicitly authorized ongoing work** as an intended Milkdrift capability.
>
> The finished product should not require a human or an external orchestration agent to manually initiate every new planning round. That would preserve much of the manual coordination Milkdrift is supposed to eliminate.
>
> However, this is approval of the **product behavior**, not automatic approval of a new session owner, scheduler, state database, or architecture.
>
> The intended process is:
>
> **Goal → Plan → Execute → Evaluate/Review → Reconsider → Revise → Continue or Stop**
>
> A review might identify a failed implementation, a flawed assumption, a better approach, or insufficient evidence. The system should be able to determine the permitted next action within the originally authorized scope.
>
> Important requirements:
>
> 1. **Automatic does not mean unconditional.** Criticism should trigger an assessment, not necessarily another execution. The correct outcome may be revision, further investigation, accepting the original result, escalation, or stopping.
>
> 2. **Work must retain its continuity.** Planning rounds should belong to one identifiable, durable investigation or work commitment. Earlier assumptions, revisions, attempts, failures, dissent, and accepted evidence remain traceable across reconsideration and restart.
>
> 3. **Authority must remain bounded.** A controller may reconsider and perform work that was preauthorized, but cannot enlarge its own permissions, weaken protected obligations, redefine success, or authorize consequential effects outside its agreement.
>
> 4. **Budgets must be cumulative.** Replanning, creating child runs, retries, remote delegation, or restarting must not reset the parent commitment's allowance. Preserve the current distinction between reservations, measured consumption, and uncertain work. Include finite iteration, time, cost, and other applicable limits.
>
> 5. **Reconsideration must respond to actual evidence.** An agent's criticism is not automatically a fact or an instruction to modify the workflow. Conflicting reviews and unverified claims should remain distinguishable. Repeatedly producing equivalent plans without meaningful progress should lead to an explicit stop or escalation condition.
>
> 6. **Human intervention remains available.** Users should be able to inspect the current reasoning and evidence, pause work, change future requirements through authorized operations, approve consequential changes, or stop the investigation. Manual operation remains supported.
>
> 7. **The same behavior must apply coherently across research, planning, implementation, and evaluation.** Do not create a special autonomous coding loop disconnected from ordinary Milkdrift workflow semantics.
>
> **Architectural investigation required:**
>
> Determine whether the existing workflow runtime, bounded repeat, controller lifecycle, proposals, reconciliation, and cumulative accounts can collectively provide this behavior.
>
> If they can, identify the missing orchestration and consumer responsibilities and complete those boundaries.
>
> If they cannot, establish precisely which semantic ownership is missing and why a new durable concept is justified. Compare alternatives rather than assuming that a new "work session" entity or second scheduler is necessary.
>
> In particular, distinguish a continuous work commitment from individual workflow revisions, runs, attempts, and published methods. Determine whether these are different entities or different representations of existing ownership.
>
> Use our S28 iterative-development scenario as a demanding example. An independent reviewer discovers a flawed premise; Milkdrift must preserve earlier evidence, reconsider the plan, identify affected future work, and continue where authorized without requiring an external actor to manually reconstruct the process.
>
> **The smaller manual-reinitiation model remains a valid operating mode, but it is not the desired ceiling of Milkdrift's capabilities.**
>
> Please continue the whiteboard investigation, compare concrete alternatives, and propose the cleanest coherent design. Do not assume the existing architecture or your proposed session model is necessarily correct.
>
> This is an important product commitment. Its exact implementation must still earn its place through the broader remedial architecture review.
