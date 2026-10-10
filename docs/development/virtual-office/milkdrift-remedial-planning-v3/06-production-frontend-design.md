# 06 — Specify the real standalone Svelte product and its engineering practice

**Owner:** product/interaction designer and frontend engineer, cross-reviewed by semantics and authority owners. **Output:** build-ready production interface specification, public-operation remedies, and frontend practice proposal. **Prerequisites:** 01–03 reviewed evidence/goals/product meanings, and initial 04/05 browser/architecture/notation work. This phase revises those proposals through actual interaction requirements; it does not simply decorate a frozen architecture. **Stop:** production design and implementation requirements; no prototype, mock app or production code yet.

Read [context and preservation](context-and-preservation.md), the dossiers/decisions, [scenarios](scenario-corpus.md), [implementation](../../practices/implementation.md)/[documentation](../../practices/documentation.md) practices and [public API policy](../../../reference/public-api-policy.md).

## The interface is a consumer and a test of the model

Design a maintained Svelte application that connects to actual authorized Milkdrift owners. It starts with zero connections, works with several independent workflow daemons and execution hosts, and remains useful when one is unavailable. Svelte is first; Iced is not this assignment, and a possible later Tauri wrapper must not dictate hidden browser semantics.

Do not define success as a polished demonstration. The implementation program must build real connected functionality from its first usable slice and retain that code. Test fixtures may control external models/networks; they may not substitute invented daemon responses as evidence that the product path works.

At the same time, this phase is planning. Produce precise surfaces, interactions, state transitions, data/action contracts and acceptance tasks. Do not build a fake working UI to make design choices seem validated.

The UI may coordinate presentation, forms, unfinished drafts and ordinary API requests. It may not own execution rules, permission decisions, durable results, service authority or a parallel workflow compiler. Client-side form checks can improve feedback, but the daemon remains authoritative. “Thin client” does not mean “no frontend design or state ownership.”

## Work outward from what the user needs to accomplish

Test the strongest interface alternatives from 03 against the reviewed 02 goals: manual diagram work, goal/agent-proposed work and hybrid use. Describe how a person first connects, finds useful operations, supplies a goal or edits a method, chooses execution resources, reviews consequences, runs work, understands failure, revises future work and reuses the result.

The first-use journey must be simple, but the whole specification must retain the hard outcomes V02–V12. Do not quietly redefine Milkdrift as a one-daemon model picker because those screens are easy. A small first release is an integration sequence, not authority to discard the rest of the product.

Use representative user roles: author, invoke-only service caller, human reviewer, restricted collaborator, multi-daemon operator and agent proposing a change. These are situations, not hardcoded application roles or implicit grants. Derive available data/actions from actual authority.

Design one navigational model covering workflows and reusable calls, direct work, active/history inspection, managed resources, capabilities, approvals/adaptation and learning. These are coverage areas, not a required menu item for every noun. Explain what is visible, nested, combined or intentionally unavailable and why. Missing public functionality must be an assigned backend remedy or an explicit unapproved exclusion.

## Specify surfaces as interactions with real owners

For each substantial surface, link the reviewed product outcome/decision and document the user's decision, fully qualified data source, actual endpoint/operation or exact gap, action inputs, relevant guards, expected effect, recoverable identity and safe next action after errors. Label current public capabilities separately from planned API changes. A proposed button is not evidence that an operation exists, and an attractive interaction does not authorize changing the product's purpose. Explain who cannot use it and what they see instead.

Cover blank/loading/empty states, disconnected owners, permission denial, stale data, conflicting edits, unavailable capability, pending approval, uncertain effect, failed verification and completed results. Avoid one generic error toast for states with different recovery consequences. Lost observation does not imply lost work; a disabled UI control does not provide authorization.

Describe agreement between related views: editing a draft versus saving a revision; selected catalogue versus pinned accepted operation; a method's public result versus its private execution; current plan versus historical occurrence; connection removal versus service removal. Treat changed grants and host identity as explicit cache/draft/recovery events, not normal navigation.

Use the notation profile for graph/edge semantics and runtime overlays. Include expansion/collapse, input wiring, branch/loop controls, selection, layout, revision comparison, prospective changes and inspection. Do not render only the most convenient node types and silently omit the rest. A legitimate read-only view is allowed when editing is not permitted, but may not substitute for an intended authoring outcome without explicit scope approval.

Map each promised interaction all the way through public operations to owning code or a defined change. Multiple API calls are not automatically a design bug: uploads, verification and effects may require distinct steps. But require a reason when a single human action causes needless coordination. Prefer a coherent owner-side operation over a browser script that fabricates atomicity or rules.

## Establish reusable frontend engineering practice

Write a proposed `docs/development/practices/frontend.md` for later adoption, linked from the existing practice selector after approval. It should be useful beyond this exact screen set without becoming a generic encyclopedia. Decide and justify:

- Svelte setup and build/routing approach; whether SvelteKit is needed; browser deployment versus possible later desktop wrapper;
- feature/state/component organization, dependency direction and public transport boundary;
- generated or maintained protocol types, strict response decoding, coordinated protocol upgrades and error semantics;
- per-connection session state, owner-qualified cache keys, stale-response rejection, independently cancelled requests and stream cleanup;
- unsaved drafts, optimistic edits/conflicts, layout state, saved exact mutation requests and reload recovery;
- credential custody, browser origin policy, proxy/broker trust where chosen, untrusted output rendering and private cache handling;
- component/style tokens, assets/icons, graph rendering/layout library selection, focus/keyboard behavior and accessible non-graph editing;
- unit, contract, real-daemon browser, accessibility and visual-regression tests with clear evidence limits;
- lint/typecheck/build rules, dependency pinning/update policy, test diagnostics and CI integration;
- limits and measurement of graph/history rendering, subscriptions, caches, input sizes and retained private data;
- package documentation, operator setup, local development and backend/frontend change coordination.

Choose mechanisms from concrete consumers. No generic “common” framework, giant global store, per-feature transport copy or second credentials authority without a demonstrated need. Review server-side state isolation if SSR is proposed; frontend state may accidentally live on a shared server rather than in one user's browser.

Use current primary documentation, including [Svelte](https://svelte.dev/docs/svelte/overview), [SvelteKit state management](https://svelte.dev/docs/kit/state-management) where applicable, browser specifications and the selected library's own docs. Do not transfer deployment decisions from another project or assume every package exists in the pinned version. Record the versions evaluated; implementation verifies them again before adoption.

## Visual quality must help real work

Specify legible typography, hierarchy, spacing, usable density, sharp surfaces, focus states and restrained motion. Avoid neon/glow/blur standing in for hierarchy. Use the unchanged [supplied Milkdrift symbol](assets/milkdrift-logo.svg) as a brand asset, not an execution metaphor; do not redesign it without reason. Address asset paths and small-size treatment without bundling font binaries.

The diagram needs deliberate edge routing, readable labels/ports, distinguishable states beyond color, and a way to inspect dense or long-running work. Specify zoom, keyboard focus, scrolling, long labels and narrow-window behavior. A beautiful static composition that hides connection or failure state is a bad product design.

Provide annotated layout/interaction specifications sufficient to build production components. Design drawings may explain the proposal but do not count as a usability pass. Only later use of the actual connected frontend by the user can establish that feedback.

## Require difficult connected journeys

Trace S01/S02 and the compound S21/S22/S26/S28/S29 as appropriate, plus the distinct service/publication, direct-execution, denied-evidence and learning paths. State exactly where the API is sufficient and where a backend change is required. Compare the narrower ordinary editor with the desired diagram/agent authoring behavior; no client-side workaround may erase unsupported semantics.

Plan a first real user-review checkpoint soon after a retained connected path works, followed by the corrections that feedback may require. This checkpoint may reopen product meanings and architecture, not merely colors or layout. Classify feedback as presentation friction, missing public behavior, mistaken product assumption or model/provider limitation before selecting a remedy. Recheck affected dependencies and preserve the user decision. Do not silently constrain this review to styling because backend work was already approved. The program must also have a later compound-work checkpoint demonstrating that advanced capabilities work **together**, not just in independent menu demonstrations.

## Output and gate

Write `working/production-interface.md`, `working/frontend-engineering.md`, and a practice proposal within the final deliverable area. Keep each current decision in one place; link unresolved whiteboard questions and the backend remediation plan rather than duplicate them.

Have an independent reviewer attempt the specified interactions using only the intended public contract. Missing operations become concrete findings, not invented clicks. Check both authorized progress and restricted views. Reopen product/ownership/notation decisions when needed.

Commit coherent design sections and checks. **Gate:** implementers know what to build, why it matters, how it reaches real owners, what fails, and what remains unproven. No essential Milkdrift ability has disappeared through “first version” wording, and no cosmetic frontend is being used to certify unresolved core semantics.
