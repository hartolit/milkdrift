# Domain crates

Portable domain building blocks.

`domain-contracts` is the sole F0 foundation and has no workspace-local production
dependencies. A type belongs in F0 only when it crosses the backend/runtime
boundary or has at least two stable, distinct domain consumers. Single-feature
vocabulary stays with its owner; in particular, `TaskId` belongs to `task-graph`.

`tokenization`, `context-planner`, `sampling`, and `task-graph` are F1 algorithm
crates. The current exact reviewed domain production DAG is:

```text
tokenization    -> domain-contracts
context-planner -> domain-contracts
sampling        -> domain-contracts
task-graph      -> domain-contracts
```

There is no current F1-to-F1 production edge, but F1 peers are not categorically
forbidden. Every domain-to-domain production edge, including a future peer edge,
must be registered exactly with a reviewed rationale, and the complete registered
graph must remain acyclic. Unreviewed domain peers fail closed.

Domain crates are always `no_std`. Host-process facilities, vendor SDKs, filesystem
I/O, network access, databases, and OS synchronization belong outside this layer.
