# Corrective reference orchestration

## Scope and status

`crates/runtime/corrective-workflow` is an incubating reference capability engine.
It proves bounded data-defined corrective execution over a generic portable graph;
it is not the final Milkdrift workflow runtime or schema.

The implemented engine deliberately supports only synchronous in-memory runs,
static typed model and validator ports, deterministic diagnostic normalization,
and a small corrective operation vocabulary. General operator workflows, durable
context workspaces, plugins, external providers/peers, recursive child runs,
authority/effect systems, persistence, and visual graph editing remain
unimplemented product direction.

## Generic graph boundary

`task-graph` is an F1, `no_std`, allocation-free domain crate. It owns only
semantics valid for arbitrary bounded directed work:

- stable `TaskId` values and generic `TaskNode<Operation>` values whose operation
  metadata is never interpreted by graph algorithms;
- duplicate-free node/dependency definitions and acyclic topology;
- deterministic ready discovery in caller definition order;
- caller-owned runtime state;
- one-based `TaskAttempt` identity and stale-attempt rejection;
- pending, running, succeeded, retryable failed, exhausted, cancelled, and blocked
  state transitions;
- bounded attempts, cancellation, and blocked-descendant propagation; and
- identity-only external input, producer, consumer, and direct-dependency
  provenance validation.

The graph does not own task kinds, model/backend selection, token budgets, output
byte limits, artifact media or semantic roles, corrective payloads, or port
behavior. Tasks may produce zero, one, or many artifacts. The graph's generic
artifact primitive is only `ArtifactId` flow and provenance; a higher capability
schema decides what an artifact means.

Topology validation uses caller-owned incoming-count and Kahn-queue scratch.
Artifact validation uses repeated borrowed-slice scans. Graph runtime state and
ready-task output are caller-owned. The allocation contract test measures graph,
provenance, and state transitions after preparation and requires zero allocations
and reallocations.

## Corrective definition boundary

`CorrectiveWorkflowDefinition` layers current corrective meaning above the generic
graph. A borrowed definition contains bounded data for:

- corrective task operation and event-facing stage label;
- graph dependencies;
- corrective artifact kind, role, and produced byte limit;
- external, task-input, and task-output artifact bindings;
- model selection and non-zero token budgets where model work is required;
- validator operation and token budgets;
- per-node generic attempt bounds; and
- terminal result and final-validation artifacts.

Corrective validation is intentionally stricter than the generic graph: every
currently supported corrective operation produces exactly one artifact, operation
and output role must agree, normalization consumes one raw diagnostic artifact and
runs once, and terminal artifacts must be a produced draft/revision plus a final
validation report. These are capability constraints, not graph axioms.

The six-stage reference behavior is `ReferenceCorrectiveTemplate` data:

```text
draft
→ compile-check or validate
→ normalize diagnostics
→ review
→ revise
→ validate
```

The convenience `execute_reference` method constructs this template, binds the
specification, and invokes the executor's ordinary definition path. The same
executor tests a structurally different three-node definition containing two
initially ready draft operations and one terminal validator. This proves that node
selection and ordering come from definition data rather than a canonical call
sequence hidden in scheduler code.

## Admission and execution transitions

Before model or validator side effects, the executor:

1. checks definition collection counts against configured task, edge, artifact,
   output-binding, external-input, and task-input maxima;
2. validates graph topology and artifact provenance with exact-size scratch;
3. validates corrective operations, outputs, normalization, and terminal roles;
4. resolves every external input to a committed compatible artifact;
5. admits all generated artifact entries; and
6. admits the worst-case event count implied by every task's attempt bound.

Only after successful admission does it allocate monotonic workflow, run-task, and
output-artifact identities and fallibly prepare bounded run mappings, graph state,
ready scratch, and declared input lists.

Each scheduler iteration asks `TaskStateTable` for ready nodes and selects the
first definition-ordered identity. The transition is then:

1. check cancellation for the selected run-unique task;
2. start the generic attempt and emit `StageStarted`;
3. execute the declared model, validator, or normalization operation;
4. reject sink accounting/capacity/allocation failure without retry;
5. size-check and commit the immutable typed artifact;
6. emit `ArtifactCommitted`;
7. mark the matching generic attempt successful; and
8. expose newly ready dependents on the next scheduler iteration.

Operational port failures mark the generic attempt failed. Remaining attempt
capacity produces `RetryScheduled`; exhaustion propagates blocked descendants and
returns an owned terminal diagnostic. Cancellation marks the selected task
cancelled and propagates blocked descendants. Either terminal failure path rolls
back the workflow's artifacts and queued events.

When every graph task succeeds, the executor reads the definition-selected final
validation report and produces an accepted or rejected `WorkflowOutcome`. Initial
validator rejection remains successful task output and can feed later corrective
stages; only operational failure consumes an attempt.

## Typed port authority and bounded output

`ModelTaskContext` and `ValidationTaskContext` carry only the authority required by
their port: workflow and attempt identity, definition-local task identity,
supported operation, applicable policy/token budget, and a restricted
`ArtifactInputs` view. Retained artifacts not declared for that task are invisible
through the view.

Ports append into executor-owned `BoundedTextSink` or
`BoundedDiagnosticsSink`. Each complete append is preflighted and reserved
fallibly before accepted storage changes. Sink failure is sticky, non-retryable,
and takes precedence over a simultaneous port error. No output is silently
truncated. Diagnostic normalization preserves the verdict, trims and collapses
typed fields, sorts and deduplicates deterministically, and builds within a
separate declared bound.

The port contracts do not define tensor execution, prompt rendering, sampling,
compiler process sandboxing, provider transports, or vendor diagnostic parsing.
Those belong to concrete execution or validator adapters.

## Artifact and event lifecycle

Root specifications are shared unowned artifacts. Every generated artifact records
its `WorkflowId`. A failed or cancelled run removes its generated artifacts and
queued events; identity sequences are never rewound, so later runs cannot reuse
rolled-back workflow, task, or artifact IDs.

Successful artifacts remain until explicit `release_workflow`. Release preserves
specifications, other workflows, and queued completed-run events. Events remain
copyable identity-only values and never duplicate artifact payloads.

[ADR-0011](../agent/decisions/0011-bound-workflow-output-at-the-port.md)
records the executor-owned output and explicit release policy. The general
workflow/workspace program must preserve bounded ownership and explicit lifecycle
when it later introduces asynchronous, persistent, provider, peer, plugin, or
recursive execution.
