# ADR 0035: Authorized recovery controls before execution readiness

Status: Accepted for implementation by the user's explicit recovery assignment.

## Context

Offline inspection preserves unsafe retained context but cannot change a workflow. Ordinary
startup correctly refuses an unsafe active lease, leaving no HTTP path to submit a prospective
repair. The existing reconciliation owner already records cancellation before dispatch and reserves
replacement work. Its scheduler must recognize that cancellation as eligible for restart.

## Decision

The daemon offers an explicit `--recovery` mode. It opens the current store and ordinary
authentication/control owners, but installs no adapters, peer services, effect workers or execution
materialization owner. It maintains the durable clock, command/security records and receipt
archival. Health is live in the `recovery` state; execution readiness remains false.

Runtime has a permanent `RecoveryControls` startup state. It accepts existing authorized pause,
cancellation, revision-reconciliation and external-work-resolution commands, preserving exact
receipts and optimistic guards. It refuses automatic recovery, scheduling, effect claim/entry and
admission reopening. This is a command-mode restriction, not a relaxation of retained-context
validation. A new normally initialized handle is required to validate repaired active state.

Control selects `CancelAndRestartSafeWork` for live proposals submitted in this mode, suppresses
automatic application, and requires explicit approval for such proposal plans even after restart.
Existing side-effect, authority, controller-account and history rules still decide which plans can
apply. Cancellation of a supported unentered attempt releases its lease and preserves its original
request/artifacts; replacement work receives fresh identities and context under the adopted revision.
The scheduler consumes restart reservations after both terminal cancellation and cancellation
before dispatch. No event, storage or proposal schema changes are needed.

Recovery reads withhold context attachments and raw artifact content because an old context may
contain undisclosed omission identities. Offline preservation retains the original bytes. Other
reads use ordinary scoped authorization and bounds.

## Consequences

An operator can preserve, inspect, review, repair and resume supported blocked work within the
same generation. Ordinary startup remains fail-closed. There is no generic corruption repair,
manifest rewriting, schema migration, account reset or external-effect resolution inferred from
process exit. Restored-copy guards remain in force until the explicit single-generation activation
decision. Control protocol 2.6 adds the health state; strict older response readers require updating.

The [operating procedure](../operations/daemon.md#authorized-recovery-controls) explains the CLI
sequence. Runtime legacy fixtures and an actual-daemon proposal/restart scenario verify the same
generation's preserved history, fresh execution, authorization, approval and replay boundaries.
