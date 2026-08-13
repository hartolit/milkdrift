# Milkdrift project architecture

Milkdrift uses the
**[Layered Workspace](../architecture.md#model-b-layered-workspace)** model.
Accepted [ADRs](../agent/decisions/README.md) record rationale, and
[workspace boundaries](workspace.md) owns the exact crate inventory and dependency
edges.

Milkdrift remains workflow-first: operator-defined workflows, scoped context,
explicit authority, and replaceable execution targets are the project center. The
current Candle/E0/E1/Slint path is a local-inference foundation plus an optional
reference application-services kit. It does not define the only Milkdrift API,
workflow plane, or future control surface.

## Physical roles

```text
.cargo/             workspace-local Cargo configuration
tools/xtask/        architecture, hygiene, and verification tooling
benchmarks/runtime/ non-production measurement observer
crates/domain/      portable contracts and algorithms
crates/platform/    process-host execution primitives
crates/adapters/    vendor, model, filesystem, network, and persistence adapters
crates/runtime/     stateful capability and resource-owning runtimes
crates/apps/        process, event-loop, transport, and presentation boundaries
```

Every tracked non-fixture Cargo package manifest must be a root workspace member, and every member declares its role under `[package.metadata.milkdrift]`; the validator never infers a role from a path prefix or package name. The current vocabulary is `tooling`, `benchmark-observer`, `domain-foundation`, `domain-feature`, `platform`, `adapter`, `runtime-foundation`, `runtime-capability`, `runtime-application`, and `application`. A declaration must also occupy the direct physical root compatible with that role, so omitted members, unknown/missing roles, and misplaced packages fail closed.

The logical dependency direction is inward:

```text
apps / hosts / transported frontends
       │
       ├── may use application-runtime (optional E1 reference services)
       │             ├── capability engines
       │             └── inference-runtime (E0 local inference)
       │
       └── may use another reviewed workflow or coarse execution boundary

E0 / capability engines
       ↓
platform / adapters / domain algorithms
       ↓
domain-contracts
```

E1 may coordinate E0 and independently stateful capability engines. E0 and
capability engines never depend on E1. A host should not reconstruct E1's state
machines if it chooses E1, but applications whose semantics differ are not forced
through it.

Packages with role `benchmark-observer` remain outside the production graph. They may observe public product APIs but may not use build dependencies or become a dependency of product, tooling, tests, applications, or another observer. `runtime-benchmarks` is the current observer; the policy can accept another explicitly declared observer without adding a package-name match.

## Domain tiers

`domain-contracts` is the F0 shared foundation. F1 algorithm crates such as
`tokenization`, `context-planner`, `sampling`, and `task-graph` depend inward on
stable domain contracts. Portable domain code does not import runtimes,
applications, platform implementations, vendor libraries, frontend toolkits, or
filesystem/network/database implementations.

`task-graph` owns generic directed-work mechanics only: stable task identity,
topology/acyclicity, bounded attempt state, deterministic ready discovery,
cancellation/blocked propagation, and identity-only artifact provenance. Its
generic node operation metadata is caller-owned and uninterpreted. Model/backend
policy, token/output limits, artifact kind/role, and corrective operations remain
above the graph boundary.

Cargo's actual normal/build graph among F0/F1 packages must remain acyclic. Ordinary legal inward or peer domain edges are not copied into a second exact-edge registry; the validator derives the graph from Cargo metadata and rejects upward facilities, unsupported external dependencies, and cycles. See [portability](portability.md) and [dependency policy](dependency-policy.md).

## E0: local inference ownership

`inference-runtime` is E0, the backend-independent single-owner local inference
kernel. It exclusively owns:

- exact prepared-load transactions and aggregate admission;
- prepared, loaded, incompatible, and cleanup-retained native owners;
- model generations, backend sequences, and generation workspaces;
- scheduling, sampling, cancellation boundaries, draining, and unload; and
- cleanup retry/exhaustion plus terminal process-lifetime retention.

`prepare_load` returns one ordinary-drop-safe,
source/configuration/device/budget-bound preparation and stable plan. E0 validates
and reserves the loading peak, consumes that preparation through `load_prepared`,
verifies the complete model, and replaces the peak with final exact ownership only
on commit. Failed materialization returns a distinct resource-bearing typestate
inside the fail-closed `FailedLoad` guard. A failed owner that changes its accepted
plan, or a contract-violating complete model that cannot unload, becomes unverified
ownership and blocks new admission.

Public handles carry generation-safe identity, not shared model ownership. Hosted
providers and peers are not E0 backends merely because they produce text; their
ownership, cancellation, accounting, and transport semantics require a coarser
execution boundary.

## Capability engines

A capability engine owns independently stateful reusable behavior with a lifecycle
separate from the application façade. `corrective-workflow` is the current example:
it owns bounded data-defined corrective operations, typed model/validator ports,
workflow artifacts, attempts, retries, validation state, bounded output, release,
and events without owning the application or local-inference lifecycle. Its
six-stage reference flow is template data interpreted by the same executor used
for other legal corrective definitions; it is not the final general workflow
runtime.

New engines require evidence of a coherent independent state/lifecycle boundary.
They do not depend on one another by default; a higher coordinator composes them.

## Optional E1 reference services

`application-runtime` is E1 for the current reference application. It owns
frontend-neutral application selection, immutable Hub resolution, selected-device
state, persistence, one-resident-model lifecycle, completion, exact compatible
chat, conversation/context behavior, bounded text output, retained cleanup, events,
and worker shutdown.

Its private composition contains one monomorphized
`HostedRuntime<CandleLlamaSource>`, one inference thread, one bounded Hub worker,
one `HfTokenizer`, request-local streaming decoders, and redb storage. Static
Candle execution stays behind the non-generic public façade.

`ModelSelection` contains only repository/revision. Public `ResolvedModel` exposes
selection, immutable identity, vocabulary, recognized-or-absent declaration, and
unit chat compatibility. It exposes no engine/source/format helpers. Public
`LoadedModel` gets actual execution scalar/device only from E0's verified receipt.

Hub declaration states are strict: absent or recognized declarations continue
through device-independent resolution; malformed, unsupported, or conflicting
declarations fail with stable Hub/application categories and no raw vendor value.
At Candle inspection, an absent declaration is sufficient only for a homogeneous
required tensor set. Mixed required `{F16,F32}` and `{BF16,F32}` layouts require the
matching recognized producer declaration before any lossy conversion.

E1's load transaction snapshots resolution/admission, submits one ticketed E0
load, and applies named generic receipt checks for correlation, identity,
declaration, scalar/device, selected device, budget/footprint, nonempty observed
evidence, capabilities, composition, limits, and tokenizer vocabulary. It accepts
nonempty observed sets containing unused `F16`, `BF16`, `I8`, `U8`, or `Other` and
does not reproduce Candle's required-tensor policy. Its footprint check uses
checked host/device totals against the fixed budget; it does not impose CPU/CUDA
component placement.

CPU is mandatory/default. CUDA is an explicit feature-gated selection with no CPU
fallback. `ApplicationDeviceSummary` publishes structured facts and an optional
backend-reported `display_name`, not presentation labels.

## Current local composition

```text
apps/desktop-slint
        ↓ optional reference host
application-runtime (E1)
        ├── Hub worker → hf-hub-adapter → immutable artifacts + shard identity
        ├── hf-tokenizer / request-local decoder
        ├── redb-storage
        └── hosted inference worker
                    ↓
             inference-runtime (E0)
                    ↓
             candle-backend
                    ↓
       Safetensors + selected CPU / feature-gated CUDA
```

Engine, artifact source, model format, declaration, observed scalar set, required
tensor policy, execution scalar, and device are distinct concepts. Candle owns
Safetensors inspection, required-range conversion, exact loading/final planning,
materialization, and placement. E0 owns generic plan admission and receipt
verification. E1 owns only its generic application transaction and projection.

The opt-in CUDA feature path is
`desktop-slint/cuda -> application-runtime/cuda -> candle-backend/cuda`; default
builds do not reach CUDA. GGUF, quantized loading, Metal, generic GPU aliases, and
automatic CPU fallback are not implemented by this composition.

## Adapters and frontend boundary

`host-runtime` quarantines bounded channels, named threads, monotonic timing, and
typed pull-oriented text/token output. One private statically dispatched core owns
the output locking, fixed storage, monotonic cursors, atomic commit, and allocation
reuse; text and token ranges remain distinct public types. The platform crate owns
no model, workflow, conversation, or application state.

Adapters own vendor/model/persistence details and do not depend on runtimes. The
local reference path composes `hf-hub-adapter`, `hf-tokenizer`, `redb-storage`, and
`candle-backend` behind E1/E0 boundaries.

`desktop-slint` owns platform paths, the native event loop, callbacks, and
presentation. It constructs labels from structured facts and projects only
repository/revision, optional declaration, selected device, actual receipt
execution, and retained state. The unit chat compatibility fact controls behavior
without exposing a profile. Each 16 ms frame drains at most 64 events and performs
one bounded text pull.

A Slint, Tauri, TUI/CLI, headless, transported, or workflow host may choose the
boundary matching its semantics. Browser and remote targets require explicit
transport/capability contracts; none is implied by E1.

## Lifecycle and persistence policy

Cleanup failure does not imply release. Public E1 retained state distinguishes
resource, `Exact`/`Unverified`/`Unknown` ownership, lower retry/exhaustion,
coordination retry, disconnect, process-lifetime retention, and independent primary
and cleanup failures. Retained state has no simultaneous normal `LoadedModel` and
locks selection/load.

Only correlated explicit release, successful unload, or clean E0 shutdown is
release evidence. Disconnect, worker/join-handle absence, zero exact aggregate
bytes, or a missing bounded snapshot owner is not. Shutdown command outcomes and
worker joins are tracked independently from cleanup; terminal retention survives
worker exit until process reclamation.

`LAS1` settings write version 2 and read exact version 1. `LAM1` model records write
latest version 3 with declaration presence tag `0` for absent or tag `1` plus code
`F32`/`F16`/`BF16`; exact versions 1 and 2 remain readable without automatic
rewrite. Key/name mismatch, corrupt records, and unknown versions are explicit.
Runtime execution and ownership facts are not persisted. The timestamp field is
`last_resolved_unix_milliseconds`.

## Generic role DAG and enforcement

`cargo xtask architecture` validates the declared roles and this compact dependency model:

```text
application         -> runtime-application
runtime-application -> runtime-capability, runtime-foundation,
                       adapter, platform, domain-feature, domain-foundation
runtime-capability  -> runtime-foundation,
                       adapter, platform, domain-feature, domain-foundation
runtime-foundation  -> adapter, platform, domain-feature, domain-foundation
adapter              -> platform, domain-feature, domain-foundation
platform             -> domain-feature, domain-foundation
domain-feature       -> domain-feature, domain-foundation
domain-foundation    -> domain-foundation
```

Same-role runtime peers are not generally legal. Domain peer/foundation edges participate in the actual Cargo-derived acyclic domain graph. Tooling has no workspace-local edges. Observers are outer-only and build-script-free. The root policy namespace and exact integer `policy-version = 1` are mandatory. Normal/build edges that already obey these invariants need no duplicate record; workspace-local development edges and restricted external edges require exact live exception records with source, target, kind, stable ID, and rationale. Stale, wrong-kind, duplicate, unnecessary, and absolute-denial override records fail.

Default features, CUDA aliases, direct dependency feature selection, the exact provider contract, and reviewed forwarding remain separately fail-closed. `cuda-hardware-tests` is a package-local, non-default test alias and cannot be forwarded by another package.

A future workflow runtime declares `runtime-capability` or the lifecycle-appropriate runtime role; a portable workspace or plugin SDK declares a domain role; a provider implementation declares `adapter`; a headless process declares `application`; and a repository utility declares `tooling`. Those packages require no package-name registry change. A new role or a genuinely exceptional edge remains an architecture change.

`cargo xtask hygiene` enforces repository policy, and `cargo xtask verify` composes the project gates plus exact registered benchmark compilation. Project-authored code denies unsafe code, with narrow generated Slint exceptions documented in [workspace boundaries](workspace.md).

This architecture page does not assert validation of the current tree or broaden historical hardware support. The authoritative support and evidence record remains [implementation status](implementation-status.md).
