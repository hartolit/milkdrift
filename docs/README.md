# Milkdrift documentation

Start with the [quick start](../README.md#fresh-directory-quick-start) to run a task, or read
[vision](product/vision.md), [architecture](architecture.md), [status](product/status.md) and
[roadmap](product/roadmap.md) in that order to understand the product and its current limits.

## Use and operate Milkdrift

- [Standalone Svelte client](../apps/web/README.md): independent hosting and browser setup.
- [Operator examples](../examples/operator/README.md): configured daemon, CLI and direct invocations.
- [CLI](../apps/cli/README.md): commands, machine output and unknown outcomes.
- [Daemon operations](operations/daemon.md), [authority](operations/authority.md) and
  [peers](operations/peers.md): deployment boundaries and protected resources.
- [Local processes](guides/local-process.md): trusted executable profiles and their lifetime limits.
- [Model endpoints](guides/local-model-endpoint.md): external model profile configuration.

## Look up a contract

[Control API](reference/control-api.md) describes task and independent host transport.
[Peer protocol](reference/peer-protocol.md) owns cross-host requests and transfer contracts.
[Public API policy](reference/public-api-policy.md) describes visibility and review obligations.
Executable constants, readers and golden fixtures establish supported versions.

## Develop and verify

Use [AGENTS](../AGENTS.md), [workflow](development/workflow.md), the
[practice guide](development/practices/README.md) and [verification evidence](development/verification-evidence.md).
Temporary assignments and their handoffs belong in the [virtual office](development/virtual-office/README.md).
Design decisions remain in [ADRs](decisions/README.md); older decisions and preserved evidence do
not advertise retired routes as current functionality.

## Learning the implementation

Follow a submitted task through these concrete owners:

1. CLI [workflow commands](../apps/cli/src/command/workflow.rs) or the Svelte client call
   [control-client](../crates/control-client/src/tasks.rs) / public HTTP. The daemon's
   [task routes](../apps/daemon/src/http/tasks.rs) authenticate and enter its bounded owner queue.
2. [Task control](../crates/control/src/tasks.rs) checks authority and passes exact public request
   bytes to runtime [admission](../crates/runtime/src/tasks/admission.rs). The atomic store retains
   the acceptance receipt with the task facts, so a missing reply can be recovered.
3. [Runtime tasks](../crates/runtime/src/tasks.rs) select ready work and freeze inputs before the
   host prepares an external call. [Capability-host](../crates/capability-host/README.md) retains
   generation, entry, materialization and lifetime responsibilities.
4. [Artifact publication](../adapters/redb-store/src/artifact/publication.rs) exposes complete
   verified output. Daemon [task reads](../apps/daemon/src/host/tasks/reads.rs) project authorized
   summaries, immutable results and bounded previews.
5. Restart the daemon with the same supported-format root. Its durable task record and receipt
   survive; external entry without a terminal report remains uncertain. The runtime
   [durability suite](../crates/runtime/tests/durable_runtime.rs) exercises those boundaries, and
   [daemon control-plane tests](../apps/daemon/tests/control_plane.rs) exercise public callers.

The [connected-task harness](../tools/evidence/src/bin/connected-tasks/main.rs) launches the actual
binary and configured external fixtures. Browser tests dispose the client before reconnecting;
the fixture server cannot complete the workflow on the browser's behalf.
