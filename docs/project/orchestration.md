# Corrective workflow orchestration

## Scope

The typed corrective workflow is an independently stateful capability engine in
`crates/runtime/corrective-workflow`:

```text
draft
→ compile or validate
→ normalize diagnostics
→ review
→ revise
→ validate again
```

`application-runtime` may coordinate this engine, but workflow artifacts, retries,
validation state, and workflow events do not share E1's application lifecycle.
Extracting that ownership keeps E1 as the application façade without forcing every
stateful subsystem into the same crate.

## Pure graph boundary

`task-graph` remains an F1, `no_std`, allocation-free domain crate. It owns:

- semantic artifact roles;
- workflow-input, task-input, and task-output declarations;
- artifact provenance validation;
- direct producer-to-consumer dependency validation;
- task retry accounting;
- attempt tokens that reject stale completion.

The graph stores no transcript or artifact payload. It validates complete
`ArtifactReference` values containing an `ArtifactId`, physical `ArtifactKind`, and
semantic `ArtifactRole`.

Every graph task declares exactly one output. A produced artifact can be consumed
only when its producer is a direct prerequisite of the consumer. Workflow inputs
are external immutable roots and cannot also be task outputs.

## Capability-engine boundary

`CorrectiveWorkflowExecutor<M, V>` owns:

- checked workflow, task, and artifact identity allocation;
- one fixed-capacity immutable `ArtifactStore`;
- one fixed-capacity identifier-only event queue;
- canonical six-task graph construction and validation;
- task state transitions and retry accounting;
- deterministic bounded diagnostic normalization;
- executor-owned bounded model and validator output sinks;
- generated-artifact ownership, failure rollback, and explicit release;
- payload-free workflow events;
- accepted or rejected terminal outcomes.

Model-backed stages use a concrete `ModelTaskExecutor`. Compile and validation
stages use a concrete `ValidationTaskExecutor`. These are coarse capability
service ports and are statically dispatched by the workflow executor. Requests
contain task metadata, the validated model policy where applicable, and borrowed
`ArtifactId` slices only. A restricted `ArtifactInputs` view enforces that a port
can resolve only its declared inputs, including when the executor retains artifacts
from earlier workflows.

Ports do not return fully allocated artifact payloads. A model port appends chunks
to an executor-owned `BoundedTextSink`; a validator appends typed findings to a
`BoundedDiagnosticsSink` and returns only its verdict. Each append preflights the
complete logical size, reserves storage fallibly, and either commits the whole
chunk/finding or leaves accepted output unchanged. Capacity and allocation failure
are sticky, non-retryable workflow failures. Operational port failures alone consume
the task retry budget. This keeps artifact storage policy stable whether work runs
locally, on a peer, or through a hosted provider.

The ports deliberately do not define tensor execution, prompt rendering, sampling,
compiler process sandboxing, or vendor-specific error formats. Those policies
belong in local E0 execution, provider/peer execution composition, or validator
adapters as appropriate. The workflow itself does not know where model work runs.

## Artifact lifecycle

One workflow starts from a previously committed, size-bounded specification
artifact. Specifications are shared roots with no workflow owner. Every generated
artifact records the `WorkflowId` that owns it.

Before allocating workflow/task/output identities or invoking any port, the executor
admits all six required artifact slots and the worst-case event count permitted by
the configured retry budgets. After admission, output identities are reserved before
graph validation, but payloads become visible only when their stage completes
successfully.

Commit order is strict:

1. execute into the stage's bounded sink, or run bounded normalization;
2. reject capacity/allocation failure without truncation or retry;
3. verify the complete typed artifact size with checked arithmetic;
4. commit the immutable workflow-owned artifact;
5. emit an identifier-only artifact event;
6. mark the matching `TaskAttempt` successful;
7. allow dependent tasks to start.

This prevents downstream work from observing an artifact identity without a
committed payload. Duplicate artifact identities are rejected and never overwrite
existing content. If any later stage fails, all artifacts and queued events owned by
that workflow are removed; checked identity sequences remain monotonic and are not
reused. Successful workflow evidence remains inspectable until the caller invokes
`release_workflow`, which removes only that workflow's generated artifacts and
preserves specifications, other workflows, and queued events.

Specification, draft, raw validation, normalized diagnostics, review, revision,
and final validation payloads are each stored once. Task requests, graph edges,
outcomes, and events retain identifiers rather than duplicating prior transcript
content. [ADR-0011](../agent/decisions/0011-bound-workflow-output-at-the-port.md)
records the output and release decision.

## Validation semantics

A validator returns a `ValidationVerdict` while appending typed `RawDiagnostic`
values to bounded executor-owned storage. Together they form the committed
`ValidationReport`. The normalizer:

- trims optional codes and source paths;
- removes empty optional strings;
- trims and collapses message whitespace;
- sorts findings deterministically by typed fields;
- removes exact duplicates;
- preserves the validator verdict.

No vendor-formatted diagnostic string is parsed.

`ValidationVerdict::Rejected` means the validator executed successfully and found
problems. The initial rejection therefore continues through normalization, review,
and revision. Only an operational port error consumes an attempt. Exhausting the
configured attempt budget returns a typed terminal failure containing the final
owned diagnostic. Model and validator ports receive non-zero token budgets and are
responsible for enforcing them at their tokenization/execution boundaries; the
workflow engine independently enforces complete artifact byte bounds during output
production. Diagnostic normalization uses fallible collect/sort/dedup construction,
checks its final typed report against a separate bound, and remains a single-attempt
non-tokenized task. The final verdict maps to an accepted or rejected
workflow outcome, both of which reference committed revision and final-validation
artifacts.

## Current composition

The `corrective-workflow` engine is separately composable from
`ApplicationRuntime` and the hosted local inference lifecycle. This permits deterministic
validator and model-service implementations to be selected without exposing
Candle, compiler-process, channel, or UI types through the public workflow
contract.

A model-task port should delegate complete generation through the selected model
execution capability. The current local composition may route that work to E0; a
future peer or hosted-provider implementation can satisfy the same coarse port
without pretending to own local sequences. It must not implement a token-by-token
frontend round trip. A compiler or validator adapter must additionally enforce its
own timeout, output bound, working-directory, environment, and untrusted-code
policy.
