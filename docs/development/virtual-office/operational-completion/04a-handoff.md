# 04A: Authorized recovery controls

The user explicitly requested completing the missing repair assignment after offline assignment 04.
The same task implements this bounded addition on the existing uncommitted 04 work at base
`d537059`. It introduces no UI, provider family or workflow primitive.

## Responsibility and result

Expose the existing authorized proposal/reconciliation path while unsafe retained context blocks
ordinary startup. Keep execution unavailable in recovery mode; preserve original history, frozen
artifacts, receipts, account owners and durable clock behavior. After reviewed prospective repair,
a normal restart must validate the same generation before replacement work can execute.

Runtime owns the permanent command-mode restriction. Control owns proposal-policy selection and
approval. The daemon owns composition, health, protected read redaction and shutdown. The scheduler
now consumes a safe-restart reservation after cancellation before dispatch as well as after a
cancelled terminal result. The normal retained-context validator remains unchanged.

The canonical explanation is [daemon recovery](../../../operations/daemon.md#authorized-recovery-controls)
and [ADR 0035](../../../decisions/0035-authorized-recovery-controls.md). Runtime/control/daemon READMEs,
architecture, current status and control API reference describe their respective boundaries.

## Verification

Focused runtime tests cover both required-context loss and legacy omission disclosure. They verify
permanent execution refusal, fresh identities/context after reconciliation, unchanged original
history/artifact bytes, unrelated run preservation and exact runtime replay.

An actual-daemon test uses the same old-writer fixture with the configured durable grant. It verifies
authenticated recovery health, withheld context, proposal approval, same-generation application,
process restart, exact external command replay/conflict and normal runtime completion. A host test
verifies clean shutdown without workers or execution materializations, followed by normal startup.

The Windows MSVC / pinned Rust 1.95 gate passes formatting, workspace/all-target/all-feature
checking, 764 workspace tests plus 24 doctests, warning-denying Clippy/rustdoc, dependency
advisory/license/source checks, unused-dependency audit and duplicate-tree review. Five manual
tests remain ignored. The additional control-service mode-replay regression passes separately,
bringing executed ordinary tests to 765. Test discovery was rerun successfully after Windows
refused to replace executables still owned by a concurrently running evidence scenario.

Default/all-feature public API inventories were generated and reviewed for runtime, daemon and
control protocol. The new surface is the permanent runtime recovery state/method, daemon recovery
constructor and protocol health state. Protocol advances to 2.6; the durable storage, proposal,
command and configuration versions remain unchanged. Assignment 04's reviewed redb surface is
unchanged by 04A.

The operator, deterministic model and controller qualification scenarios pass against actual
binaries. Final binaries are rebuilt and exercised after compilation completes to avoid Windows
executable-lock races. Gate logs, scenario reports and binary hashes live under
`target/assignment-04a`; API inventories live under `target/public-api/assignment-04a`.
No real external provider is qualified. The existing duplicate `syn` version warning remains;
no dependency policy failure or unused dependency was found.

## Limits

Recovery controls do not repair arbitrary corruption, migrate schemas, replace the run's retained
grant, reset accounts or infer outcomes of entered external effects. Existing safe-restart rules
support side-effect-free/read-only work. Restored copies keep their execution guards until explicit
single-generation activation. Every remaining active blocker is checked again at normal startup.
No deployment, commit, push or external-provider call is part of this assignment.
