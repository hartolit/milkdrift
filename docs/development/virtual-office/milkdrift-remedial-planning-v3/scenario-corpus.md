# Shared scenarios — preserve ordinary use and the difficult combinations

These are proposed test situations derived from the user's intent and the earlier planning package, **not claims that the code supports them all**. Keep S01–S20 identities from the first package so completed research remains reusable.

For every case, record current support and intended behavior separately. State owner(s), permissions, input/output meaning, accepted state, interruption outcome, visible user action, and relevant evidence or exact API gap. Link the outcomes in [context and preservation](context-and-preservation.md).

Coverage must include each major outcome and each high-risk relationship. Apply the same relevant cases to competing models. Detailed traces are required where they can distinguish the alternatives; a source-backed short disposition suffices elsewhere. Do not make every phase rewrite the corpus or pretend a review of twenty cases exhausts the system.

## Ordinary use, workflow construction and inspection

**S01 — First interaction:** A new user opens the standalone application with **zero connections**; adds a workflow-enabled daemon with explicit address and credentials; verifies the server's identity and rights; discovers capabilities; creates a one-step workflow and executes it on permitted inputs. A broken, inaccessible or auth-denied daemon must be explainable and reversible without restarting the UI.

**S02 — Different briefs, same saved workflow:** Build a model step followed by review; connect supplied input/previous output; save immutable revision; run twice with different input files; inspect actual prompts, outcomes and artifact provenance; retry/recover without duplicating provider effects.

**S03 — Change while running:** An agent proposes a future-stage edit after a failed review; distinguish definition revision, accepted current run, completed steps, pending steps, agreement and approval. No retroactive rewrite. What does the user see before/after decision?

**S04 — Model selection changes:** A saved workflow's model capability has vanished, changed generation, or become unauthorized. Explain what remains saved, which edits are permitted, and why a run can no longer proceed. Do not invent fallback provider selection.

**S05 — Agent-authored work:** Given a goal instead of manually placed nodes, a permitted agent proposes a valid workflow, including branching, failure handling and verification. How can a human understand, edit, approve or reject it? Distinguish a stated desired UX from current public authoring limits.

**S06 — Advanced graph:** Parallel approaches with isolated mutable workspaces, join policy, reducer, retry/repeat bound, wait/signal and terminal outcomes. Evaluate whether a user needs visible Fork/Join/Reducer boxes or whether notation can compose them without changing semantics. Include one branch failure and one cancelled branch.

## Hosts, capabilities, methods, authority

**S07 — Three daemon identities:** Connect to workflow owners A and B and execution-only host C. Each has different grants and capabilities. Create/open a workflow owned by B, view a run owned by A, and use C for a permitted direct invocation. A connection must not falsely appear to own another's workflows or rights. Disconnected A must not block B and C.

**S08 — Remote execution without methods:** A workflow on A invokes a granted operation served by peer B. B becomes unreachable before entry, after durable acceptance, and after execution with lost reply. Establish exact owner of work/history/uncertainty/account, and user-visible options. Do not automatically create a published method as a transport bridge.

**S09 — Reuse vs service publication:** A workflow is reused as a pinned subworkflow; the same or another workflow is published by B as a callable method with public inputs/outputs and service authorization. A third caller invokes only the method. Contrast reusing an internal workflow, granting external service access, and delegating operations. Define collapsed visual representations without erasing those distinctions.

**S10 — Service lifecycle:** A published method changes generation, is retired, or its service grant is revoked while an accepted call runs. Another client has only invoke/read-result permission, not inspection of internals. Define visible result, authorization boundaries and recovery; no phantom new revision.

**S11 — Authority transitions:** Two users see different portions of the same capability catalogue. A grant is narrowed, rotated or revoked during browsing/streaming. The UI must never reveal hidden rows, imply edit permissions based on old state, or leak another daemon's token.

**S12 — Capability locality and installation:** Compare directly invoking a preinstalled tool, invoking an attached external model, installing a managed service/resource that survives calls, and using a published workflow. Decide whether they deserve shared visual vocabulary and which states are genuinely distinct.

## Evidence, safety, learning and unexpected paths

**S13 — Uncertain consequential action:** A deployment/update may have executed, but the response was lost. UI offers read/inspect/recover or explicit authorized remedy; never equates disconnect, cancel acknowledgment or timeout with safe retry. Contrast verification refusal with uncertain side effect.

**S14 — Protected operation:** An exploratory agent tries to weaken a required verifier, bypass a protected effect through a direct operation, or promote a previously failed candidate. Show current control owner and precise refusal without exposing secrets. Evaluate whether “adaptive” needs its own node type.

**S15 — Evaluation and promotion:** A proposed better method is compared on held-out inputs with some negative/inconclusive outcomes, then deliberately promoted or rejected. Reusing, evaluating, and publishing must not be confused. Which UI concepts are actually necessary?

**S16 — Cancellation, approvals and deadlines:** Human approval wait, an autonomous controller request, revoked approval, connection loss and canceled task occur during a live run. Who can approve, cancel, modify future work, or observe the history? What is pending vs completed?

**S17 — Artifact and history visibility:** A viewer can see that an attempt exists but cannot inspect private context or output bytes. Paginated timeline and per-attempt evidence must distinguish denied, missing, truncated, unknown and zero rather than guessing.

**S18 — Mixed network environments:** A browser and later a Tauri wrapper connect across LAN, HTTPS proxy and overlay network with different trust and CORS/TLS realities. Define actual reachable/authenticated transports, SSE/event resumption, credential storage, origin policy, reconnect and app-start behavior. No `localhost` assumption or token in URL.

**S19 — High-impact change migration:** A core node/blueprint/method concept is merged or replaced. What happens to existing supported definitions, frozen exact run records, saved commands, current active runs, versions, user-authored layouts, public protocol consumers, CLI and external tools? No aspirational “just migrate” claim.

**S20 — Editing vs executing:** An operator opens a workflow without permission to change it; another agent can propose but not approve a revision; a third party can execute only a published method. See the difference between visible actions, possible actions, actual authority and authority recheck when submitting.


## Compound cases — make good features collide

**S21 — Nested work, resource maintenance and revoked authority:** A owns a workflow, B exposes a callable service, and its child needs an already protected working area. While the parent waits, another client requests maintenance and the caller's grant changes. Account for worker capacity, editing access, lifetime holds, continuation rights and visible progress together. Which decisions can occur, which must refuse or wait, and how is uncertainty resolved without “busy forever”?

**S22 — Same names, different owners, concurrent edits:** Two connected workflow authorities both contain `release`, and two actors have different rights. A user edits one revision while another author updates it. The UI reconnects to a replacement endpoint with a different host identity. No draft, grant, artifact, command or cursor may silently move to the other owner. Preserve recoverability without discarding legitimate unfinished edits.

**S23 — Runtime change meets notation:** One branch completed under the old revision, another is unstarted, a third is waiting on a signal. The new plan changes the join policy or removes future work. Explain the actual candidate validation, accepted old evidence, signal scope and diagram overlay. A visually valid graph is not necessarily a permissible live revision.

**S24 — A local lesson becomes an external service input:** Selected evidence contains private project data. A method-improvement agent may read it locally, but the proposed service would run under another authority. Distinguish permission to inspect, use as context, transfer, publish and expose a public result. A sensitivity label alone is not proof of network confinement; an authorized artifact reference is not universal access.

**S25 — Operator independence and resource drift:** The operator changes an owned container/service through ordinary external tools, then wants to release it from Milkdrift management while preserving output. Determine whether those operations exist. If proposed, define reconciliation, active-use obligations, service configuration and credential lifetime; stopping Milkdrift must not be confused with making a service independent of it. Do not silently replace the current lifecycle with a generic container dashboard.

**S26 — Continuous work under changing conditions:** A long-running authorized agent workflow encounters failed verification, develops a tool, adds future investigation, calls a reusable method, waits for human input, and resumes after a session/daemon interruption. Budgets, evidence and constraints remain meaningful across revisions. It must be possible to make legitimate progress without manually reconstructing every stage or obtaining an unrelated global privilege.

**S27 — Client lifetime versus work lifetime:** Close a tab, abort a fetch, log out, remove a saved connection, revoke a grant and request cancellation. These are different actions. Show how each affects observed data, cached sensitive content, continuing work and later recovery. A UI command receipt must not become an alternative execution ledger.

**S28 — Use Milkdrift for this very kind of iterative project:** A human supplies a fragmented brief and current repository. Analysts inspect separate areas with shared context; independent critics challenge relationships; the coordinator revises a decision; an implementer completes a change; verification finds a new counterexample; another session resumes; an evaluated lesson becomes available for later projects. Identify what current commands support, what requires a proposed change, and what must remain a human decision. Neither a fabricated “research node” nor an external agent silently doing all orchestration proves Milkdrift supports this process.

**S29 — Representational round trip:** Express the same reviewed behavior in the proposed diagram, accessible non-graph controls, machine-authoring input and daemon definition. Make a permitted edit and save/reopen it. Check semantic equivalence where supported, explicit loss/refusal elsewhere, identity-preserving layout changes, private method internals and truthful execution overlays. Run-state annotations must not become new instructions.

**S30 — Upgrade without restarting the product:** A chosen redesign changes a definition/control/service representation. Reconcile supported saved definitions, active and uncertain work, exact requests, current CLI, new Svelte client, external capabilities and rollback. Explain which supported states transition, which require draining or explicit refusal, and how users retain their actual work. Do not preserve every obsolete prerelease encoding; do not abandon currently supported data merely to simplify a migration.

## Test families and evidence

Use S01/S02 for a low-friction baseline, S07–S13 for distributed/use-rights boundaries, S03/S06/S14–S16 for adaptation/control, S21–S30 for compound behavior. The program must retain both ordinary usability and the harder value: a two-step workflow alone cannot qualify it.

For important comparisons, independently define the expected result before looking at fixture assertions. Use temporary stores and controlled external providers when executing the existing product; the real daemon must still decide authority, revisions, scheduling, recovery and outputs. New proposed semantics may have modeled traces during planning; label them as modeled rather than executed.

Let an independent reviewer introduce valid variations not given to the design authors in advance. Cases need not be secret forever: retain them and their origin after review. Do not retune the criteria to favor a candidate, or count a model's failure to write code as proof that the architecture is wrong.

Check useful transformations: changing display names/layout must not change execution; changing owner or grant must not preserve invalid authority; hiding a method's internals must not grant edit rights; different task inputs must not share writable state; reconnecting must not create new effects. These are candidate properties whose precise scope must be grounded in the intended contract, not universal assertions about every operation.

Keep a compact coverage matrix with current/proposed status, compared alternatives, evidence, linked decision and required implementation proof. Unknown is legitimate but cannot support a dependent readiness claim. Add scenarios for newly discovered relationships; do not aim for exhaustive combinatorial enumeration or stop solely because this list is complete.
