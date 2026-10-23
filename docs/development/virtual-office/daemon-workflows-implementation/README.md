# Daemon workflows — implementation sprint

**Prompt 03 completed locally; actual user review in 02 is in progress.** The user explicitly instructed
execution of 03 on October 17 and confirmed proceeding with the review left as an evidence gap.
They have since connected Studio A and requested simpler setup, daemon-owned names, the supplied
visual direction and the real logo. The subsequently assigned 02a correction implements simpler
connection and shared daemon naming. Subsequently assigned 02b applies the supplied dark visual
direction and exact logo throughout the client. The user then rejected the forms/navigation and
could not discover multiple-workflow creation. Subsequently assigned 02c implements compact creation,
task addition and workflow navigation. The October 18 recheck rejects its remaining forms and page
layout. Assigned 02d replaces them with canvas authoring, compact left-side selection and direct
source connections backed by narrow daemon contracts; implementation and focused verification are complete. Naming acceptance and
the remaining substantive early review are pending. The October 19 discussion selects daemon-owned
projects and user-controlled folders with project-level execution environments. Assigned
[02e](02e-projects-and-execution-environments.md) owns that correction, including explicit Podman
project attachments. Implementation and focused checks are complete; positive physical Podman
attachment qualification remains open on this host. Resume the actual early user review.
The prepared [canvas experience side sprint](../canvas-experience/README.md) now owns the coherent
replacement under [ADR 0052](../../../decisions/0052-canvas-authoring-and-method-experience.md).
It fulfills outstanding 02 only through actual review and takes over all unimplemented 04 methods.
Main 04 is superseded as an assignment, not completed; side S01–S05 and main 05–07 remain unassigned.
The consumer/CI retirement,
required-input absence and cumulative-time defects pass focused regressions and the complete strict
static gate at `c9e9809`. Publication and a corrected hosted run remain pending. Tested local
checkpoints `37aa8c4` and `95a4dc5` remain preserved. The
[handoff](handoff.md) records the current local review instructions. Agent checks do not complete
the actual user review in 02.
The former remedial P00–P09 is superseded and must not be resumed.

## Outcome and design

A person selects a project on a daemon, starts meaningful work in standalone Svelte, closes the client, then reconnects and finds
its results. The owning daemon accepts model-created research, composition and corrective tasks,
expands the current saved method locally, and uses remote environments as tools. The selected canvas
explains work and its inputs; compact details and a secondary timeline expose actual progress,
dependency blocks, method origins and unresolved effects. Ordinary project files
and Git hold project knowledge; recent daemon history has bounded rolling retention.

Read [vision](../../../product/vision.md) for the short product account and
[ADR 0051](../../../decisions/0051-daemon-task-workflows.md) for the selected technical defaults,
alternatives and responsibilities. [Architecture](../../../architecture.md) labels existing code
separately from this target. [Code-change map](code-change-map.md) names source, consumers, reuse and
removal. [Journey](journey.md) supplies one connected acceptance trace and a different project brief.
These are complementary owners, not competing versions of the design.

The first assignment replaces the ordinary workflow boundary in the existing Rust owners and
connects a production Svelte client. This is substantial: the old graph/publication/learning callers
must retire, while host/Podman effect safety survives. Its first usable checkpoint is an actual
browser-submitted task; it cannot spend the whole assignment on backend cleanup before showing work.
The following phases extend that maintained code. No mock frontend or fabricated success is delivery.

## Prerequisites and standing

Start from the current worktree, preserving newer work. The supplied reference is
`750ebb26607c910ae96c6f6b0f0b3ea12228180c`; it is not a reset target. Branch
`codex/milkdrift-pre-product-reset-2026-10-12` and annotated tag
`milkdrift-pre-product-reset-2026-10-12` preserve `f821ce093742246390f17edc77252ceb75672f97`.
Keep them and the prior v3 evidence/disagreements. The [handoff](handoff.md) identifies the current
tested checkpoints and remaining review; the original planning packet remains available in Git.

Implementation uses a new named development store. Bounded local inspection found no identified
active Milkdrift daemon/user store, but retained 197 test/evidence stores under ignored `target`.
Leave those and existing external services alone. Prompt 01 rechecks for actual preservation
obligations; any newly found live state needs a narrow transition decision. There is no planned
old-store converter, in-place reset or second engine.

Use the pinned Rust toolchain and existing repository tools. Prompt 01 introduces the frontend's
locked package/tool versions, typed client and checks. Actual user participation is required in 02
and 06, including rechecking the connection and visual corrections. Controlled Rust model/tool
endpoints are available as fixtures only; real model use requires
an already configured authorized endpoint/allowance. Do not invent credentials, paid usage or live
service permissions. Missing real-user/external prerequisites remain explicit evidence gaps.

## Ordered assignments

Prompt 03 has completed its focused acceptance. Assigning one prompt authorizes its whole
responsibility and local coherent commits; it does not silently assign its successor.
After the completed 02e correction's focused checks, the next order is
**side S01 → S02 → S03 → S04 → S05 → main 05 → main 06 → main 07**.
Side S03 closes outstanding 02 only with real evidence; S04 owns the former 04 end to end.
The correction labels preserve the original phase numbers; 03 is already done. Existing 04–07
prompts included review/repair but did not explicitly assign this visual reference, real logo or
browser onboarding change. Prompt 05's shared connections concern execution hosts, not browser labels.

| Order | Full execution prompt | Prerequisite | Usable outcome |
| --- | --- | --- | --- |
| 01 | [Connected task and Svelte client](01-connected-task-and-svelte.md) | Explicit implementation instruction | Real task submission/output/discovery/reconnect, canonical fresh-store task path, affected legacy paths removed. |
| 02 | [Early user review and repair](02-early-user-review.md) | 01 focused checks; actual user | A person completes the first journey; functional and usability findings are repaired. |
| 02a | [Simple connection and shared daemon name](02a-daemon-connection-identity.md) | Partial 02 feedback; current 03 checkpoint; explicit assignment | Address/credential setup, automatic identity pinning and a shared, authorized daemon rename. |
| 02b | [Visual direction and real brand](02b-visual-design-and-brand.md) | 02a focused checks; explicit assignment | Supplied dark visual direction and logo across the real client; user rechecks the result and resumes 02. |
| 02c | [Workspace and ordinary task creation](02c-workspace-and-task-creation.md) | Rejected 02b interaction review; explicit assignment | Obvious New workflow/Add task actions, compact contextual authoring and stable navigation; actual user rechecks the journey. |
| 02d | [Canvas and direct authoring](02d-canvas-and-direct-authoring.md) | Rejected 02c interaction review; explicit assignment | Persistent workflow canvas, compact left selection panel and direct source/result connections; protocol forms removed and real daemon metadata supports the interaction. |
| 02e | [Projects and execution environments](02e-projects-and-execution-environments.md) | October 19 selected design; 02d focused checks; explicit assignment | Daemon-owned projects group workflows around ordinary user folders; project defaults bind host/Podman tools consistently and preserve files across environment changes. |
| 03 | [Evolving work and parallel research](03-evolving-work.md) | User authorized proceeding with 02 pending | Ordinary model tasks admit parallel research, composition, correction and bounded continuation through the same commands as humans. |
| 04 | [Method responsibility transfer](04-local-methods.md) | Superseded before implementation | Complete method delivery is owned by side S04/S05; this is not another runnable assignment. |
| 05 | [Remote tools and retention](05-remote-tools-and-retention.md) | Accepted side S05 handoff, including outstanding 02 and transferred 04 coverage | Persistent connection administration, remote uncertainty/recovery and protected rolling retention across clients. |
| 06 | [Combined user review and repair](06-combined-user-review.md) | 05; actual user | The full animation journey and different-project reuse work in production UI; consequential feedback is fixed. |
| 07 | [Final full-system test and repair](07-final-test-and-repair.md) | 06 and all focused checks | Full current Rust/frontend gates and combined journey pass on repaired code; accurate limits and handoff. |

## Working and verification rules

Every prompt reads [AGENTS](../../../../AGENTS.md), canonical docs and the relevant
[implementation](../../practices/implementation.md), [documentation](../../practices/documentation.md)
and [workflow](../../workflow.md) guidance. Review prompts also use
[cohesive review](../../practices/cohesive-review.md). Each assignment completes implementation,
refusal/recovery, all affected daemon/client consumers, removed superseded paths, tests, docs and
evidence. Listed files/cases do not limit in-scope correction. No foundational choice is deferred to
implementation discovery; inspect source to implement the selected responsibilities and catch drift.

Run focused checks with every changed boundary, including 02a/02b/02c/02d/02e, and fix known failures immediately.
Compile affected packages/callers, test intended filters using discovery, run applicable documentation/example checks,
and inspect the diff. Commit coherent tested changes regularly, with explicit staging and
`git diff --cached --check`; preserve checkpoints without squashing, rebasing or pushing.
One coordinator owns shared files and Cargo jobs when agents work in parallel.

This is the explicit multi-phase schedule permitted by
[verification policy](../../workflow.md#explicit-multi-phase-sprint-schedule): **07 is the sole named
local full-system test-and-repair assignment for the remaining main sprint.** The separately prepared
side sprint owns its S05 full gate before main 05; main 07 reruns the current integrated system after
05/06 and cannot inherit a side pass for later changes. Earlier main prompts run complete focused checks, not
repeated workspace gates; they are not full-system acceptance. CI remains enabled and equally strict.
Do not defer a known phase defect to 07 or weaken CI, assertions, discovery or tests to fit the plan.

Frontend checks introduced in 01 are `npm ci`, `npm run check`, `npm run lint`, `npm run test:unit`,
`npm run build` and `npm run test:browser` from `apps/web`. These are maintained package scripts.
Use Svelte/TypeScript checks, accessibility and
focused component tests plus Playwright for actual browser behavior. Node scripts are confined to
frontend build/test work; Rust owns backend fixtures, daemon launch/fault harnesses and protocol
code generation. A browser driver may click or assert UI, but never supply daemon scheduling/recovery.

Keep raw output in ignored `target/daemon-workflows/` and browser artifact paths excluded from Git.
Record source identity, command, discovered/executed cases, environment and limits in the canonical
[verification evidence](../../verification-evidence.md) only when evidence is actually produced.
Update status/architecture/reference/operator docs as their behavior changes. Keep temporary handoff
in this sprint; no progress diaries or duplicate status pages elsewhere.

## Boundaries and next review

No production work, live deployment, spending, host privilege change or data deletion is authorized
by this planning deliverable. The planned GUI, task engine changes and local test fixtures are within
scope when their prompt is explicitly assigned. Inference, new provider families, network mesh,
remote workflow ownership, permanent knowledge systems and unrelated primitives remain excluded.
Do not ask the user to choose routine implementation organization already settled by this packet.

The user assigned [02e](02e-projects-and-execution-environments.md) after 02d's implementation and
focused checks. After the project correction's focused checks, resume the actual
[02 review](02-early-user-review.md) using the current environment
and actions in the [handoff](handoff.md#resume-the-visual-and-task-review) through side S03 after the
side's implementation is assigned and its first two phases complete. The old main 04 must not run.
Have the user choose a
project, create useful work, locate its files, connect an input/result, switch between tasks and
recheck naming, model assignment and execution environment. Record what they
actually do and where the interface still gets in their way. Their earlier feedback remains valid;
automated passes do not establish acceptance. Preserve existing stores, services and checkpoints.
Main 04 is superseded by the separately assigned side S04, following actual side S03 acceptance.
Side S05 owns that sprint's full gate; main 07 later verifies the integrated remaining main work.
