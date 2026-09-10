# milkdrift-blueprint

To describe a workflow such as “implement a change, verify it, then review a failure,” define the
tasks and their relationships here. The result is an immutable `BlueprintRevision` that can be
inspected, saved, and submitted to the runtime. A revision describes intended work; a run's events
later record which tasks actually executed and under which revision.

The [prompt-sequence compiler](../prompt-sequence/README.md) is a concrete author of these
definitions. It emits ordinary nodes and edges, using the same mutation and validation APIs as
other callers. The [crate example](src/lib.rs) shows the construction path in Rust.

## Assemble work and its inputs

Start an external task with [`TaskConfig`](src/model/node.rs). A capability requirement describes
the operation needed and any exact capability/profile restrictions. A context policy asks which
earlier evidence the task should receive. Live capability selection and permission checks happen
later, when the task is prepared for execution.

Declare ports on the task's `Node`. Control ports and edges express ordering; data ports and edges
express which values flow between nodes. A [`DataPort`](src/model/contract.rs) input can bind a
literal, workflow input, prior node output, or durable reference. A `NodeOutput` binding also needs
the corresponding data edge: the binding chooses the value or path within it, and the edge declares
the dependency. Schema compatibility uses exact schema identity and version.

Durable bindings name [workspace values or artifacts](../workspace/README.md); they do not embed
files or prove that referenced bytes exist. Runtime preparation resolves those references. For a
model task, the input named `milkdrift.model_task` carries the
[model request document](../model/README.md#construct-a-request).

## Express decisions and structured work

Structured operations are `NodeKind` variants in the same graph. A branch evaluates declared
conditions to choose a route. A fork creates isolated branches; a join decides when enough of its
own branches have finished. A reducer separately collects or composes the declared results, so
waiting for branches does not implicitly merge their private workspaces.

Use a pinned subworkflow to call an exact revision and interface. Repeat invokes a pinned body
under an explicit iteration ceiling instead of adding a cycle to the graph. Wait and signal-wait
nodes describe durable holds. Successful terminals must bind any required workflow outputs.
The [structured definitions](src/model/structured.rs) explain the policy choices; the
[kernel examples](tests/kernel.rs) show how their ports and edges fit together.

## Publish a change

Submit a complete `MutationBatch` to `BlueprintRevision::genesis`, or call `revise` with the exact
base revision ID. Operations apply in order to a private candidate. The final graph must have a
valid entry/terminal structure, reachable nodes, compatible bindings, and no control/data cycles.
A refusal leaves the original revision intact. Inspect `ValidationError::diagnostics()` and its
stable codes to locate the problem.

An edit creates another revision so existing executions can keep the definition that governed
them. `content_digest()` identifies semantic content; `id()` also binds parents, sequence, author,
and reason. Explicit merge parents record ancestry for a candidate the caller has already resolved.
Saving a revision does not adopt it into a live run: that uses the separate
[prospective reconciliation path](../../docs/decisions/0005-prospective-revision-reconciliation.md).

## Request context, then inspect the selection

`TaskConfig::direct_inputs` starts with declared inputs. To add earlier evidence, attach a
[`TaskContextPolicy`](src/context.rs); its executable review-task example requests bounded
ancestor evidence. Selectors request candidates, while exclusions, branch visibility, authority,
and budgets determine what can be supplied.

```text
task policy + available inputs and historical evidence
                         |
              runtime selects for an attempt
                         |
              manifest records selections and omissions
                         |
              host supplies selected content to the adapter
```

The [manifest](../model/README.md#follow-the-selected-context) records the actual selection for a
model/process attempt. It supplements declared input bindings, and retries preserve its selection.
Required-evidence checks survive `StopAtFirstOverflow`, and omission redaction follows independent
access facts. Runtime checks a model request's session against the task declaration before claiming
work; provider support remains separate. Older saved manifests are subject to the runtime's
[reuse checks](../runtime/README.md#run-structured-work-and-select-its-inputs), not silently upgraded declarations.

Definition construction needs no daemon or endpoint and has no feature flags. The kernel tests
cover graph construction, refusal, revision identity, and document round trips. Use the
[verification policy](../../docs/development/workflow.md#choose-verification-for-the-change) for
changes, and the [operator examples](../../examples/operator/README.md) to run a complete workflow.
