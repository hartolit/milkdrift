# ADR 0033: Explicit controller qualification and account inspection

Status: Accepted for implementation; production operational qualification remains blocked.

## Decision

Use the existing control-owned lifecycle in the daemon composition before runtime recovery. Keep
`runtime.controller_activation` default-disabled. `qualification` requires a non-default development
build feature; `enabled` requests production activation and is refused until the existing evidence
prerequisites are met. No live installation is enabled by a source change or development test.

An active account binding requires a lifecycle during recovery, including unmarked descendants.
A marked task cannot enter before account establishment. The one-shot installation API, existing
transactional reservations, immutable lineage, and conservative unknown-usage rules remain owners.
The child-creation transaction likewise refuses unaccounted children of marked revisions,
including ordinary subworkflows preceding activation. Account establishment in that same
transaction permits the normal first controller child and binds it atomically.
The model adapter's requested output limit does not establish enforceable input/output/cost bounds.
It therefore still refuses controlled model entry; this negative evidence cannot qualify production.

Protocol 2.5 adds cumulative accounting to authorized run/controller reads and durable terminal
detail to attempt reads. Committed equals settled plus outstanding; blocked accounts expose no
spendable remaining allowance. Ordinary runs explicitly report inactive accounting. Descendant
inspection also authorizes the originating controller run because totals cover the whole account.
Old responses lacking the accounting field mean unavailable, not zero use or unlimited allowance.
This is an additive control protocol change; CLI JSON and daemon configuration versions stay at
2 and 9 respectively. The optional configuration field preserves disabled behavior for old files.

Prospective reconciliation of an attached child must not invalidate its parent link. Check the
immutable creation event against the parent's original revision pin while allowing an authorized
current revision to differ. Inputs, workspace, authority, and account linkage remain constrained.

## Consequences

Deterministic actual-binary evidence can exercise the real composition without claiming that a
fixture or an ordinary external model run qualifies production. The explicit refusal is retained
until a bounded authorized external controller loop and the other current prerequisites have been
accepted. [Status](../product/status.md) owns that decision; the
[evidence guide](../development/verification-evidence.md) owns reproducible verification.
