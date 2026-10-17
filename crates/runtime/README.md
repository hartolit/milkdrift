# milkdrift-runtime

`TaskRuntime` owns the ordinary task state machine. The daemon supplies its durable `WorkflowStore`,
capability executor, authority evaluator, clock, and identity generator. Runtime admits task batches,
freezes starts, accepts observations and preserves uncertainty. It has no inference engine and owns
no worker threads.

A command checks its exact actor-local request identity before applying a generation-guarded change.
Storage commits the workflow document, discovery fields, resource reservations and exact response in
one transaction. Changed reuse conflicts; retrying the original request recovers its accepted result.
`create_exact` and `edit_exact` additionally preserve the canonical public envelope before transport
mapping, so omitted defaults and different upload identities remain distinguishable.

`tick` discovers bounded pages and freezes eligible tasks' definitions, named input bytes, capability
generation and authority lineage. A required missing input keeps work queued. Optional producer
outputs become absent only after settlement. Resolved dynamic outcomes are checked against the
same bounded content contract before a start is committed; an oversized immutable outcome fails
without entering an external capability. The worker calls `execute` outside the scheduler. Local
preparation precedes a fresh authority check and durable external-entry intent. Enforceable maximum
resource use is charged cumulatively before entry, including nested calls; restart and task removal
do not replenish the account. Native and managed resource owners enforce their additional guards. A generation-capacity refusal
that proves non-entry leaves the exact accepted start queued for another attempt to acquire a permit.
Its frozen request and invocation identity remain unchanged and no execution allowance is charged;
a refusal that cannot establish exclusive ownership remains uncertain.

`ExecutionReporter` accepts sequenced append-only observations. Progress, output references and
terminal results have independent bounds, with capacity reserved for terminal settlement and stop
receipts before admission. A successful provider terminal must satisfy the task's declared required
outputs. A correction can satisfy a failed obligation only through explicit adoption; it never changes
the original result or the bindings of an already started consumer.

A persisted entry with no terminal proof becomes uncertain after restart. It is never automatically
entered again. An accepted unentered start may resume with its exact frozen inputs. Cancellation
acknowledgement alone does not establish stop. Proven terminal settlement permits the executor to
release native claims; recovery repeats that release idempotently if the daemon stopped between the
durable settlement and the release callback.

`begin_shutdown` closes new admission and scheduling while preserving exact receipt replay, stop
requests and settlement. The host separately chooses whether accepted workers drain or remain for
recovery. Public callers use [`TaskControlService`](../control/README.md), which rechecks scoped
authority for mutations and reads.

Opted-in model tasks publish an immutable `model_response`. Accepting its output report records
pending action admission in the same transaction, so a restart cannot detach that accepted response
from its proposal. Control admits actions only after successful task settlement; an interrupted
entered source remains uncertain with its pending response inspectable.
Control reads only its structured `TaskBatch`, checks current authority, and calls the ordinary batch
transaction. Source linkage, allocated IDs and receipt commit together. Exact replay cannot add the
children again. The authoring generation changes with edits and starts; the source task's own progress
reports do not invalidate its snapshot. A competing edit/start requires a refreshed proposal.

Rejected responses remain inspectable. Explicit automatic-repair policy may admit one ordinary
feedback task for that source response; otherwise entry waits for a human decision. Model batches,
rejection streaks and failed corrective lineages are bounded. `SetLimits` changes future ceilings;
`Continue` never refunds prior tasks, model batches, elapsed time or unresolved external work.
An accepted proposal that adds future tasks resets the rejection streak; automatic feedback does
not. Failed lineage counts remain in history, but only a lineage with an outstanding obligation or
open work pauses execution. A settled optional failure alone does not prevent finish. Reaching the
model-batch ceiling still permits already admitted tasks to run; the next proposal is refused.
An explicit wait or undecided response prevents completion even when all task results have settled.
The workflow records which model rejection owns a diagnostic wait. Repair or dismissal clears only
that source's wait; it cannot replace or release an explicit pause accepted during model execution.

The [durable runtime tests](tests/durable_runtime/tasks.rs) use production redb storage and controlled
external boundaries. They cover exact request recovery, edit/start races, input freezing, cumulative
allowances, revocation, correction, uncertainty and shutdown. Follow the repository's
[verification policy](../../docs/development/workflow.md#choose-verification-for-the-change) for the
checks required by a change.
