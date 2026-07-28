# Domain crates

Portable domain building blocks.

`domain-contracts` is the sole shared F0 leaf and has no workspace-local
dependencies. `tokenization`, `context-planner`, `sampling`, and `task-graph` are
F1 crates: they may depend on `domain-contracts`, but never on one another.

Domain crates are always `no_std`. Host-process facilities, vendor SDKs, filesystem
I/O, network access, databases, and OS synchronization belong outside this layer.
