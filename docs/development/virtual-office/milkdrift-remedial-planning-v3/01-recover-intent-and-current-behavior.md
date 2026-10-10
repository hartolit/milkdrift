# 01 — Recover original intentions and reconstruct actual behavior

**Owner:** primary-intent readers and source/consumer investigators; coordinator integrates after their initial accounts. **Output:** an evidence-backed intent register, system dossier and recorded operator/browser observations. **Prerequisite:** 00. **Stop:** reconstruction, gaps and testable questions; disputed goals are reconciled in 02, not silently settled here.

Read the current context packet, [scenarios](scenario-corpus.md), [worked traces](worked-coherence-traces.md), [implementation practice](../../practices/implementation.md), [documentation practice](../../practices/documentation.md), [workflow](../../workflow.md) and relevant source owners. Respect the [README's experiment scope](README.md).

## Recover the grounds before comparing the maps

Read [user-source excerpts](intent-source-excerpts.md) and [claim/decision rules](whiteboard-and-decisions.md). Expand the intent register from actually accessible original sources and relevant project history. Keep quoted statements, their surrounding qualifications, and your interpretations separate. Preserve uncertain or contradictory language rather than sanitizing it into a confident requirement.

Some primary-source readers should inspect the original material before reading the coordinator's preferred product explanation. Record what each reviewer has seen. When several documents cite the same earlier inference, identify the common root instead of reporting several corroborating sources. A later tentative suggestion does not automatically supersede an older explicit instruction.

For high-impact ideas, identify the original problem, proposed mechanism, later scope changes and what is still unknown. Missing original chat or inaccessible evidence remains a gap; do not reconstruct exact user approval from an ADR written by an agent. This material informs the vision reconciliation in 02.

## Two maps must meet

Build the system map from both directions:

**From intent:** what should a person or agent accomplish, what choices/evidence are needed, and what would success or a legitimate failure look like? Use current goals and a small relevant history of changed intent. Do not start by naming screens after crates.

**From implementation:** trace actual public commands/reads, owned state, transitions, external calls, output and recovery. Source references include exact types/functions and their consumers/tests, not just ADR headings.

Compare the maps and expose their disagreements for 02. Mark an absent public route, contradictory constraint or unclear purpose; do not silently correct the description to match whichever side is convenient or choose the desired product solely from current source. A working CLI command is useful evidence but does not establish browser reachability or a usable interaction.

## Complete the significant concept and outcome inventory

Cover all meaningful families, including newly discovered ones: definition/revision/run/occurrence/attempt; generic tasks and capability requirements; control/data flow; decision/fork/join/reducer/repeat/wait; subworkflows, reusable copies and callable publications; direct/remote work; actors/grants/service authority; approval, controller/account limits and prospective adaptation; artifacts/context/knowledge; installation/working-area ownership; provider/adapter behavior; client connections, commands, pages, streams and layouts; learning/evaluation/promotion; history/retention/recovery.

For each family, identify the outcome, actual representation and owner, public path and rights, dependencies, lifetimes, relevant evidence and unresolved questions. Record current naming as provisional. Do not produce an essay for every struct; deepen an area when its relationship or proposed change has material consequences. A lightly reviewed family needs evidence and a trigger for revisiting it.

For major actions trace request → authorization → definition/input validation → durable acceptance → scheduling/effect → output/uncertainty → inspection/recovery → user decision. Include branches and callbacks. Use the deliberation protocol's backward, neighbor and composed-lifetime checks. Establish what the downstream consumer assumes **before** accepting the producer's explanation.

## Execute controlled existing journeys where available

Do more than read documentation. Reuse maintained examples/test fixtures and exact product binaries, in isolated installations. Select discriminating journeys that collectively cover:

- create/open, edit, input, save/run, inspect and reconnect/reuse through ordinary operations;
- direct execution and a workflow-origin operation, identifying their different record ownership;
- pinned reuse versus published service invocation, including invoke-only disclosure;
- two workflow owners and an execution-only host, or source-backed limitations when a route is not supported;
- future repair/adaptation, required acceptance, resource use or learning where these relationships are critical to a proposed redesign.

Use controlled provider responses to isolate deterministic product behavior; do not replace core decisions with mocks. Do not require a live model to invent correct application code to prove a permission/recovery path. Conversely, do not label a controlled response as autonomous model success.

Choose at least one interruption/denial and one legitimate progress case for each high-impact boundary under redesign. Record exact commands, source/build, declared fixtures, actual responses and final state. Missing rights, dependency/tool availability and failed observations remain visible. Tests may establish contract behavior; the dossier still must explain whether it serves the user outcome.

A reader should be able to reproduce a critical observation from the report, not depend on a missing `Downloads` archive or the author's memory. Sanitize evidence and retain private raw data under `target/remedial-planning-v3/`.

## Browser feasibility comes before final screen design

Trace current authentication, protocol negotiation, identity, CORS/origin handling, TLS expectations, SSE framing and request recovery. Inspect both HTTP handlers and actual client behavior. Distinguish what a Rust CLI can do from what a browser permits.

Where available, run bounded browser-origin probes against isolated real daemons: authorized JSON request, preflight/cross-origin behavior, streaming, interruption and connection to another independent origin. Keep browser test code as a temporary diagnostic; it must not become a mock frontend or bypass permission checks. Use public browser documentation and record versions/environment.

Classify findings as existing supported behavior, missing daemon/deployment functionality, client implementation need, operator provisioning requirement or untested platform assumption. A central cloud service or privileged local daemon is not an automatic remedy. Carry viable alternatives into 04, with the user goals they would serve; do not wait until the GUI is designed to discover a transport assumption is impossible. Early feasibility findings inform 02 but do not by themselves decide which valuable outcomes to remove.

## Investigate the cause, not just the current symptom

For high-impact contradictions, inspect the relevant decisions/commits: what was the original need, which surrounding assumptions changed, which extra mechanism was added, and why current tests do not decide the issue? Limit history research to the question; no full chronological rewrite.

Check tests for expectation independence: does the expected result express a need or merely reconstruct the same code? Do fixtures only cover one name, one actor, one daemon, one model, one completion order? Suggest a valid variation that would expose a hidden assumption. Do not assert that untested means broken.

## Feed the early reconciliation without waiting for every detail

Pass sufficiently grounded intent and significant source paths into 02 as they become available. Continue factual work where needed; mark its dependent goal/design claims conditional. Do not hold vision reconciliation hostage to a full source audit, and do not claim a current public behavior from documentation alone. Identify a real consequential question suitable for the [review-method trial](review-method-trial.md).

## Dossier and handoff

Maintain `working/system-dossier.md` with outcome/concept coverage, relationships, representative full traces, a current-support matrix for S01–S30, recorded evidence and explicit gaps. Link large inventories/logs instead of copying them. Current facts, design hypotheses and proposed changes remain distinct.

Bring consequential disputes to the existing whiteboard with evidence and exact uncertainty. Update `working/context.md` to include newly established dependencies without importing a preferred redesign into the facts. Commit coherent source-map and observation updates separately as they settle.

**Gate:** another session can follow ordinary work and the difficult reuse/distribution/adaptation/resource paths without the author filling in hidden steps. Coverage gaps and unsupported scenarios are explicit; browser assumptions are exposed early. No production source is changed and no unperformed experiment is counted as success.
