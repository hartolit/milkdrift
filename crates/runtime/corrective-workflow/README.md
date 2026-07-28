# corrective-workflow

Stateful corrective workflow orchestration over the portable `task-graph`
contracts.

The crate owns the canonical draft → validate → normalize → review → revise →
validate flow, immutable bounded artifacts, attempt identities, retry accounting,
diagnostic normalization, and workflow events.

Model and validator work enter through coarse statically dispatched ports. The
crate does not own model tensors, token scheduling, provider transports, UI state,
or the application lifecycle. A caller may eventually satisfy a model task with
local generation, a peer node, or an external model service without changing the
workflow's graph semantics.

`application-runtime` may coordinate this engine but does not contain it.
