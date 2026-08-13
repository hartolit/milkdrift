# corrective-workflow

`corrective-workflow` is an incubating reference capability engine over the
generic portable `task-graph` mechanics. It is not Milkdrift's future general
workflow runtime.

## Data-defined execution

`CorrectiveWorkflowDefinition` borrows bounded arrays of:

- generic task nodes and dependencies;
- supported corrective operations and stage labels;
- definition-local artifact metadata;
- external, task-input, and task-output bindings;
- model/validator policy, token budgets, attempt limits, and output byte limits;
- terminal result and validation artifacts.

The executor validates shape limits, graph topology, provenance, corrective
operation/output compatibility, external bindings, artifact entry capacity, and
worst-case event capacity before allocating run identities or invoking a port.
It then initializes generic graph state, selects the first ready node in
definition order, dispatches its declared operation, and commits the output before
marking the attempt successful.

`ReferenceCorrectiveTemplate` expresses the current behavior as ordinary data:

```text
draft
→ initial validation
→ diagnostic normalization
→ review
→ revision
→ final validation
```

`CorrectiveWorkflowExecutor::execute_reference` constructs that template and
calls the same `execute` path available to other legal definitions. Tests also run
a structurally different three-node definition with two initially ready nodes;
there is no six-stage scheduler branch.

## Typed bounded ports

Model and validator ports receive operation-specific immutable contexts. Each
context contains run-unique and definition-local task identities, the configured
operation/policy/budget, and an `ArtifactInputs` resolver restricted to the
declared committed input identities. Ports cannot use that view to inspect retained
artifacts from another workflow.

Ports append to executor-owned `BoundedTextSink` or
`BoundedDiagnosticsSink`. Every append checks complete logical size and reserves
fallibly before commit. Capacity and allocation failures are sticky,
non-retryable, atomic, and never truncate output. Operational port failures alone
consume the generic task attempt bound.

## Transactions and lifecycle

Each successful attempt commits one immutable corrective artifact and one
identifier-only event before releasing dependent tasks. A late error, exhaustion,
or cancellation removes every artifact and queued event owned by that workflow.
Workflow, task, and artifact identity sequences remain monotonic and are not
rewound after rollback.

Successful output remains inspectable until `release_workflow` explicitly removes
the workflow-owned artifacts. Shared specification inputs, other workflows, and
already queued successful-workflow events remain intact.

## Deliberate limits

This engine supports only its typed model, validator, and diagnostic-normalization
operations; synchronous in-memory execution; bounded static definitions; and
static Rust ports. It does not implement persistent workspaces, arbitrary node or
plugin loading, provider/peer lifecycle, recursive or self-modifying graphs,
durable resumable runs, external effect authority, or visual graph editing. Those
remain responsibilities of the unimplemented general workflow/workspace program.
