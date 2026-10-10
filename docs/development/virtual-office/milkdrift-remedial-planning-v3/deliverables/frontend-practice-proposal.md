# Frontend practice — proposal for adoption

This proposed practice began with Bram's FP1 and is revised by Delta for F1/r4 and F2/r4,
2026-10-10. Its intended canonical home
after P00 adoption is `docs/development/practices/frontend.md`, selected through the existing
practice guide. That canonical file and selector have not been edited in this planning assignment.
The adopter must adjust relative links for that home. Product scope, architecture and verification
remain with their existing owners; this practice does not independently authorize GUI work.

Use this practice when building or reviewing Milkdrift's authorized Svelte client. Combine it with
the [implementation](../../../practices/implementation.md) and
[documentation](../../../practices/documentation.md) practices when their work is involved.
Use the [workflow](../../../workflow.md) for assignment scope and completion. Version and
package-specific choices belong in the maintained application README and lockfile, not a stale
catalogue of preferred tools in this practice.

## Trace an interaction to its real owner

Begin with the person's decision, what they can observe, the exact public command/read and the
failure/recovery path. Inspect producers, readers, consumers and tests before calling a missing
button a backend gap. Follow preparation through acceptance, execution and result: a declaration,
accepted request and completed effect are different facts.

The browser owns presentation, selection, unfinished input, local drafts and bounded ordinary
request coordination. It consumes daemon-owned workflow semantics, authority, immutable identities,
results and persistence. It must not acquire a second workflow compiler, permission engine,
execution ledger, service scheduler or canonical hash implementation. Form validation may improve
feedback; the owning public operation still validates and refuses. A compound interaction must
produce the same accepted meaning through graph, outline, API/CLI and agent consumers.

Begin ordinary interaction with goals, inputs, expected outcomes, permissions, budgets and acceptance
criteria. Generated port IDs, account bindings and request mechanics belong in an inspector when
they explain a decision; they are not prerequisites for expressing intent. Names and plain-language
defaults must correspond to exact public semantics, not replace required authority or bounds.

## Edit one canonical source and preserve historical meaning

The selected NP4 source is a typed family with structured `Program` as the new convenience default
and explicitly versioned `GraphNative` support. Each immutable revision has one authoritative
source; both lower through blueprint to one checked execution plan and use the same runtime and
reconciliation. Diagram, accessible outline, forms and machine consumers submit the appropriate
complete owner-validated edit operations. Existing graph construct/genesis, import, copy and edit
APIs retain their bounded semantics, with no upgrade-age cutoff or migration eligibility registry.
Their convenience compiler/recognizer moves into shared blueprint helpers, preserving old accepted
shapes and refusals. An old command version must not silently compile to the new source family.

The browser may keep unfinished draft input and layout, but no second editable representation of
one revision. A Program diagram is derived; GraphNative edits address its actual graph source.
Neither may infer missing semantics or silently repair the daemon's returned document. Program
conditional order is explicit priority distinct from identity; graph-v3 retains its exact old order.
Program's typed value source does not require duplicate data-edge maintenance. Parallel completion,
result availability and unsettled effects remain distinct in either view.

Conversion is an explicit owner operation creating a new revision with origin and semantic
correspondence. Show precise unsupported relationships and keep the supported graph-native
edit/proposal path available when conversion refuses. Do not drop unknown constructs, reinterpret
old identifier-based priority or make a lossy save look like a harmless format upgrade. Graph-v3
retains its complete existing edit vocabulary, including inactive reusable definitions and
compound governed repairs; it need not acquire every new Program feature. Source equivalence is
required for a claimed unchanged conversion, while an intentional future-work revision has the
separate live-adoption obligations below. A cancelled run plus new work is not equivalent repair.

Separate successful source validation from permission to adopt it into an active run. Prospective
adoption preserves completed evidence, selected clauses and active/uncertain occurrences under their
governing plans. Show the exact affected pending frontier, current guard and required approvals;
refuse active reparenting, changed consumed inputs or incompatible result/protected contracts.
Pause does not prove external work stopped. Valid immutable sibling saves are not automatically
conflicts; actual stale run/layout/envelope guards retain their distinct meaning.

## Make ongoing control and evidence inspectable

Consume the selected WorkCommitment owner through ordinary public create, inspect, criticism,
answer, pause/resume/cancel and authorized proposal operations. The UI must remain a client when
the tab closes: it owns no reconsideration loop, task scheduler, account establishment or result
ledger. A commitment projection links accepted coordination facts and associated runs; it does not
copy their lifecycle into a new client truth. Distinguish current reasoning and claims from accepted
evidence, pending decisions from entered effects, and stopped coordination from unsettled work.

Criticism triggers assessment under the accepted policy. Present retaining the original conclusion,
further investigation, prospective revision, escalation and stopping as legitimate outcomes.
Expose cumulative limits, reserved/uncertain consumption, progress-rule stops and questions that
need a precise answer. A newly generated plan, run, retry or context does not create new allowance.
Give pause and cancellation a usable recovery path even under ordinary storage pressure; their
truth comes from the owner's final-entry and settlement facts, not a locally disabled button.

Knowledge assessment need not start from an executable method or managed resource. Keep exact
sources/versions/outputs, criteria, reviewer identity and authority, judgment, reasons, limitations
and counterevidence visible. Distinguish human/agent judgments from automated verification; label
retrospective assessment and author self-review honestly. Known participation/shared sources and
unknown independence remain visible. Do not turn multiple agreeing reviewers into an independence
claim or a favorable judgment into a passed mandatory check. Preserve negative and inconclusive
outcomes in discovery and fresh-context selection.

Keep evaluation, approval for selected knowledge reuse, executable-method adoption and publication
separate actions with their own authority and exact receipts. An approved lesson remains bounded
by its applicability and evidence. The browser must not silently replace a method, reinterpret an
old protected learning record or manufacture trusted verification from an uploaded assessment.

Use existing meaningful public operations before adding endpoints. If the operation is missing,
define its owner, inputs, current authority checks, durable effects, bounds, idempotency, errors
and all affected consumers. A backend correction must be delivered with its frontend consumer
and refusal/recovery evidence. Do not simulate success in the client while waiting for the owner.

## Keep frontend boundaries small and explicit

Separate transport/decoding, connection sessions, local recovery and feature interactions. Route
components compose those boundaries. Presentation controls receive data and actions rather than
fetching arbitrary endpoints. Reuse the same transport/session implementation across features;
avoid a generic framework or giant global store that obscures who owns cancellation and state.

Choose Svelte/Kit routing and rendering deliberately. A static client must specify deep-link
fallback, base paths, asset/HTML versioning and actual origin topology. A dev proxy does not prove
the deployed browser route works. If server rendering or server functions are proposed later,
review isolation and credential custody before using shared module state. User data must never
be shared across server requests because a client-store pattern was copied to a long-lived server.
Consult the current primary [Svelte docs](https://svelte.dev/docs/svelte/overview) and
[Kit state guidance](https://svelte.dev/docs/kit/state-management), recording versions evaluated.

Backend executables, fixtures and orchestration remain Rust under repository policy. TypeScript
may implement browser UI and browser tests, not a replacement backend/evidence engine. New package
or cross-feature abstraction requires a demonstrated shared contract. Prefer feature-local modules
and private interfaces until reuse proves a broader owner useful.

## Decode and preserve the public contract

Negotiate the supported protocol before ordinary operations. Rust constants/readers/fixtures
own wire meaning. Maintain or deterministically generate frontend projections with contract drift
checks; either choice still requires strict runtime decoding of untrusted responses. Type assertions
and generated interfaces do not validate network input.

Bound bytes before parsing and validate required fields, tags, depth, arrays, strings and diagnostic
shapes. Treat unsupported executable meanings as incompatible, not as generic editable tasks.
Never drop unfamiliar fields and save the resulting reduced document. A read-only compatibility
view must state its limits and preserve the original document for supported handling.

Preserve exact numeric guards and identifiers. JSON numeric integers may exceed JavaScript's safe
integer range: use a qualified lossless decoder/serializer and exact internal representation for
such fields. Test values on both sides of `2^53` and at protocol extrema. Native `Response.json()`
followed by a cast cannot recover rounded digits. Arbitrary nested data and exported exact request
records must round-trip without silent numeric changes.

Classify network failure, unknown response, malformed protocol, refusal, conflict, unavailable
capability and observed execution failure separately. Expose actionable owner diagnostics without
logging private payloads. A client-generated optimistic state is not server acceptance.

## Bind every view and request to a connection

Distinguish endpoint URL, local label, stable host identity, actor and exact grant. Authorized
cache partitions include these authority facts and object/revision/generation. A connection epoch
guards against late responses after logout or switching; cancellation alone does not prevent old
promises from updating the current view. Never union grants across two connections to one owner.

Changing host or authority invalidates authorized read caches and observation cursors. Private
drafts/exact requests retain their original binding for recovery; they must not migrate silently
to a different actor or replacement host. Reauthenticate and recheck before recovery. Cache metadata
may itself disclose private identities, so removing content while retaining forbidden links is
not sufficient masking.

Recovery follows each operation owner's actual contract. Do not generalize a saved-workflow
helper's exact-authority check to a direct-serving path that can authorize retained replay using
current disclosure rights and its stored original basis. Conversely, permitted current reads do
not imply permission to replay every old command. Keep original records unchanged and test these
positive and denied cases through their public owners.

A route owns its subscriptions and disposes them when no longer needed. Streams have finite
frame/queue limits, last fully handled cursor and explicit resync. Preserve the distinction between
durable run cursors and process-local capability/health incarnations. A complete empty snapshot
replaces the old projection. Closing a view or aborting fetch stops observation, not accepted work.
Expose actual cancellation through a separate scoped command and inspect its result.

## Make local retention and recovery deliberate

Choose credential custody and local private-data retention explicitly for the supported deployment.
Session-only bearer credentials must not leak into browser persistence, URLs, history, exported
files or logs. If a proxy/cookie/native credential mechanism is proposed, name its new custody,
origin/CSRF and recovery responsibilities; do not inherit assumptions from bearer fetch.

If identity discovery requires authentication, trusted origin/TLS governs credential delivery
before stable host comparison. Do not claim a post-authentication host check protected a bearer
from the endpoint that already received it. State that limit or design a stronger explicit owner.

Retained browser drafts and requests are private local data. Explain whether storage is encrypted
and what logout does. UI authentication gating does not encrypt IndexedDB or protect a shared
unlocked browser profile. A personal-profile retention mode and a session/export mode have
different recovery costs; the product must not silently infer trust from the presence of storage.

Before an effectful submission, freeze the exact request identity/body/guards and original authority
in a recoverable versioned record or a deliberate completed export. Storage failure must block
the send, not leave an unknown effect without its locator. Lost response retains that original
request; exact replay never invents a new identity or refreshes its guards. A new intended action
requires a newly reviewed command. Backend idempotency, not a tab-local mutex, prevents duplicate
effects after crashes.

Bound retained drafts and requests by bytes/count. Reproducible read caches may use eviction;
pending/unknown submissions may not. Keep bounded reserved capacity or explicit export/native-client
recovery for urgent allowed cancellation/control, so a full ordinary store does not silently
remove useful stop actions. Full protected capacity requires resolution or explicit
export before accepting more effects. Logout clears credentials/authorized views while preserving
locked recovery records according to the chosen policy. Forget/clear distinguishes local evidence
deletion from backend cancellation or cleanup. Unsupported local record versions remain exportable
and must not trigger a destructive database reset.

## Give every user an operable representation

Use semantic native controls and explicit labels before custom widgets. Complex diagrams need
complete keyboard and non-dragging pointer interactions plus a usable outline/form representation.
Keyboard support alone does not satisfy a non-drag pointer requirement. Every meaningful graph
edit, policy, data binding and diagnostic must be available in the alternative representation.

Choose graph/layout/headless-control libraries using current maintainer documentation and actual
complex fixtures. A graph library owns rendering, not execution rules; layout changes positions,
not edges or grants. Disable default destructive gestures until they enter the controlled draft
edit flow. Library accessibility claims are evidence to investigate, not certification of the app.

Use a small token system for spacing, typography, contrast, state and focus. Preserve approved
brand assets unless redesign is authorized. Do not communicate state only by color, motion or
hover. Long labels, partial pages, hidden graph neighborhoods and restricted content need honest
text. Avoid decorative effects that obscure hierarchy or connection/uncertainty status.

Test focus after navigation, failed validation, modal completion and live refresh. Provide meaningful
page titles, restrained live announcements, visible/unobscured focus, text zoom/reflow, forced
colors and reduced motion. Verify the actual composed application against the selected WCAG target
with both tools and manual interaction. [WCAG 2.2](https://www.w3.org/TR/WCAG22/),
[dragging guidance](https://www.w3.org/WAI/WCAG22/Understanding/dragging-movements.html).

Treat model output, logs, filenames, artifacts and diagnostics as untrusted content. Render text
or reviewed typed views by default. Rich Markdown/HTML requires an explicit sanitization and URL
policy, never raw markup injection. Restrict packaged scripts and network origins according to
the supported deployment; document CSP constraints when users add hosts. No invisible analytics
or diagnostics upload may capture private work.

## Qualify production interactions and keep evidence honest

Pin toolchains, direct dependencies and the complete package lock; document license/peer/runtime
requirements and deliberate install-script policy. Use reproducible clean installation, strict
typecheck, lint, formatting check, focused unit/component checks and a production build. Upgrade
dependencies with the meaningful affected browser/keyboard/contract checks, not just compilation.

Unit tests cover client logic and boundary errors. Contract tests compare Rust-produced wire
documents with frontend decoders/serializers. Browser integration uses the actual static build
and real authorized daemons with controlled external capability fixtures; fake daemon replies
do not qualify an end-to-end product path. Record browser versions, topology, protocol, source
checkpoint and actual checks. Test permitted completion together with denied/stale/unknown cases.

Include lost replies, browser/daemon restart, changed host/grant, malformed/truncated streams,
resync, storage overflow, concurrent edit, restricted artifacts, cancellation uncertainty and a
second usable owner while another fails. Visual snapshots and automated accessibility scans
supplement keyboard, pointer and screen-reader review; they do not prove semantic behavior or
human comprehension. Sanitize diagnostic traces and explicitly protect any private evidence lane.

Include typed source reload through diagram/outline/API, Program ID renaming without choice-priority
change, exact old graph semantics, versioned graph creation/import/copy/edit, precise conversion
refusal with positive pending-work repair, and unchanged old-command recovery after default changes.
Ongoing-work tests
must include rejected criticism with retained reasons, an authorized revision, no-progress stop,
restart without a duplicate model call, and pause/cancel while effects remain unsettled. Knowledge
tests need a nonexecutable item, retrospective and controlled bases, allowed self-review, a refused
independence requirement, and favorable evaluation that confers no publication authority.

Measure graph/history workloads, request/subscription counts, bytes, memory and interaction latency
on named devices/browsers. Set explicit bounded operating policy and truthful overflow behavior.
Virtualization may limit rendered elements without dropping semantic objects from a saved draft.
Do not claim maximum-scale support from a small demonstration.

The first retained useful connected slice needs an early real human review. A later compound
review must combine advanced capabilities and recovery, not merely visit each menu independently.
Classify feedback before repair: presentation friction, missing public behavior, wrong product
assumption or model/provider limitation. Reopen affected decisions when warranted. Follow the
workflow's focused/full-gate policy and report actual limits; passing mocked browser tests or
writing a plan does not establish production acceptance.

For the present proposed program, full source ownership is P01; the first connected Svelte slice
and browser foundation are P02, followed by its real human checkpoint. Ongoing commitment control
is P04, knowledge assessment is P07 and the combined experience is P08. These phase assignments
belong to that program and should be omitted when this reusable practice is adopted canonically.

The maintained package README teaches setup, supported deployment/browser matrix, credential
provisioning, private retention/recovery, normal commands and troubleshooting. Canonical product,
architecture, protocol and verification facts remain in their owners, with links instead of
duplicated inventories. Temporary design debate and observations stay in the virtual office.
