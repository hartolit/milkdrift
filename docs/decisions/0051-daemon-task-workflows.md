# 0051 — Daemon-owned tasks and locally expanded methods

**Standing:** selected engineering design, 2026-10-12, with the October 17 connection correction
below selected after partial user review. Implementation through prompt 03 and the assigned 02a/02b
connection and visual corrections is recorded in status. The subsequent workspace interaction
correction was implemented in assigned 02c and rejected in the October 18 user recheck. The canvas
replacement below is implemented and passes 02d's focused checks; it is not user-accepted. Later phases
require their own assignments.
[Vision](../product/vision.md) owns the requirements;
[status](../product/status.md) owns implemented facts. The
[implementation sprint](../development/virtual-office/daemon-workflows-implementation/README.md)
owns sequencing, code changes and acceptance.

## Context and decision

The user wants workflows that grow as ordinary tasks execute, with a phone-independent daemon,
Svelte, current-at-activation method templates and remote environments used as tools. The earlier
P00–P09 program is superseded. This decision replaces its region/GraphNative, WorkCommitment,
published-workflow and evaluated-learning requirements; those are not prerequisites for this product.
It preserves scoped authority, truthful uncertainty, bounded storage and independent host safety.

Use one task dependency graph in the existing semantic/runtime owners. Retain the useful Rust
capability, authority, persistence and adapter mechanisms. Replace the old control-token graph and
its public revision/proposal ceremony rather than lowering a second task language into it. The
existing runtime remains the only scheduler. Internal immutable task definitions and optimistic
generations explain changes without exposing a Git-like hierarchy to users.

The following choices are engineering defaults, not quotations from the user. They are concrete
enough to implement; evidence from the required user reviews can justify changing them.

## Tasks, inputs and files

A workflow has a daemon-issued ID, title, brief, owner/access scope, project-root grant, monotonic
edit generation, lifecycle and cumulative allowance. A task has a stable workflow-local ID, title,
description, instructions, named bindings, explicit dependencies, configured capability selection,
output declarations and optional origin/addresses links. Research and composition are roles, not
types. A method use is a scheduler expansion item with bindings, not a model implementing templates.
Tasks are required by default; an authorized edit can declare an unstarted task optional. Completion
tracks required task IDs and output contracts. A typed adopt-correction action names one original
task and a successful substitute with compatible outputs, redirects the completion obligation and
explicitly listed pending bindings, and preserves both histories. Reject adoption cycles, competing
adoption and changes to started consumers. The operation requires scoped workflow-edit authority;
an agent cannot waive requirements outside that grant. No separate agreement entity is introduced.

Bindings are inline text/typed values, supplied workflow inputs, captured files, exact task output
references, or settled task outcomes. A task-output reference also declares a dependency; validation
refuses an inconsistent separately supplied edge. Required-success is the default: a missing value
keeps a task queued with a reason; producer failure blocks it. An explicitly optional input has a
typed absent value, never an empty-string substitute. A settled-outcome input waits for completed,
failed or proven stopped work and receives its outcome plus available results. Uncertain and
stop-requested work is not settled. Dependencies are acyclic and all conditions must hold. Users can
supply inputs, revise unstarted bindings, or add corrective tasks and rebind pending consumers.

At start the runtime freezes the definition, selected input bytes/digests, provenance, capability
generation and authority basis in one durable start. File paths alone are not frozen inputs. The
host captures authorized files into bounded artifacts before entry and confirms the accepted start
uses those bytes. Text files reach models as bounded text; unsupported binary inputs return a clear
refusal rather than relying on currently unsupported generic file parts. Task results have immutable
references even when tasks also write ordinary files.

Work directories belong to explicit configured roots. Parallel research gets separate writable
subdirectories. Managed-installation editing claims do not cover arbitrary native roots: add a
small private durable path-claim owner in capability-host using persistence ports. Tasks declare
read/write roots; write/read or write/write claims conflict on equal or ancestor/descendant physical
paths. An undecomposed task claims the whole project root. Resolve path aliases and refuse ambiguous
symlink/root changes before entry. Retain claims until durable settlement or authorized reconciliation,
including uncertainty/restart. Managed roots also retain their installation claims. This coordinates
declared Milkdrift work, not out-of-band native writes or a new sandbox. Reading another task's
results requires a declared binding/access decision, not shared directory discovery. Native tools
retain their documented trust limits; this does not claim arbitrary local processes are sandboxed.
Models request permitted tools through a bounded Rust model/tool loop in capability-host. Each tool
call uses normal authority, budget, entry and reporting; the loop never schedules workflow tasks.
Existing coding-agent process profiles may also perform an ordinary task. No inference or new provider
family is added. Text/tool output is bounded and recorded with its provenance before the next model call.

## Commands, concurrency and automatic continuation

Humans and models call one control-owned typed batch: add tasks/uses, revise or remove unstarted
items, bind inputs, link/adopt corrective work, and request continue/wait/finish. Each mutation supplies
workflow generation and exact idempotency key. The whole batch validates references, cycles,
authority and bounds before one durable acceptance; either all changes commit or none do.
New items use batch-local aliases; acceptance allocates daemon identities and returns the exact
alias map in the receipt. Models never need to guess future IDs. Optional producer-output bindings
still wait for their declared dependency to settle; optional supplied values may explicitly be absent.
Generation conflicts return the current generation and affected IDs. Humans refresh and review;
a permitted agent receives bounded conflict feedback and may propose a new batch against the new
snapshot. There is no blind resubmit or automatic semantic merge. Start races serialize: either an
edit wins before the start snapshot, or it is refused as already started.

A model returns results and an optional structured action batch. Save its immutable response,
digest and pending admission record before processing additions. Recovery resumes that record with
the same key. Once accepted, response/admission linkage, new IDs and receipt are durable together.
A duplicate response cannot create duplicate children. Invalid actions leave results inspectable and
record rejection. Out-of-grant actions wait for a human decision; models cannot enlarge their grants.
For invalid/conflicted batches, an explicitly granted automatic-repair policy can admit an ordinary
follow-up task containing the response, diagnostics and refreshed snapshot. Control constructs it
through the same checked batch path and cumulative account, with an exact source-response key.
Otherwise the workflow waits for the user. This is how correction is invoked; a browser or fixture
does not secretly drive the next model call. Failed corrective lineages and rejection streaks stop
at the limits below, including rejected feedback tasks; they cannot create an unbounded retry loop.

Completion requires an explicit finish request and satisfied required tasks/results, with no queued,
running, blocked, undecided-admission or uncertain work. Finite human-authored work may request
finish-when-settled on submission; agents request finish in their final batch. Empty runnable queues
without finish mean waiting, not success. A wait action records a question and required input,
releases workers and resumes after an authorized reply. A failed task remains failed; a required
obligation can be satisfied by explicitly adopting a successful corrective task, never by relabeling
history. Settled optional failure does not prevent finish; optional uncertain/running work still
prevents it. Removing an obligation requires scoped edit authority and remains visible.

Defaults: 256 admitted tasks/workflow, 32 tasks/batch, 32 admitted model batches, 4 concurrent tasks,
3 consecutive rejected/no-progress action responses, 3 failures on the same corrective lineage,
60 minutes of active execution allowance, and 16 tool calls/task. No automatic retry follows ambiguous
entry. A new accepted task/output resets the no-progress streak, not cumulative budgets. These are
configurable bounded policies shown before start. The daemon/grant supplies tighter token, byte,
process and spending limits; absent external-call allowance refuses external entry. Children,
expansions, retries and restarts share the workflow account. Human limit increases are prospective
recorded commands and never erase spent allowance. Exhaustion pauses entry and requests bounded
cancellation, retaining uncertainty until an honest outcome is possible.

The prompt-01 review correction enforces this time allowance with the existing runtime account.
Final entry reserves an adapter-enforced maximum against prior elapsed time and simultaneous live
reservations; a descriptor estimate cannot establish that maximum. Each open attempt contributes
elapsed time at serialized runtime boundaries. Terminal proof releases unused reserved time;
uncertainty and cancellation acknowledgement do not. Refusing a bound larger than the remainder is
conservative: the daemon cannot promise that an adapter will finish sooner than its enforced bound.
An HTTP timeout bounds the client's attempt, not remote physical work; unresolved work remains held.

Running instructions/inputs cannot be revised. Stop requests target the exact attempt and display
separately from stopped. Replacement is new linked work; it cannot repeat unsafe effects until the
original settles or authorized reconciliation records evidence and permits that continuation.
Restart never treats a missing worker as proof of non-entry. Pending consumers are explicitly rebound
when a correction replaces their expected result.

## Methods and activation

Use a daemon-wide library, independent of project/workflow, with opaque method IDs, mutable names,
owner ACLs, save generation and canonical content digest. A save installs a complete validated
body: parameters, ordinary tasks/dependencies, named result exports and nested local method refs.
Consumers may use but not edit it. A queued use records ID and bindings, not a pinned body. Renames
preserve identity. No published-version hierarchy is needed.

At activation resolve the latest saved root and reachable nested templates in one consistent library
snapshot. Retain bodies, digests, origin identities and deterministic template-local-to-workflow ID
maps in an accepted expansion record. Atomically commit the complete flattened task batch, exported
result map, receipt and expanded marker. The accepted record may precede materialization; no child
is runnable until the final commit. Recovery uses that exact snapshot, never newer content or duplicate
insertion. An interruption before acceptance selects current content at the next attempt.
Acceptance changes the use from queued to expanding and freezes its bindings/snapshot. Rebind/remove
now refuses as already accepted. A stop during expansion records a stop flag; recovery materializes
the accepted batch with children stopped before entry, then releases unused reservations. A workflow
pause allows materialization but no child entry until resume. Holds survive until that commit.
Removal that wins before acceptance creates no snapshot or children. These rules also apply to races
with restart and finish; an accepted expansion is never silently abandoned or hidden by removal.

Nested methods expand recursively within that activation, not as later live references. This makes
one activation recoverable and inspectable; a separately queued top-level use still selects current
content at its own activation. Check cycles on save and activation; maximum depth 8 and 64 flattened
tasks/use within remaining workflow capacity. Bound traversal to 256 nested reference occurrences
and the combined canonical snapshot/export/import to 1 MiB, checked incrementally before loading or
flattening beyond the limit. Per-template/document limits also apply, including many empty nested
uses. Reject the whole expansion on bound/interface failure.
This consciously chooses a local snapshot over independent nested updates mid-expansion.

A use is a timeline group/origin marker. Exports resolve to child result refs; dependents wait for
export producers and declared completion dependencies. The group succeeds when its required copied
tasks succeed and otherwise shows failures/blocks. It has no child runtime, worker, budget or authority.
Future-copy edits validate exports and remaining dependencies like ordinary task references and do
not write the library. Origin records retain both selected template content and later local edits.

Saving incompatible required parameters is allowed and reports affected queued-use counts.
Activation blocks with expected/supplied diagnostics until an authorized actor rebinds, replaces the
use or repairs the template. There are no silent defaults for required inputs. Deletion is refused
while queued uses or saved nested refs need the method; deliberately remove/replace references first.
Accepted snapshots survive deletion as long as protected tasks need them.

Cross-daemon sharing is bounded export/import including nested templates. Import creates new local
IDs, rewrites internal refs and retains source daemon/method/digest provenance. Export requires read
permission for the entire bundle. Exclude secrets, grants and live connections; there is no automatic
update propagation, remote execution or central library service.

## Browser, discovery and connections

Build standalone Svelte + TypeScript with Vite and a locked npm dependency tree in `apps/web`.
There is no server-side workflow state or required backend-for-frontend. Rust public protocol owns
semantics; Rust tooling generates TypeScript DTOs/client codecs checked against shared wire fixtures.
UI state contains drafts, presentation preferences and caches.

The October 17 user review changes the original manual-name/manual-identity connection default.
Ordinary setup asks for an endpoint and scoped bearer credential. The authenticated session returns
the daemon's stable installation identity and shared display name. The client learns the identity on
first connection and pins it to that endpoint; a reconnect mismatch refuses ordinary work and requires
an explicit forget-and-reconnect decision. No old drafts, request recovery or cached results transfer
to the replacement. The identity identifies the daemon, not the browser or device. It remains
available in connection details and is carried on subsequent requests without manual entry.

First-use trust rests on the operator-supplied endpoint and the transport/authentication rules below.
A returned installation ID is not a certificate or an independent verification of server identity.
The original initial session already sent the credential before comparing the manually entered ID;
automating that comparison does not introduce a stronger cryptographic claim.

One durable daemon metadata record owns its display name and generation. Restricted operator
configuration seeds it only when absent; restart cannot overwrite an accepted rename. A narrow
daemon-administration command performs an authorized, generation-guarded rename with exact durable
request replay. GUI and CLI use the same path. Renaming changes no stable identities, workflow
history, grants or host relationships. Duplicate names are allowed and endpoints distinguish them.
Bounded observation invalidates connected clients' metadata so they refresh the shared name;
reconnect and fresh clients read it from the daemon. A local last-observed name is only an explicitly
cached display value. Browser-authored aliases cease to be a competing name owner.

Operators bootstrap the daemon and initial administrator using restricted local configuration.
Tokens stay in memory by default, never URLs/logs, and clear on disconnect. Reauthentication on a new
device is expected; a recovery file is not. Persist only non-secret endpoints, learned identity pins,
last-observed names and layout locally. Authorization changes invalidate streams and clear inaccessible
cached data. Key caches/actions by daemon identity and actor, not a global workflow ID. Browser
onboarding does not replace the separately authorized execution-host relationships below.

Implement exact configured CORS origins, authenticated fetch, bounded preflights and exposed paging/
stream headers. Fetch-based SSE carries bearer headers; reconnect uses scoped cursors and an expired
cursor triggers a new snapshot. Refuse wrong origins and cross-scope cursor reuse. Production phone
access uses operator-provided browser-trusted HTTPS termination to a configured private listener;
local development may use loopback HTTP. Never require certificate bypass or insecure LAN requests
from an HTTPS page. Test proxy stream flushing and the actual deployment path. No SSH transport or
GUI credential relay is needed. See the platform contracts for
[CORS](https://developer.mozilla.org/en-US/docs/Web/HTTP/Guides/CORS) and
[SSE](https://developer.mozilla.org/en-US/docs/Web/API/Server-sent_events/Using_server-sent_events),
and [Svelte's UI scope](https://svelte.dev/docs/svelte/overview).

The read plane lists authorized workflows across permitted scopes, tasks, results, methods,
capabilities and connection summaries without a known workflow ID. Use indexed case-insensitive
search of title/description and bounded retained text results, not embeddings. Pages default to 50,
maximum 200, with actor/daemon/query-scoped cursors, stable snapshot bounds and explicit truncation/
expiry. No hidden-record count leaks. Timeline lanes derive from dependencies and actual timestamps;
queued placement has no invented start. List and inspector expose the same accessible actions.

### Browser workspace interaction

The compact composer and navigation in 02c improved the ordinary create/add path but retained a
protocol-shaped advanced editor. The October 18 user recheck rejects that result as complicated and
bloated, names the input/dependency/output/path fields, and requests a canvas with task details on
the left. Hiding those fields under another disclosure or embedding them inside graph nodes would
retain the rejected interaction. The selected replacement is a workflow canvas and direct authoring,
owned by assigned [02d](../development/virtual-office/daemon-workflows-implementation/02d-canvas-and-direct-authoring.md).

Navigation selects the daemon and workflow. The main area keeps that workflow's tasks, source cards
and connections visible; a compact left panel edits or inspects the selected item. Selection does
not replace the canvas or move its camera. Instructions remain the only required writing field,
with a suggested editable title and a visible, deliberate executor choice. Add text/file context or
select a real result using connectors or the equivalent keyboard/touch action. IDs, aliases and
binding keys are generated. Output choices come from their real producer contract. Task creation
without connections remains useful; a canvas is not a mandatory graph-building exercise.

Source cards project accepted inline or supplied values and retained results. Draft sources remain
explicitly unsaved; layout and selection own no workflow semantics. Read projections must expose
bounded, authorized input/port/result metadata sufficient to reconstruct connections in a fresh
client, including queued supplied values and off-page references. Capability authoring metadata
comes from the configured adapter/profile, not frontend guesses about its name. It reveals no secret
or unrelated execution configuration, and never grants authority. A missing or changed contract
retains the draft and explains the unavailable choice.

Task-output wiring retains successful-result semantics by default. A plain-language deliberate
choice can instead select a settled outcome, including failure; optional absent output is a different
rule. The client includes the implied dependency, while order-only prerequisites select a visible
task without passing data. Correction adoption still redirects future task-output bindings and keeps
failed history. Shared-source edits show their future consumers; started inputs remain frozen.

The earlier validation required a producer output declaration, yet existing tasks could publish
artifacts without one. The narrow extension in 02d allows this: a new `TaskOutput` may reference either a declared
future output or an actual named output already retained in that producer's result. Shared admission
and runtime owners validate actual published evidence, permissions, media/bounds and the existing
success/optional rule. For an undeclared published source, use its retained media type and the
existing finite 2 MiB task-output allowance, together with current consumer input/authority bounds,
as the compatibility rule for initial selection and correction redirect. The original artifact's
observed length is not a maximum for a longer correction. Declared outputs keep their original
stricter contracts. An empty declaration map cannot bypass the new binding's replacement checks.
Caller assertions cannot manufacture an output. This does not add a declaration
retroactively, accept an arbitrary artifact ID, change a started input or bypass correction semantics.

Captured client files remain task inputs, separate from native filesystem access. File-tool tasks
select authorized project roots and explicit read/write modes through narrow daemon-owned metadata;
the owning Rust path/authority checks resolve and validate the claim. Ordinary context connections
need no project grant. No browser file selection or graph gesture grants host access or enables tools,
model continuation, writes or more execution budget.

New cards and wires remain bounded in-memory draft changes until explicit Start/Add/Apply. Accepted
ready tasks may start immediately, so submitting a card before its required connections exist is
incorrect. New workflows accept their first tasks together; an empty canvas is not a durable empty
workflow. The existing atomic batch accepts related changes with generated aliases and current
generation/version guards. The product reviews those changes visually and in plain language, without
a protocol staging form or required JSON preview. Exact pending requests survive navigation, conflicts
require review, and disconnect/authority loss disposes of inaccessible drafts.

Spatial positions are not timestamps or dependencies. Actual timing remains in details and a
secondary timeline. Canvas/list views share state and operations; focusable connections and a compact
narrow-screen panel preserve access without precise dragging. Presentation may use bounded local
layout preferences, not a new shared graph store. The strongest risk is replacing forms with an
equally specialist graph editor. Reopen the interaction design if an actual user cannot create useful
work without drawing, or connect a file/result without learning schema terms. Browser passes establish
behavior, not acceptance. 02d must remove the superseded product forms while retaining their useful
operations; programmatic DTOs and CLI commands remain available.

## Execution-host connections

Host connections live on the workflow daemon. Reuse peer HTTPS, expected identity, delegated grants
and operation IDs. B's administrator issues a finite inbound relationship credential for A through
B's public administration path. A's administrator imports endpoint, identity and credential through
A's public path. These are distinct authorities even for one person. Use existing restricted private
secret-file references: stage writes, commit an opaque handle, clean unreferenced staging files after
interruption. Never return secrets from reads. A can use B without B receiving authority to use A;
remove symmetric setup requirements that serve no directional authorization need.
Secret import retries reuse a staged identity derived from actor/command identity. Canonical receipt
matching binds the non-secret fields and a keyed fingerprint of secret bytes using a durable private
installation key. Same key/same secret replays; changed secret conflicts without allocating another
handle. Secret bytes never enter ordinary receipts. Lost one-time credential delivery requires explicit
rotation; replay returns the original non-secret acceptance, not secret read-back.

GUI/CLI support add/test/edit/disable/revoke/remove and shared catalog/health/operation inspection.
Separate inspect, tool-use and administration scopes. Seeing B through A gives no direct B login.
Changes use expected connection generation. Disable/revoke persists across restart and closes new
entry. Rotation creates a new generation; accepted operations retain their old identity/association
for query/cancel. Refuse removal while operations are unresolved. Unreachable is health, not proof
of failure or non-entry. Query/replay the exact remote operation after reconnect, never submit a fresh
unsafe command to make a timeline complete.
Expose remote selections as connection ID plus remote capability ID; names are labels. Namespace
them in A's public/model catalog so equal names from two hosts cannot collide. At task start freeze
the selected descriptor and connection generation. Revalidate stale/revoked offers before entry;
refresh cannot substitute another host silently. Returned files use scoped artifact transfer with
verified digests before local result binding. Svelte and model proposals select from that same catalog.

## Retention and restart

Default completed detail/results/optional-context retention is 30 days, configurable per daemon.
Protect the transitive references needed by unfinished tasks, accepted admission/expansion, unresolved
operations, current templates and exact command recovery before collection. Retain the existing durable
clock/rollback boundary. Collect only content both old enough and unprotected; if capacity remains
full, refuse new admission/upload rather than deleting protected state.

Defaults are a 1 GiB retained-history/artifact allowance and a separate 256 MiB/100,000-entry compact
receipt allowance, first exhausted wins. All are configurable within validated bounds; tighter host
budgets apply. Keep canonical exact requests and compact original command results for replay/conflict.
Large task artifacts are references, not copied into receipts; fetching expired content reports expiry.
Receipts and unresolved host state remain capacity-bounded for the store generation, with truthful
admission refusal when full. Rolling history does not promise unlimited lifetime writes. Do not reset
a store generation automatically, forget accepted keys, or retain a permanent full event archive.
Receipt storage can retain submitted command text beyond display-history expiry; report that as
protected command recovery content, not a claim that every copy of an input has been erased.

Admission reserves bounded space for the accepted work's remaining entry, result/uncertainty,
settlement, reference release and mandatory control records, using the task/tool/output ceilings.
Ordinary admission cannot spend this reserve. Stop, revoke and reconciliation use server-issued
action identities for the target generation: repeated requests replay their bounded action slot,
not another arbitrary receipt allocation. Reserve these slots with every admitted task/connection,
plus an administration slot for a capacity-increase command. Refuse new entry/resume if its complete
reservation cannot fit. Increasing capacity reserves the next administration slot prospectively.
Thus reaching the ordinary receipt ceiling still permits settlement, stop/revoke and authorized
resolution; it cannot strand accepted work. Exhaustion has a tested control path, not a hidden
unbounded emergency log. Interrupted records recover within the same reserved allowance.

Restart uses the single-owner recovery gate, redb transactions and host reports. Recover accepted
mutations/expansions before scheduling their tasks. Reconnect accepted attempts by retained identity;
a local process without proof of absence/completion becomes uncertain. Inspection stays available
while entry is blocked. Project Markdown/Git files are outside daemon collection and never deleted
by a retention sweep.

## Adoption, alternatives and costs

Select a new explicitly named development store and preserved old code for old stores. Inspection at
`750ebb26607c910ae96c6f6b0f0b3ea12228180c` found no running Milkdrift daemon, no stores in the checked
conventional user/service locations and no repository redb outside ignored `target`; 197 prior test/
evidence stores remain there. This is not a remote-machine or all-disks inventory. Nothing was opened
for mutation or reset. Prompt 01 rechecks only adoption facts. Unexpected real active state requires
a narrow preservation decision, not a speculative migration engine. New binaries refuse old workflow
stores before writes. No automatic upgrade/removal of old hosts or external resources is authorized.

Prompt 03 uses task document schema 3 and redb internal document format 22, retaining physical
schema 19. Earlier task documents lack model admission linkage, frozen tool policy and authoring
generations; the preceding review also added duration reservations and accounting watermarks.
Readers refuse earlier formats without mutation rather than infer missing historical facts.
Use a fresh development root for this revision and retain the old binary with its existing stores.

The 02a name correction advances physical schema to 20 while keeping internal format 22. Its
atomic adoption accepts only the exact preceding 19/22 store, adds metadata and rename receipt
tables, and preserves workflow and receipt bytes. It cannot infer facts missing from older formats.
Names seed only when absent. Adding `RenameDaemon` to Controller changes that preset's grant digest;
frozen workflow authority is not rewritten to match it. Operators must finish old work using its
preserved executable/configuration or accept prospective authorization refusal under the new grant.
The [daemon guide](../operations/daemon.md) explains the practical upgrade sequence.

The first cut costs more than a GUI wrapper: ordinary workflow representation and obsolete consumers
must change together. Connect Svelte to a useful real task before completing that cleanup, retain the
production UI, and hand off one runtime/path. Retain host/Podman mechanisms and reprove affected safety.

Keeping the old graph saves initial edits but imports unrelated routing/revision/publication concepts
and still fails local activation. A blank-slate rewrite discards useful effect/uncertainty evidence.
Definition-time pins miss owner edits; remote method services change ownership. Permanent rich history
defeats rolling retention; deleting all receipts permits duplicate effects. These costs explain the
choice, not a claim of measured improvement. Reopen a choice only for a concrete journey failure,
real preservation obligation, measured capacity issue or user-review finding, not another broad audit.
