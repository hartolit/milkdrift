# Capability contracts

A task asks for an operation before the runtime knows which executor will perform it. This crate
gives blueprint, runtime, and adapters a shared language for that request and for the observations
returned afterward. It contains portable values and pure matching; the
[capability host](../capability-host/README.md) connects them to live adapters.

## From a requirement to one invocation

Four objects answer different questions:

| Object | What it tells the reader |
| --- | --- |
| `CapabilityRequirement` | What the task needs: an operation, optional exact identity/profile, features, streaming, cancellation, and acceptable effects/trust. |
| `CapabilityDescriptor` | What one immutable generation advertises, including operation schemas, concurrency limits, and resource estimates. |
| `ResolvedCapabilitySnapshot` | Which generation and operation were selected for an attempt, with a digest of the frozen selection facts. |
| Host-owned `CapabilityAdapter` | The live implementation that can enter a process, endpoint, or peer. |

For a supported consumer trace, follow `TaskConfig` in
[blueprint](../blueprint/src/model/node.rs), the runtime's
[dispatch construction](../runtime/src/engine/dispatch.rs), then the host's
[selection](../capability-host/src/registry/selection.rs) and
[entry](../capability-host/src/registry/execution.rs). The task keeps a requirement so later work
can resolve against available generations. An accepted attempt keeps a snapshot so a replacement
adapter cannot silently change the work already selected.

`CapabilityDescriptor::matches` checks semantic compatibility only. Repeated categories are
alternatives; every requested feature and trust zone must be advertised. A new requirement allows
any side-effect class until narrowed. Neither a match nor a trust-zone label grants permission.
The host evaluates authority, freshness, and capacity separately before selecting a candidate.
`CapabilityObservation` carries changing availability and load without changing descriptor identity.

## Reporting what happened

`InvocationRequest` names the exact capability, operation, profile, and inputs; its companion
snapshot supplies the descriptor revision. Inputs hold bounded inline JSON or opaque durable
references. A frozen context manifest has a separate reference, with reserved input names for
selected content. The host materializes those references; this crate does not fetch bytes.

Adapters emit sequenced `InvocationEvent`s containing progress, output references, and a terminal
report. Large results belong in artifacts. `InvocationFailure::retryable` is an adapter observation;
runtime retry decisions also depend on the recorded side-effect and idempotency contract. A lost
reply after external entry may therefore leave `Uncertain`, even when the transport reports an error.

Cancellation has two facts: `accepted` says the request was accepted for processing;
`terminal_boundary` says the executor can guarantee that no later external effect will occur.
The acknowledgement binds the invocation and cancellation request sequence. Receiving a request
alone never establishes that boundary.

## Using the portable forms

Use the `*Document::from_json` readers at serialized boundaries and `to_canonical_json` when exact
bytes matter. They check versions, duplicate keys, bounds, and domain invariants. The
[document owner](src/document.rs) and [golden tests](tests/runtime_contracts.rs) explain the
supported older invocation/snapshot forms; [status](../../docs/product/status.md) lists current
versions. Opaque reference identities deliberately keep workspace and storage types out of this
crate. `AdmissionBound::Unknown` likewise preserves a missing enforceable resource limit instead
of turning an estimate into a reservation guarantee.
