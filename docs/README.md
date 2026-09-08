# Milkdrift documentation

This index routes readers to each documentation owner. It does not duplicate product, status,
architecture, or protocol facts.

## Understand the product

- [Product vision](product/vision.md) owns enduring intent and non-negotiable semantics.
- [Architecture constitution](architecture.md) owns boundaries, terminology, dependency direction,
  and compatibility rules.
- [Current status](product/status.md) owns implemented behavior, limitations, and the latest evidence
  snapshot.
- [Roadmap](product/roadmap.md) owns ordered unfinished product work.
- [Architecture decision records](decisions/README.md) explain durable decisions and tradeoffs.

## Develop and verify

- [Development workflow](development/workflow.md) contains build, test, lint, fixture, and focused
  evidence commands.
- [Engineering rules](development/engineering-rules.md) define standing implementation-quality
  and documentation policy.
- [Virtual office](development/virtual-office/README.md) holds temporary sprint assignments and
  handoffs, with a [whiteboard](development/virtual-office/whiteboard/README.md) for broader issues
  and discussions that may span sprints. Completed sprint files are removed after useful outcomes
  are incorporated into maintained documentation.
- [Verification and operational evidence](development/verification-evidence.md) defines repeatable
  evidence lanes and their limitations.

## Use Milkdrift

- [Headless prompt-sequence dogfood](guides/headless-dogfood.md)
- [Local process operator guide](guides/local-process.md)
- [Local model endpoint guide](guides/local-model-endpoint.md)
- [External interoperability evidence](guides/external-evidence.md)

## Operate Milkdrift

- [Daemon operation and durable state](operations/daemon.md)
- [Control and execution authority](operations/authority.md)
- [Peer connectivity](operations/peers.md)

## Look up a contract

- [Local control API](reference/control-api.md)
- [Peer protocol](reference/peer-protocol.md)
- [Prompt-sequence schema](reference/prompt-sequence-v2.md)
- [Public API policy](reference/public-api-policy.md)

Executable schema constants, readers, and golden fixtures remain the primary evidence for current
contract versions.

## Learning the implementation

Follow one run through these six boundaries. Use IDs returned by the CLI; source links identify
the decision owners, and tests observe their results through public boundaries. The exact package
map is in [architecture](architecture.md#owners-and-dependency-direction).

1. **Run the fresh-directory scenario.** Execute the [README quick start](../README.md#fresh-directory-quick-start),
   then the [ordinary process example](../examples/operator/README.md#one-byte-pinned-local-process).
   Begin at daemon [configuration compilation](../apps/daemon/src/config/compile.rs),
   [startup](../apps/daemon/src/host/startup.rs), and [shutdown](../apps/daemon/src/host/shutdown.rs).
   The independent actual-binary [starter assertions](../tools/evidence/src/bin/headless-cli-evidence/setup.rs)
   run with the [headless evidence command](development/verification-evidence.md#actual-binary-scenarios).
2. **Trace one command.** Repeat the starter import with its original command ID and bytes, then
   change the request under that ID to observe conflict. Follow CLI
   [blueprint routing](../apps/cli/src/command/blueprint.rs),
   [control client](../crates/control-client/src/lib.rs), daemon
   [HTTP command adaptation](../apps/daemon/src/http/commands.rs),
   [authentication](../apps/daemon/src/auth.rs), [authorization](../apps/daemon/src/host/authorization.rs),
   [receipts](../apps/daemon/src/host/receipts.rs), and runtime
   [command admission](../crates/runtime/src/engine/command_planning/admission.rs).
   Independent proof: `durability::daemon_command_idempotency_restart_and_stale_conflict` in
   [control_plane](../apps/daemon/tests/control_plane/durability.rs).
3. **Trace one external attempt.** Run `milkdrift --json attempt inspect run-process ATTEMPT_ID`.
   Follow runtime [scheduling](../crates/runtime/src/engine/scheduling.rs),
   [dispatch](../crates/runtime/src/engine/dispatch.rs), [final entry](../crates/runtime/src/engine/effects/entry.rs),
   host [permit-backed execution](../crates/capability-host/src/registry/execution.rs),
   process [monitor](../adapters/local-process/src/process/monitor.rs), and
   [durable reporting](../crates/runtime/src/engine/effects/reporter.rs).
   Independent proof: `lifecycle::terminal_observation_precedes_a_later_worker_boundary_failure`
   in [structured_runtime](../crates/runtime/tests/structured_runtime/lifecycle.rs).
4. **Trace one artifact.** Run `milkdrift --json artifact metadata ARTIFACT_ID` and
   `milkdrift --json artifact get ARTIFACT_ID --output result.txt`.
   Follow the host [publication/materialization port](../crates/capability-host/src/materialization.rs),
   redb [publication](../adapters/redb-store/src/artifact/publication.rs), and daemon
   [authorized read owner](../apps/daemon/src/host/artifacts.rs).
   Independent proof: `artifact::artifact_publication_fault_boundaries_recover_without_dangling_metadata`
   in redb [contracts](../adapters/redb-store/tests/contracts/artifact.rs).
5. **Restart and trace recovery.** Follow the [operator restart procedure](../examples/operator/README.md#startup-and-restart),
   then inspect `milkdrift --json run show run-starter` and its timeline. Trace redb
   [journal append](../adapters/redb-store/src/journal/append.rs), runtime
   [projection replay](../crates/runtime/src/projection/replay.rs) and
   [recovery](../crates/runtime/src/engine/recovery.rs).
   Independent proof: `redb_run_replays_after_complete_object_teardown_and_finishes` in
   [durable_runtime](../crates/runtime/tests/durable_runtime.rs); the headless evidence scenario
   additionally kills and reopens the actual daemon and checks retained uncertainty.
6. **Apply a prospective revision.** Use the [failure/remediation commands](guides/headless-dogfood.md#failure-and-remediation)
   to submit, inspect, approve, and apply with exact returned guards. Trace control
   [proposal service](../crates/control/src/service.rs), runtime
   [classification matrix](../crates/runtime/src/reconciliation.rs), and
   [application](../crates/runtime/src/engine/reconciliation.rs).
   Independent proof: `control_workflows::headless_dogfood_failure_remediation_and_restart_are_durable`
   in [control_plane](../apps/daemon/tests/control_plane/control_workflows.rs).

Run an individual proof with `cargo test -p PACKAGE --test TARGET --all-features FILTER -- --exact`.
Use the target and filter above; [workflow](development/workflow.md) lists focused suites and
platform prerequisites. The actual-binary scenario also exercises guarded proposal adoption.
