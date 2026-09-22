# Daemon control and execution authority

Configure authority by asking what this actor needs to do and which resources that work can touch.
The credential selects an actor; its grant must cover both the command and later capability entry.
For example, permission to start a workflow does not by itself permit its process to execute or
let the caller download restricted output.

Daemon configuration schema 11 requires every actor binding to contain an explicit `authority`
table. Preset names deterministically expand to typed operation sets; they do not imply resource
access and are not retained as executable session policy. The resource scope, numeric ceilings,
validity interval, grant identity/revision, and revocation generation are independent inputs to the
immutable schema-4 grant. Authentication selects that exact actor and grant but grants nothing by
itself.

## Choose operations and resources

Start with the maintained [operator configuration](../../examples/operator/daemon.toml) and
[process/model setup](../../examples/operator/README.md). Its authority table is complete;
select the exact workflow, capability, operation, profile, trust, and resource facts required by
the intended operation. Test fixtures are compatibility evidence, not operator configuration.
Capability authority is either `{ "type": "deny_all" }` or an explicit conjunctive allow scope.
Every allow dimension is `{ "type": "any" }` or `{ "type": "only", "values": [...] }`.
`Only` requires 1..=128 ordered unique values; an empty array is invalid and never means wildcard.
The side-effect ceiling applies in addition to every selector.

### Filesystem, network, and secrets

Filesystem roots use one canonical
durable grammar: Unix roots are `/` or begin with `/`, and ordinary Windows drive roots are `C:/`
or begin with an uppercase ASCII drive plus `:/`. Both forms use `/` separators and compare exact
components rather than string prefixes. A Unix root never contains a Windows root, different
Windows drives never contain one another, and component case is exact. Host adapters first
canonicalize native paths and then convert them to this grammar; exact case intentionally fails
closed rather than approximating Windows Unicode case rules. Relative paths, traversal, repeated
or trailing separators, backslashes or mixed separators, drive-relative paths, UNC/device paths,
alternate-data-stream syntax, reserved Windows device names/characters, trailing-dot/space aliases,
control characters, and non-UTF-8 paths are refused. Broad authority is explicit as Unix `/` or a
named Windows drive root such as `C:/`; `/` is not a cross-platform wildcard. Network destinations
are credential-free `host:port` values and network profiles are named immutable transport profiles.
Secret references are opaque names; secret values never belong in the document.

### Match the complete task requirement

Revision admission checks the complete capability requirement envelope. Unspecified requirement
dimensions mean `Any`, so a narrower grant refuses the revision even when an exact capability is
named. Set task `placement.localities` and/or `placement.peers` to prove those restrictions. A
nonempty exact peer set also proves `peer` locality. Admission accepts only restrictions contained
in the run's grant; dispatch and final entry still check the actual descriptor and current authority.
An empty placement set admits no candidate. It never enables a fallback.

Locality and peer restrictions intersect exact capability, profile, operation, trust-zone, execution
trust, and effect requirements. They do not infer any of those dimensions from a catalog or grant.
For example, a grant narrowed to a provider profile still requires that profile in the task. A
locality-only requirement leaves the peer dimension unrestricted and cannot prove a grant narrowed
to exact peers. See the [two-host authoring example](peers.md#pin-tasks-to-approved-hosts).

## Grant inspection separately

Artifact authority is either `{ "type": "deny_all" }` or an allow scope containing an explicit
`Any`/nonempty `Only` identity selector and a nonempty sensitivity set; there is no implicit empty
identity wildcard. Artifact operations use this scope independently of workflow/run scope: an
`Any` artifact selector is not restricted to artifacts produced by the grant's selected workflow.
Layout authority is either deny-all or a shared-layout scope with an explicit
revision selector. Actor-owned/private layouts are reserved in the authority vocabulary but are
not implemented by the control protocol, daemon, or persistence adapter. Empty peer and workspace
scopes deny access unless their explicit wildcard boolean is set. Daemon flags independently grant coarse
readiness, detailed health, the caller's own authority view, redacted configuration, and bounded
audit views. Protected artifact/provider/peer/health details are therefore not implied by workflow
inspection.

## Set bounds and validate

### Budget scope

These limits answer different questions. `--print-effective-config` labels their scopes;
raising one does not raise the others.

| Owner | Meaning |
| --- | --- |
| Grant `AuthorityBudget` | Permission ceiling for one command/request. It does not record lifetime spending. |
| Adapter profile and request | Limits on one attempt, such as time, bytes, concurrency, or requested model output. Provider observations and estimates do not become hard guarantees. |
| Runtime workers and queues | Capacity for concurrent work and pending commands. Slots become reusable; cumulative usage does not. |
| Controller account | One cumulative allowance shared by the controller and every descendant, across retries, revisions, and restarts. Committed equals settled plus outstanding reservations. |
| Storage retention | Hot operational detail, archival batches, and retained bytes; independent of permission and cumulative admissions. |

A billed model profile declares its largest permitted call in the per-request monetary permission
check, rounded upward to whole hundredths of its currency. This conservative check happens before
request preparation; the cumulative account then reserves the smaller bound for the exact prepared
request. An explicitly unbilled profile needs no monetary permission amount.

`run show` reports `controller_accounting`; `controller status` includes the same account as
`accounting`. An ordinary run reports `state: inactive`. An active account exposes its exact
identity, declaration origin, policy digest, revision/digest, currency, reservations, settled use,
outstanding obligations, committed totals, and remaining allowance. Cost is in millionths of the
declared currency, artifacts in logical bytes, process/model admissions in entry counts, and
input/output in logical model tokens, including generated reasoning. An envelope must explicitly
declare that unit; byte counts and unspecified provider units cannot enter the token account.
Cache re-evaluation and model calls inside an agent process are not direct input/output tokens.
A bound that cannot be expressed conservatively
is refused before entry; unsupported currencies/units are not converted.

A controller may explicitly use no currency with a zero monetary allowance for unbilled work.
This refuses all currency-bearing requests; unknown charge is still refused. Endpoint billing is
an operator declaration, independent of location or authentication. Direct model tasks reserve
input/output tokens, artifacts and model admissions using the frozen prepared contract. Local
coding agents reserve process admissions and artifacts and have launch deadlines; their internal
API calls do not become metered direct model calls. See the
[supported local contract](../guides/local-model-endpoint.md#controlled-local-text-requests).

Do not add outstanding reservations to committed again. Missing terminal unit/cost observations
retain their reserved remainder and block further admission. An unresolved external effect keeps
its outstanding obligations reserved. `remaining: null` means allowance
cannot safely be offered while blocked, even if a subtraction would be positive. Inspect the block
reason and exact attempt evidence; missing cost or units are not zero. Reading a descendant's
shared totals also requires permission to inspect the originating controller run.

### Grant ceilings

Every numeric ceiling must be present for a safe grant, including provider-neutral `units`. Use a
finite `valid_until`, declare the strongest side effect the actor may cause, and grant only the
filesystem, network, secret, locality, trust zone, and peer facts required by registered adapters.
The daemon validates these facts before it opens storage.

`trusted_host_process` authorizes code that runs with the daemon account's host privileges. The
adapter mediates argv, environment, selected materialization, and declared output import, but it is
not a filesystem or network sandbox. Grant this class only to explicitly byte-pinned process
generations. `sandboxed_process` is a distinct exact class; granting or requiring it never permits
the current local-process adapter.

Wildcard workflow, capability target/operation, artifact, peer, or workspace scope; unknown side
effects; infinite validity; or missing ceilings are rejected unless
`dangerous_allow_broad_authority` is `true`. That flag is a deliberate acknowledgement, not a
shortcut for generating hidden wildcard facts. Broad grants remain limited by the resources
written in the configuration.

Older configuration and authority-grant schemas are rejected. JSON configuration has no fallback
reader. Migration is manual: start from a reviewed schema-11 TOML configuration, replace every
legacy capability and artifact array with an
explicit `Any` or nonempty `Only` selector, retain `DenyAll` where no invocation or presentation
access is intended, choose finite limits,
and explicitly configure each peer relationship's `artifact_sensitivities`. Run
`milkdrift-daemon --config PATH --check-config`, then inspect
`--print-effective-config` before starting the daemon. The effective output is normalized TOML,
redacts secret-source details, and is independent of source comments and formatting.

## Changing authority

Use `milkdrift daemon authority` to confirm the actor and grant selected by the credential. When
a start is refused, compare the whole reachable task requirement with the grant; when later entry
is refused, also compare the exact adapter's filesystem, network, secret, and resource needs.

Advance `grant_revision` whenever changing a grant's content; an identity/revision pair binds one
immutable grant. For revocation, advance the revocation generation or disable the actor and restart.
Existing page and reconnect
cursors then fail closed; open streams stop future disclosure on their next bounded check;
already-entered external work keeps its truthful terminal history.

Replacing a referenced credential file changes the value resolved on subsequent requests. Changing
a variable in a client shell does not change the environment of an already running daemon. Client
credential objects and old cursor MACs also do not update themselves: reconnect with the new value
and obtain fresh continuations when the server refuses the old ones.
