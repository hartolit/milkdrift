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

Before scheduler polling, the daemon calls `process_model_admissions` on a bounded workflow page.
Control reads the exact retained model response with the workflow's scoped authority and parses only
its structured batch. It never promotes result text to commands. Invalid/conflicted proposals retain
a diagnostic; an explicitly permitted feedback task receives the response and a fresh snapshot.
Out-of-grant proposals wait for a person. Every feedback task uses the same capability, tools and
cumulative account, and its exact source key prevents duplicate retries after restart.
Feedback retains every original frozen input and adds the response under an available named input,
identified in its instructions. A full input allowance refuses feedback instead of replacing a value.
An explicit wait remains in force when a response is rejected or a rejection is dismissed.
An unrelated explicit wait pauses automatic feedback until continuation. An accepted repair can
clear only the diagnostic wait owned by its source response. Temporary storage failures preserve
feedback eligibility; exhausted persistence capacity leaves a retained refusal for a person to resolve.

Authority presets expand to ordinary operation sets. Resource selectors, ceilings, expiry and
revocation remain explicit grant facts; choosing a preset does not enable a hidden controller.
The [authority tests](tests/task_authority.rs) demonstrate creation refusal when start scope is absent
and read isolation across workflow scopes.
