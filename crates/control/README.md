# milkdrift-control

`TaskControlService` is the shared scoped command boundary for human, service and model-proposed task
changes. It receives a trusted `ActorAuthorityContext`, evaluates current authority, and delegates
semantic admission to `TaskRuntime`. The daemon, CLI and Svelte client all use this path.

Creating a workflow requires creation and start scope for its daemon-derived identity. Editing future
definitions requires edit scope; a batch containing a stop request separately requires cancellation
scope. Read authorization is checked again against the current grant and clock, including receipt
recovery. A retained receipt is evidence of past acceptance, not permanent permission to disclose its
contents.

`create_exact` and `edit_exact` bind acceptance to canonical public request bytes while passing typed
definitions to runtime. Runtime owns alias allocation, immutable starts, prospective edits, output
contracts, cumulative accounts and atomic persistence. Control never schedules capability work or
interprets client presentation state.

Authority presets expand to ordinary operation sets. Resource selectors, ceilings, expiry and
revocation remain explicit grant facts; choosing a preset does not enable a hidden controller.
The [authority tests](tests/task_authority.rs) demonstrate creation refusal when start scope is absent
and read isolation across workflow scopes.
