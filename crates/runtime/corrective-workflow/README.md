# corrective-workflow

Stateful corrective workflow orchestration over the portable `task-graph`
contracts.

The crate owns the canonical six-stage flow:

1. draft
2. initial validation
3. diagnostic normalization
4. review
5. revision
6. final validation

A rejected validation verdict remains a successful task result. The workflow
continues after initial rejection, and the final verdict selects an accepted or
rejected `WorkflowOutcome`. Successful artifact commits and workflow events keep
the same stage order as the graph above.

## Executor-owned bounded output

Model and validator ports receive executor-owned output sinks instead of
returning fully allocated payloads:

```rust,ignore
fn execute_model_task(
    &mut self,
    request: ModelTaskRequest<'_>,
    artifacts: &ArtifactInputs<'_>,
    output: &mut BoundedTextSink,
) -> Result<(), Self::Error>;

fn execute_validation_task(
    &mut self,
    request: ValidationTaskRequest<'_>,
    artifacts: &ArtifactInputs<'_>,
    output: &mut BoundedDiagnosticsSink,
) -> Result<ValidationVerdict, Self::Error>;
```

`BoundedTextSink::append` supports chunked generation. Every append checks the
complete post-append UTF-8 byte count and uses fallible reservation before
changing accepted text. Overflow and allocation failures are atomic, sticky,
and never truncate output.

`BoundedDiagnosticsSink::append` copies each `RawDiagnostic` into executor-owned
storage only after checking its complete structured size and reserving vector
and string storage fallibly. Validation accounting uses one byte for the
verdict, one byte each for severity and option tags, UTF-8 string payload bytes,
and four payload bytes for each present `u32`. Each diagnostic therefore has a
nonzero structural cost, so diagnostic count and string growth share the same
`TaskOutputContract::maximum_bytes` bound. The internal normalization stage uses
the same accounting and bounded, fallible construction.

A port should stop when an append returns `OutputSinkError`. Even if it maps or
ignores that error and then returns its own operational error, the executor
checks the sink first and returns the corresponding non-retryable
`WorkflowError`. Sink failures are never retried as port failures.

Task token budgets remain in `ModelTaskRequest` and `ValidationTaskRequest` for
the concrete port to enforce. Artifact byte limits remain independently owned
and enforced by the output sinks.

## Artifact lifecycle

Root specification artifacts are shared and return `None` from
`Artifact::owner`. Every generated artifact returns `Some(WorkflowId)` for the
workflow that produced it.

A successful workflow retains all six generated artifacts until the caller
releases them:

```rust,ignore
let removed = executor.release_workflow(outcome.workflow());
```

`release_workflow` returns the number of generated artifacts removed. It
preserves specifications, artifacts from other workflows, and already queued
successful-workflow events.

If execution fails after allocating a `WorkflowId`, the executor automatically
removes that workflow's generated artifacts and queued events. Identifiers
allocated by a failed or released workflow are never reused.

## Scope

The crate owns immutable bounded artifacts, workflow ownership, attempt
identities, retry accounting, deterministic diagnostic normalization, and
identity-only workflow events. It does not own model tensors, token scheduling,
provider transports, UI state, or the application lifecycle. A caller may
satisfy a model task with local generation, a peer node, or an external model
service without changing graph semantics.

`application-runtime` may coordinate this engine but does not contain it.
