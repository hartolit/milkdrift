# Corrective workflow orchestration

## Scope

The typed corrective workflow lives inside the existing E1 `application-runtime`
ownership domain:

```text
draft
→ compile or validate
→ normalize diagnostics
→ review
→ revise
→ validate again
```

No third engine crate is introduced. The project-specific engine tiers remain:

```text
application-runtime (E1 workflow and application use cases)
    ↓
inference-runtime (E0 model and sequence resource ownership)
```

The older implementation-plan example named `task-orchestrator`, but the current
architecture consolidates application orchestration in E1. Adding another engine
would duplicate ownership and violate that consolidation.

## Pure graph boundary

`task-graph` remains an F1, `no_std`, allocation-free feature crate. It owns:

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

## E1 execution boundary

`CorrectiveWorkflowExecutor<M, V>` owns:

- checked workflow, task, and artifact identity allocation;
- one fixed-capacity immutable `ArtifactStore`;
- one fixed-capacity identifier-only event queue;
- canonical six-task graph construction and validation;
- task state transitions and retry accounting;
- deterministic diagnostic normalization;
- output-capacity enforcement;
- payload-free workflow events;
- accepted or rejected terminal outcomes.

Model-backed stages use a concrete `ModelTaskExecutor`. Compile and validation
stages use a concrete `ValidationTaskExecutor`. These are coarse application
service ports and are statically dispatched by the workflow executor. Requests
contain task metadata, the validated model policy where applicable, and borrowed
`ArtifactId` slices only. A restricted `ArtifactInputs` view enforces that a port
can resolve only its declared inputs, including when the executor retains artifacts
from earlier workflows.

The ports deliberately do not define tensor execution, prompt rendering, sampling,
compiler process sandboxing, or vendor-specific error formats. Those policies
belong in their respective E0 or adapter implementations.

## Artifact lifecycle

One workflow starts from a previously committed, size-bounded specification
artifact. Before allocating workflow/task/output identities or invoking any port,
the executor admits all six required artifact slots and the worst-case event count
permitted by the configured retry budgets. Capacity failure therefore cannot leave
partially executed tasks or orphaned output artifacts. After admission, output
identities are reserved before graph validation, but payloads become visible only
when their stage completes successfully.

Commit order is strict:

1. execute the model, validator, or normalizer stage;
2. calculate the complete output size with checked arithmetic;
3. reject output beyond the declared `TaskOutputContract` without truncation;
4. commit the immutable typed artifact;
5. emit an identifier-only artifact event;
6. mark the matching `TaskAttempt` successful;
7. allow dependent tasks to start.

This prevents downstream work from observing an artifact identity without a
committed payload. Duplicate artifact identities are rejected and never overwrite
existing content.

Specification, draft, raw validation, normalized diagnostics, review, revision,
and final validation payloads are each stored once. Task requests, graph edges,
outcomes, and events retain identifiers rather than duplicating prior transcript
content.

## Validation semantics

A validator returns a typed `ValidationReport` with a `ValidationVerdict` and
`RawDiagnostic` values. The normalizer:

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
responsible for enforcing them at their tokenization/execution boundaries; E1
independently enforces complete artifact byte bounds. Diagnostic normalization is a
single-attempt non-tokenized task. The final verdict maps to an accepted or rejected
workflow outcome, both of which reference committed revision and final-validation
artifacts.

## Current composition

The workflow engine is intentionally separately composable from the existing
`ApplicationRuntime` model-acquisition and lifecycle host. This permits deterministic
validator and model-service implementations to be selected without exposing
Candle, GGUF, compiler-process, channel, or UI types through the public workflow
contract.

A production model-task port should delegate complete generation to E0 when that
coarse generation operation is connected. It must not implement a token-by-token
frontend round trip. A compiler or validator adapter must additionally enforce its
own timeout, output bound, working-directory, environment, and untrusted-code
policy.
