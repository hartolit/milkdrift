# ADR 0033: Explicit controller qualification and account inspection

Status: Accepted, including explicit activation for the qualified bounded operating scope.

## Decision

Use the existing control-owned lifecycle in the daemon composition before runtime recovery. Keep
`runtime.controller_activation` default-disabled. `qualification` requires a non-default development
build feature; `enabled` explicitly installs the same lifecycle in ordinary builds. The integrated
acceptance review closes the finite prerequisites recorded in [status](../product/status.md).
No live installation is enabled by a source change or development test.

An active account binding requires a lifecycle during recovery, including unmarked descendants.
A marked task cannot enter before account establishment. The one-shot installation API, existing
transactional reservations, immutable lineage, and conservative unknown-usage rules remain owners.
The child-creation transaction likewise refuses unaccounted children of marked revisions,
including ordinary subworkflows preceding activation. Account establishment in that same
transaction permits the normal first controller child and binds it atomically.
The requested output limit alone does not establish an enforceable envelope. Endpoint schema 2
adds operator-owned billing and token contracts. Supported fresh text requests count the complete
prepared UTF-8 body under an attested byte-BPE/template contract and select the server's total-output
parameter. Unknown or unsupported contracts still refuse controlled entry. Completion uses the
same frozen generation and keeps raw provider usage separate from tariff-calculated accounting.

Controller policy schema 2 permits an absent currency only with zero monetary allowance. This
means no currency-bearing request may enter; it does not authorize unknown cost. Old policy and
endpoint schemas are explicitly refused, with no unbilled defaults. Redb internal document format
advances from 14 to 16 (physical schema 11), refusing older stores; no migration is claimed. Existing
event version 3 keeps the same owner and tagged event shape: historical currency strings retain
their meaning, while current account budgets may encode null. Account readers validate that null
cannot carry a positive monetary budget. No existing event or historical decision is rewritten.

Admission envelopes now explicitly declare logical `model_tokens`; the shared account refuses
bounded quantities with unspecified units. Missing historical envelope units remain unknown and
re-encode without a new unit field. Format 16 prevents reopening a format-15 account under the new
admission rule. This preserves historical decoding without silently relabelling old quantities.
The unit is a capability-owned adapter contract, not workflow-controlled configuration. No token
conversion, GPU-work meter, or second account is introduced.

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

Deterministic actual-binary evidence exercises explicit enablement, disabled-start/recovery refusal,
and the same reservations, approval and reconciliation paths as the accepted bounded local external
controller loop. This supports the recorded operating scope, not arbitrary providers or traffic
capacity. [Status](../product/status.md) owns that decision and its limits; the
[evidence guide](../development/verification-evidence.md) owns reproducible verification.
