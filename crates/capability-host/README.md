# Hosting external capabilities

This crate connects the durable runtime to live implementations of `CapabilityAdapter`. The
runtime decides which work may proceed and records what happens; the host owns registered adapter
generations, execution permits, and workers. Start here when changing that connection or implementing
an adapter. Operators configure existing adapters through the
[daemon guide](../../docs/operations/daemon.md).

## Follow one task

The daemon's [capability construction](../../apps/daemon/src/host/capabilities.rs) loads validated
process/model profiles, creates adapters with explicit data and secret ports, registers descriptors,
and refreshes health. Registration calls `authority_requirements` and `start` before publishing the
generation. Re-registering the same descriptor replays the existing registration; it does not replace
the live adapter. Changed facts need a new descriptor revision.

The runtime reaches the host through `TaskExecutor`. The following shows the local runtime path;
the [peer service](../../adapters/peer-http/README.md) supplies its own durable acceptance boundary.

```text
task requirement
  -> host: match current descriptors, evaluate authority, check health/capacity
  -> runtime: persist selection and frozen context, claim the invocation effect
  -> runtime: recheck the exact attempt/lease and current authority
  -> host: acquire exact-generation permit and derive request resource bounds
  -> runtime: commit entry decision and any controller reservation atomically
  -> adapter: prepare inputs, enter external work, report observations
  -> runtime reporter: durably accept observations and update run state
```

Resolution returns a selection, not a held permit. The later claim can fail if capacity or the
exact generation is gone. No entry or cancellation path substitutes another generation. New
resolution considers the highest non-draining revision per identity and ranks eligible candidates
by configured priority, capability identity, then revision. Missing, unavailable, or stale health
prevents new selection. The daemon periodically refreshes current generations using its boundary
clock; adapters must return that supplied timestamp.

## Implement an adapter

Read [`CapabilityAdapter`](src/adapter.rs) for the lifecycle contract, then a concrete implementation
such as [local process](../../adapters/local-process/README.md). One instance represents one
generation. Declare its filesystem, network, secret, and budget requirements before registration.
`admission_envelope` describes enforceable bounds for the exact request; an unavailable bound stays
unknown. It runs before durable entry, so it must not start the external operation.

`execute` receives the selected request and durable provenance. Send incremental observations through
`AdapterReporter`; a successful report has crossed the caller's durable boundary. Propagate reporter
failures. Return `Rejected`/`Unavailable` only when external work was not entered; use
`ExternalFailure` if the outcome cannot be proven after entry. Returning `Ok(())` without terminal
evidence does not complete the workflow. Runtime preserves an already durable terminal report even
if the worker later fails.

`InvocationDataAccess` lets adapters read selected inputs, lease an isolated directory, and publish
artifacts through persistence ports. `StoreInvocationDataAccess` verifies references and content,
uses the run's accepted workspace budget, and attaches invocation/input provenance to restricted
outputs. Callers supply authorized selections and artifact-read authority; this bridge is not a
fresh actor/grant evaluator. Distinct input names may reference the same exact artifact; produced
artifacts record that causal parent once while the invocation retains both bindings.
Dropping the materialization releases temporary files. Models can read
bounded bytes directly without creating a process directory. `SecretResolver` resolves explicit
references at the authorized adapter boundary; its production implementation is
[local-secret](../../adapters/local-secret/README.md).

## Cancellation and shutdown

`EffectWorkerHost::start` creates fixed execution threads plus one cancellation thread. The owner
calls `poll` to claim only as much work as its bounded queues can accept. The separate cancellation
queue lets a stop request run while execution workers are occupied.

```text
runtime cancellation effect -> host locates invocation's owning generation
  -> adapter.cancel -> acknowledgement for that exact request
  -> ongoing execution reports terminal evidence when its outcome is known
```

For example, a process adapter acknowledges a stop flag; its monitor delivers signals and observes
termination. The model route can close a local response but cannot prove remote termination.
Adapter hooks run outside the registry lock, with panic containment and permit release on return.
These mechanisms do not supply missing external completion evidence.

Draining removes a generation from new resolution while retained exact selections can still enter.
`finish_drain` requires all held permits to be released. For worker shutdown, `Drain` lets queued work
finish, `Cancel` asks adapters to terminate resources, and `Retain` skips queued entry so recovery
can account for it. The deadline result says whether shutdown was clean and lists known unresolved
invocations; a deadline does not forcibly join blocked threads. The embedding owner must keep
persistence available for final worker reports. The daemon owns that ordering in its
[shutdown path](../../apps/daemon/src/host/shutdown.rs).

`generations` and `catalog_generations` expose scoped read models rather than adapter handles.
Peer catalogs further filter these observations for their relationship and expiry. For adapter
tests, enable `test-support` and use `conformance::run_adapter_conformance` with a fresh fixture
per scenario. Keep mechanism-specific failure and cleanup tests alongside that shared contract;
[verification commands](../../docs/development/workflow.md#focused-suites) cover both.
