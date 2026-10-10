# A production workbench for inspectable, adaptable work

F1/r3 and F2/r3 propose a maintained standalone Svelte application that connects to real Milkdrift
owners. The proposal is ready for an implementation decision with explicit qualifications; it is
not a prototype, working application or usability result. The current negative browser observation
shows why transport work is required before the first connected journey can pass.

The user sees four connected destinations: **Work** for methods, drafts, runs and future changes;
**Operations** for direct tools and public services; **Resources** for enduring installations and
their editing/lifecycle obligations; **Improve** for selected knowledge, declared comparison and
prospective promotion. Owner, actor and freshness remain visible. Common discovery helps find
useful work, but the committing actions keep exact reference, independent copy and public invocation
distinct. Private internals are never invented to make a public service look like an editable node.

Saved definitions remain immutable and may have valid sibling revisions from concurrent authors.
The UI shows ancestry and the explicitly selected revision; it does not invent a global mutable
head or report every concurrent save as a stale conflict. Actual run, layout and envelope guards
retain their distinct refusal behavior.

The first retained journey connects to an actual daemon, authors a small method with named input,
validates and saves a revision, supplies a brief, runs it and inspects the accepted result. A second
brief exercises reuse. The user then reviews this real application at P03, before most advanced
work is built. Their feedback may change the product or public API, not only styling. The program
continues through complete structured authoring, services/direct work, controlled adaptation,
managed resources and evaluated improvement. P08 tests those together in demanding combined work.

## What implementers can build from this proposal

The [full interface specification](../working/production-interface.md) contains annotated shell
layout, responsive dimensions, every substantial action's current public operation or named remedy,
inputs/guards/recoverable identity, diagrams and non-graph authoring, and the blank/loading/denied/
stale/conflict/approval/uncertain/failed/completed states. It covers ordinary and advanced work,
publication/invoke-only use, independent owners, resource handoff/return and negative/inconclusive
learning. These tables are the build annotations; this brief does not duplicate their authority.

The [engineering contract](../working/frontend-engineering.md) specifies `apps/workbench`, static
SvelteKit 3/Svelte 5/TypeScript 6, Node 24.21.0 LTS and npm 12.2.0, narrow feature/transport/recovery
owners, exact numeric wire handling, browser state partitioning, bounded storage/streams/caches,
test scripts and a Rust-owned actual-daemon fixture lane. Package compatibility and exact remaining
library patches must be verified and locked at P01. Svelte Flow 1.6.3 is conditional on actual
complex graph and accessible authoring tests; native controls and a complete outline are required.

The [frontend practice proposal](frontend-practice-proposal.md) is prepared for later adoption
at `docs/development/practices/frontend.md`. It supplies reusable methods for owner tracing,
protocol/session handling, exact recovery, accessible rendering, dependency/test policy and honest
evidence. Canonical practices and selector have not been modified by this planning task.

## Consequential choices to review

| Choice | Proposed consequence | Strong alternative and remaining proof |
| --- | --- | --- |
| Browser deployment | Operator/user-hosted static build with approved endpoint set in CSP; each daemon permits its exact frontend origin; remote TLS proxy retains loopback daemon binding | Fixed-target same-origin proxy can reduce setup friction. General public-hosted arbitrary-endpoint use remains unqualified. P01 tests actual release CORS/CSP/HTTPS and selected browsers. |
| Credential custody | Bearer in tab memory only; reload requires provisioning again | Persistent login or native vault needs a separate custody design. Authenticated identity reads mean trusted origin receives bearer before stable host comparison. |
| Private drafts/exact requests | Explicitly chosen personal-profile IndexedDB retention, unencrypted at rest; no trust inferred | Session/export mode avoids persistent private browser data but requires exact files before effects and can lose unsubmitted drafts. P03 must test this real friction. |
| Recovery under pressure | Never evict pending/unknown requests; reserve finite control capacity, then export/authorized native route | There is no unrecorded emergency-send bypass or unlimited queue. Clearing browser data outside the app can destroy its only locator. |
| Changed authority | Clear authorized views, quarantine original records; workflow and direct operation recovery follow their own owner contracts | A known-object read is not old-command replay. Direct serving may permit exact recovery using current Inspect and stored original basis; never silently rebind a request. |
| Authoring meaning | Graph, outline and agent input share daemon construction; UML Activity-grounded notation with explicit Milkdrift qualifications | Canonical regions and full standard execution remain serious alternatives if complete cases expose repeated exceptions; no interoperability claim now. |
| Exclusive reconvergence | R-C01 adds a bounded same-scope choice continuation, with selected result/provenance and compatible future repair | Current direct convergence is refused. An explicit pinned child remains a different boundary, not a hidden equivalent. New merge must not erase nested uncertain effects/holds. |

The supplied Milkdrift SVG is unchanged. The proposed visual system uses sharp dark surfaces,
cream text, restrained blue emphasis, explicit state labels, visible focus, system fonts and
44-pixel ordinary controls. Colors and hierarchy remain candidates until rendered contrast and
human checks. Keyboard and non-dragging pointer paths must support the same work as the diagram;
screen-reader/zoom/forced-colors checks supplement automated accessibility tests.

## Backend work is named, not implied by buttons

Existing public commands already cover immutable construction, ordinary model editing, uploads,
runs/results, proposals, direct/public invocation, publication and managed/learning actions.
[The source-backed API map](../working/frontend-evidence.md) distinguishes these from actual gaps.
R-A01/02 complete browser admission and client integration; R-A03 adds scoped installation/recipe
discovery; R-A04 adds the independently authorized public-call→internal association; R-A05 first
tests narrow-grant method-list behavior. R-C01 owns reconvergence. R-FE01 adds bounded learning
receipt discovery from the existing owner, and R-FE02 proves or completes a maintained goal-planning
consumer using ordinary model/proposal contracts. No browser scheduler or second durable study
database fills those gaps.

The [P00–P09 program](proposed-implementation/README.md) assigns each complete boundary, actual
human checkpoints, focused checks and final full-system repair. [F1's decision topic](../../whiteboard/discussions/implementation/production-frontend.md)
retains the strongest alternatives and actual cross-review; [BR1](../working/review-first-bram.md)
records challenges and dependent rechecks. No implementation, new runtime probe, package install,
paid model call, live deployment change or human-use pass was performed by the frontend author.
