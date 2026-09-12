# milkdrift-control

This package lets a human, service, or AI inspect work and request changes through the same
authorized operations. `ControlService` translates those requests into immutable revisions and
[runtime commands](../runtime/README.md). It is the place to work on proposal handling, risk
classification, or controller policy; scheduling and event commits remain with runtime.

The same installed adapter exposes `workflow.accept_result`. Use an ordinary acceptance task and
branch to distinguish an invocation's completion from the output required by its workflow.
[Result acceptance](../../docs/guides/result-acceptance.md) explains the typed contracts, verifier
reports, authorization, and rejection routes. The operation reads immutable evidence and performs
no external validation work; the configured producer owns the checks it reports.

## Change a running workflow

Suppose a review step must be added after existing work. A caller submits a
[`WorkflowProposalDocument`](src/document.rs) naming the exact base revision and digest, the
observed run sequence, and an ordinary blueprint mutation. The proposal's author, risk notes,
requested action, and completion claim are input data, not permission or execution evidence.

[`ControlService::execute`](src/service.rs) checks the authenticated actor context, optimistic
guards, complete old/new capability requirements, and deterministic
[risk classification](src/policy.rs). A permitted proposal creates an immutable revision. For a
live run, runtime records a reconciliation plan explaining which work can change prospectively.

```text
proposal + exact base and run sequence
                |
       validation, authority, risk
                |
       immutable proposed revision
                |
       runtime reconciliation plan
                |
       recorded approval if required
                |
       apply at the plan's guarded boundary
```

Low risk allows automatic application only when requested and separately authorized. Approval
does not apply a plan by itself. Application checks that the plan still describes the run;
a stale plan requires a fresh proposal/decision. Completed work and already committed attempts
keep their original definitions. The [reconciliation planner](../runtime/src/reconciliation.rs)
owns the classifications and [ADR 0005](../../docs/decisions/0005-prospective-revision-reconciliation.md)
explains why a later edit cannot rewrite them.

Simple commands such as pause, resume, signal, cancellation, or retained-work resolution use the
same service. Inspect a `RunInspection` for the current operational frontier, or page the timeline
for older events. A compact current view does not imply that earlier attempts disappeared.

## Integrate the service

Construct `ControlService` with the same revision store, runtime, and authority evaluator used by
the rest of the application. Supply `ActorAuthorityContext` from a trusted authentication boundary.
The daemon's [receipt boundary](../../apps/daemon/src/host/receipts.rs) additionally retains the exact
external response. Runtime operations derive stable command identities, so redelivery can finish
recording a response after runtime acceptance without appending the same transition again.
Control calls may span several durable steps; an error is not a promise that nothing was saved.

[`WorkflowControlAdapter`](src/adapter.rs) exposes these operations as an ordinary hosted capability.
Its result sink publishes canonical results as artifacts. It gives workflow-driven controllers
the same command path and authority checks as other callers.

[`build_controller_blueprint`](src/controller/policy.rs) constructs an ordinary bounded repeat
wrapper. `ControllerLifecycleOwner` interprets its immutable policy and assesses cycles, proposals,
checkpoints, and cumulative usage. [Persistence accounts](../persistence/src/controller_account.rs)
reserve resources at final adapter entry and charge artifact publication; lifecycle assessment
consumes those facts. Authority presets expand to ordinary grants and enable no hidden role.

The production daemon currently leaves the lifecycle uninstalled. Library integration and
final-entry accounting exist, but production activation still requires the qualification recorded
in [status](../../docs/product/status.md) and [roadmap](../../docs/product/roadmap.md).
Constructing the service, a preset, or a controller blueprint does not install the lifecycle.

Run `cargo test -p milkdrift-control --all-features` for proposals, risk/authority refusal,
reconciliation, controller accounting, and restart cases. The ordinary tests use local fixtures;
they do not qualify real external-agent interoperability.
