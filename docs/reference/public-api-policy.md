# Public API policy

Milkdrift is unreleased. Workspace libraries, daemon and clients upgrade together. Rust APIs,
public protocols and durable development formats may change in one coherent revision; update
producers, consumers, fixtures and documentation together and refuse unsupported versions.
An incompatible upgrade must not reinterpret saved facts or silently give old requests authority.

## Classification

Every exported item has a current reason to cross its package boundary:

1. **External product contract:** intentionally consumed by an application or adapter.
2. **Workspace adapter contract:** implemented or consumed by another workspace package.
3. **Durable schema contract:** required to interpret accepted storage or exchanged data.
4. **Test-only exposure:** explicitly gated fixture/fault support.
5. **Accidental exposure:** no current consumer or invariant; remove or narrow it.

A workspace consumer can justify visibility without promising a stable third-party API. Root
re-exports make an intentional entry point or returned type nameable. Private implementation
modules stay private. Do not preserve a retired owner solely to keep an old API inventory unchanged.

## Surface ownership

Blueprint task definitions and bindings are semantic contracts consumed by runtime/control and
serialized in frozen starts. Runtime's task records expose immutable starts/results to persistence
and authorized daemon projections. Construction and transition ownership remain in runtime;
clients never write those records directly.

`InvocationAdmissionEnvelope::execution_duration_ms` is an adapter contract: its bound must come
from an enforced attempt deadline, never a descriptor estimate. Runtime's `TaskExecutionDuration`
is durable evidence for an accepted reservation and its elapsed-time watermark. These exports
support the same cumulative workflow account; they do not create a second scheduling or budget owner.

`WorkflowStore` commits a snapshot and exact receipt atomically with generation and capacity checks.
Its artifact, entered-invocation and managed-use facts allow the durable owners to enforce holds
and publication without decoding a second copy of runtime semantics. Native-path claim ports
coordinate physical path identity and preserve uncertain writers. They are adapter contracts,
not alternate schedulers.

Public task DTOs in `control-protocol::tasks` are transport contracts for CLI, Svelte and independent
JSON consumers. Rust generates the TypeScript declarations and envelope decoder. Summary/read/result
DTOs deliberately have different payload sizes: a discovery page never embeds all captured files.
`validate_response_capacity`, cursor binding and error status mapping remain pure wire helpers.

Independent `DirectInvocationDraft`, request preparation, serving invocation records, peer limits
and artifacts remain their host/protocol owners. Managed-resource requests and durable holds remain
independent of ordinary task definitions. No public graph, proposal, controller, prompt-sequence,
published-workflow or evaluated-learning API survives solely for old product consumers.

Evidence library exports serve separate Rust fixture/measurement binaries. Product dependency
resolution must not enable `test-support`, `test-admin` or `operational-evidence` helper features.
The shared child/HTTP fixture lifecycle preserves failure and cleanup evidence; it is not a product
server implementation. UI caches and generated DTOs add no durable authority.

## Review method

Trace changed exports through production callers, tests, docs and wire/durable readers. Inspect
library roots and default/all-feature inventories, keeping raw generated reports out of Git:

```sh
mkdir -p target/public-api
cargo public-api -p PACKAGE -sss --all-features --color never > target/public-api/PACKAGE.all-features.txt
cargo public-api -p PACKAGE -sss --color never > target/public-api/PACKAGE.default.txt
```

The default comparison is required for packages with test/evidence features. Check dependency
resolution independently for production daemon/CLI; a workspace all-feature build can hide a
helper-feature leak. Record which inventories and checks actually ran in verification evidence,
including toolchain or environment limits. A type count is not proof of a coherent boundary.

The prompt-01 task cutover intentionally removes the previous graph/publication/controller API
families. Review new task, storage, host and transport boundaries together with their consumers.
Later method and automation exports require their own explicit assignments and review.
