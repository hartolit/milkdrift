# milkdrift-blueprint

This crate defines ordinary task workflows. A task names an external capability, instructions,
explicit inputs, dependencies, output promises, and optional native filesystem access. It contains
no scheduler and grants no authority. [ADR 0051](../../docs/decisions/0051-daemon-task-workflows.md)
owns the task-workflow replacement.

`TaskDefinition::validate` checks one bounded definition. `validate_task_set` also checks that
producer references and corrective origins exist and that dependencies and correction lineages
cannot cycle. Inline text, captured UTF-8 files, supplied values, immutable task outputs and settled
outcomes have explicit variants. Missing required inputs keep a task queued; optional absence is a
value only after the producer settles. Captured files retain their exact bytes, digest and upload
origin rather than reopening the source path when work starts. Each instruction or supplied text,
captured file or JSON value is limited to 16 KiB. Before admission, the combined instructions and
known input content must fit 24 KiB after JSON escaping; oversized submissions and edits return a
refusal that asks the caller to shorten or split the task.

`WorkflowCreate` accepts client-local task aliases. Runtime allocates durable identities and returns
the complete alias mapping in the accepted receipt. `TaskBatch` changes future work under the exact
workflow generation. Revisions and removal require the observed task version; started definitions
remain immutable. A correction names the task it addresses, and adoption separately identifies any
unstarted consumer bindings to redirect. Original results remain evidence.

`WorkflowLimits` bounds cumulative admission, active execution and corrective work. Runtime and
control enforce those limits against durable facts and current scoped grants. Clients may present
these types but must send changes through the daemon's shared command path.

The [task contract tests](tests/tasks.rs) exercise invalid references, cycles, captured-byte integrity
and output declarations. Runtime tests cover admission races, execution and persistence.
