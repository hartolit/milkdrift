# Execution, federation and the browser client

**Proposal:** retain the daemon owners that already distinguish a workflow, an accepted operation
and a surviving managed installation. Complete the public inspection routes and build the Svelte
client against them. Start with explicitly configured cross-origin bearer fetch; remote deployment
uses an operator HTTPS proxy to the daemon's loopback listener. This is A1/r3, selected for planning
under G1/r1 and P1/r2 after joint criticism and final source correction. It awaits user decision
and implementation evidence. It does not
claim that the current daemon works from an independently hosted browser: the actual E04 probe
failed its authenticated cross-origin reads and stream request.

## What connecting and running mean

A connection gives the client an authenticated view of one host. Its address and local label are
not its durable identity, and knowing two hosts does not connect them as peers or combine their
grants. The client checks protocol, stable host identity, actor and exact grant before exposing
cached data or submitting a saved command. Two hosts can both contain `release`; every reference
keeps its real owner and exact revision/generation. One failed connection leaves the others usable.

Definitions retain immutable ancestry. Two valid drafts from the same base may save as sibling
revisions; the UI must show and let the user select them explicitly. The current authoring guard
checks the submitted envelope against that draft's exact base, not a mutable latest-workflow head.
An actual mismatch refuses without discarding the draft. The proposal adds no hidden overwrite,
automatic rebase or new head-locking API.

| Action | What owns the accepted work | What the user can expect |
| --- | --- | --- |
| Invoke a process/model operation directly on B | B's serving record, exact capability generation and adapter entry evidence | No workflow is invented. Inspect public invocation/result, cancel when authorized, recover the exact request after a lost reply. |
| Run a workflow at A and delegate a task to B | A owns the workflow history/account; B owns the accepted operation/effect evidence | A broken connection does not transfer ownership or authorize a new target. Results cross the selected public boundary. |
| Use an exact reusable child | The current workflow owner resolves a saved local revision and records a real child with inherited authority/constraints | Later edits do not change an already accepted child. A revision merely readable on another host is not automatically pinnable here. |
| Make an independent copy | The accepting owner creates a new workflow identity with provenance | No run state or authority is copied. An identity-bound governing agreement can refuse copying. |
| Call a published service | Public invocation plus exact internal association; internal execution uses configured service authority | Invoke-only callers receive declared outputs without internal graph access. Retirement refuses new calls while preserving accepted ones. |
| Manage an installation | Its managed owner retains intent, generation, editing claims and lifetime holds | Installation life can exceed a call. Cancellation or a terminal run does not prove a physical writer stopped. |

Shared discovery may show these useful choices together, but the committing action must explain
which of them it will perform. A generic “Use” action cannot promise identical editing/control.
Service internals remain opaque unless separate current permissions allow actual inspection.
There is no assumed direct relay through an execution-only intermediary, transparent workflow
migration or release of a running service from management. Those outcomes require explicit later
decisions. Preserving data during removal is not release-management.

## Progress and recovery across the boundaries

Consider A's workflow calling a service at B whose child edits a protected area. A reserves the
task allowance and B accepts an exact request/generation. B records the planned child association
before starting it. If an acknowledgment is lost, recovery inspects or replays that exact request;
it does not create a new child or reset the allowance. A waiting publication wrapper releases its
worker so its child can run while retaining method/resource lifetime protection. Editing moves
only to the exact accepted child and only with the necessary quiescence evidence. Conflicting
maintenance refuses; unrelated work can continue.

If the child stops with durable evidence, editing returns and later work can progress. If stop
proof is lost, cancellation and restart retain uncertainty and holds. An authorized operator can
inspect and resolve the exact ownership condition; the system must not invent a successful or
failed effect merely to clear “busy.” Grant revocation can block future entry/disclosure without
rewriting already accepted history. Known usage settles once; unknown usage keeps its reservation.
This composition has source and focused test support, including legitimate child progress and
retained holds, but the complete multi-owner browser journey remains an implementation gate.

Continuous agent work uses the same scoped commands. Failed verification stays in history;
prospective repair changes future work under the governing constraints. Children inherit their
account and restrictions, while service calls use explicit bounded allowances. Waiting for a human
and reconnecting after interruption must recover the same run. A new request label is not a way to
reset inherited obligations. The broader research/critic/coordinator/lesson process still needs its
own proved routes; these mechanisms alone do not establish the full S28 scenario.

Closing a tab or aborting a fetch stops observation, not accepted work. Logging out clears the
active token and authorized view caches. Forgetting a connection changes client configuration.
Revocation changes rights. Cancellation is a separate server command whose receipt is not proof
of physical stop. The frontend must explain each result and preserve explicit recovery records
without presenting them as an execution ledger.

## Browser delivery and local data

The preferred initial deployment is an operator/user-hosted static Svelte application with an
approved daemon endpoint set, per-connection bearer fetch and bounded fetch-based SSE. CORS stays
disabled unless the daemon operator configures exact allowed
origins; preflight handles transport only and every actual request still authenticates normally.
Remote connections use trusted HTTPS termination in an operator proxy. The daemon remains bound
to loopback. Redirects are refused, cookies omitted and tokens kept out of URLs/logs. Native
EventSource cannot supply the required arbitrary Authorization header, so it is not the client
transport.

The static host also configures the browser's CSP connection policy. Adding an endpoint outside
its approved set requires a hosting-policy change, which the interface must explain. Arbitrary
endpoints from a generic public-hosted app remain unqualified. Origin/TLS trust governs initial
bearer delivery because version/authority discovery itself requires authentication; the subsequent
stable-host check prevents command/cache rebinding, not disclosure of the bearer to a replaced
already-trusted origin.

Tokens live only in memory; reload requires authentication again. Nonsecret connection metadata
can persist. Bounded private drafts and exact pending requests may persist in IndexedDB under an
explicit personal-profile choice; session/export mode keeps no private record unless deliberately
exported. Unknown submissions cannot disappear through ordinary cache eviction.
If durable local recovery cannot be retained, a deliberate exact export or refusal must precede
new effectful submission. A bounded part of the recovery capacity is reserved for control requests;
if even that cannot retain cancellation, offer exact export or an authorized native-client stop
route without silently discarding pending work or sending unrecorded effects. Browser profile
access or a same-origin script compromise can expose
retained content; omission of bearer persistence reduces that exposure but does not eliminate it.

On replacement host or changed authority, the client clears authorized caches/cursors and keeps
unfinished private work bound to its original identity for explicit recovery, export or discard.
It must not silently send that draft elsewhere or reveal the previous actor's results. Known run
inspection under new authority is separate from replaying a workflow command under its original
grant. Direct serving may permit explicit retained-request recovery under current Inspect rights;
the server compares its original accepted basis. Missing or denied reads do not prove no effect.
Independent
observation streams keep their own ordering/freshness; there is no global clock or permission union.
Capability snapshots replace the complete authorized view, including empty snapshots, and process
restart can require resynchronization. Reconnecting a feed never automatically retries a mutation.

The strongest smaller alternative is an operator-controlled same-origin proxy serving the static
client and forwarding to a fixed daemon. This avoids CORS but constrains independent hosting; an
arbitrary multi-target gateway adds target allowlisting, SSRF and possible credential-custody work.
A broker or native wrapper needs a concrete unmet requirement. Neither is a hidden prerequisite
or an assumed cure for browser restrictions.

## Work required before implementation can claim this outcome

- **R-A01:** opt-in exact-origin transport/config and actual browser-to-daemon qualification,
  including the supported HTTPS-proxy topology, authenticated streams and refusal paths.
- **R-A02:** a per-connection frontend identity/cache/exact-recovery adapter using the existing
  public authority/protocol contracts. No replacement identity API is needed.
- **R-A03:** bounded authorized managed-installation and approved-recipe discovery. Existing
  inspect/action routes work only when the client already knows the identifier.
- **R-A04:** safe forward navigation from a service invocation to its real internal run/definition
  only for independently authorized readers. Invoke-only behavior must stay opaque.
- **R-A05:** a focused mixed-publication, narrow-grant list probe before choosing a pagination
  remedy. Source raises a progress concern; it is not an established failing behavior.
- **R-A06/R-A07:** correct misleading pin/copy/role/relay claims; decide whether release-management
  is required before adding a new managed lifecycle operation.

Richer structured authoring, same-scope exclusive merge and learning discovery have their own
03/05/06 owners. A child workflow cannot invisibly stand in for a merge because it adds identity,
context and authority boundaries. A shared diagram does not justify a uniform backend ledger.

The implementation gate must exercise a real static build against two daemons: negotiation,
scoped reads, a controlled mutation, exact lost-reply recovery after reload, bounded SSE resync,
changed host/grant, forbidden origin and continued use of the second connection during first-owner
failure. Current actual native journeys establish useful ordinary/direct/delegated/publication
behavior. The browser probe establishes a missing route. Neither substitutes for that future gate.

Prefer additions that do not change durable execution facts. Any migration must retain exact
requests, uncertain effects, artifacts, publication associations and managed holds. Rollback is
safe only when the older readers accept all written facts and still preserve replay; restoring an
old database while an external effect survives can duplicate work. Role removal cannot abandon
active obligations. Frontend rollback must preserve or safely export newer unresolved local records.

The choice should reverse if browser deployment tests favor a same-origin route for the intended
users, if human use repeatedly misinterprets common discovery, or if a fully traced structural
alternative materially improves progress and recovery. Those changes reopen their dependent
product, notation and implementation decisions. The
[detailed architecture proposal](../working/architecture-and-federation.md) owns the source-owner
inventory and compound traces; the [observations](../working/observations.md) state exactly what
was executed, and the [whiteboard decision](../../whiteboard/discussions/architecture/owners-and-browser.md)
records current authorship, alternatives and unresolved objections.
