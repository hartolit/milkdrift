# Assignment 03 handoff

Owner: Codex. Base: `741b230`; implementation and review corrections are in the commit containing
this handoff. Local implementation and verification are complete. No live host configuration has
changed. Production activation is
default-disabled and an `enabled` request is refused; the main assignment is not operationally
accepted. Assignments 04–08 have not started.

## Implemented boundary

The actual daemon installs the single control-owned lifecycle before recovery through explicit
`qualification` configuration and the non-default `controller-qualification` build feature.
Disabled recovery refuses active account bindings, including unmarked descendants; marked work
cannot enter before account establishment. The existing one-shot API and account owner remain.
Review caught an unmarked subworkflow preceding activation entering without an account. Child
creation now refuses that route in the existing account transaction; activation and its first
accounted child can still commit together. The regression fails on the original implementation
and when the new marker check is inverted. Actual binaries refuse it in both activation modes.

Authorized run/controller reads expose the canonical account, committed usage, reservations,
blocked state, and knowable remaining allowance. Ordinary runs explicitly report inactive.
A descendant read also requires inspection authority over the controller origin. Per-request
authority, attempt limits, worker capacity, cumulative accounting, and retention remain distinct.

The installed scenario found and now covers two runtime defects: unrelated sibling journal writes
could starve final entry after preparation, and an attached child's authorized prospective revision
could invalidate its parent creation pin. Final entry refreshes unrelated writes while checking its
own exact ticket; child linkage checks the immutable creation event. Proposal assessment selects
the declared controller node without misreading sibling nodes as controller policies.

Canonical explanations are in [ADR 0033](../../../decisions/0033-explicit-controller-qualification.md),
[daemon operation](../../../operations/daemon.md#controller-activation),
[authority](../../../operations/authority.md), [architecture](../../../architecture.md), and
[control API](../../../reference/control-api.md). Obsolete unconditional uninstalled statements
are replaced with the explicit development/production distinction. No replacement ledger or loop
framework was added.

## Finite activation checklist

| Prerequisite | Current owner and evidence | Decision |
| --- | --- | --- |
| Concurrent final entry | Redb account transaction contracts; installed scenario's three descendants compete for two entries and retain two reservations before settlement | Installed scenario and full gate pass. |
| Crash/reopen, retry, uncertainty, late terminal | Persistence `missing_usage_blocks_and_late_evidence_settles_the_original_reservation_once`; control admission/recovery contracts; installed process crash after entry and lease expiry | Account and original reservation survive two reopens; disabled recovery refuses. Full gate passes. |
| Artifacts, lineage, compaction | Redb lineage, final-entry and artifact-charge contracts; control `controller_artifact_charge_is_exact_replay_safe_abort_safe_and_restart_durable`; installed revision/replay/compaction checks | One account survives repaired child revision, charged artifacts, cold receipt replay, and terminal restart. Full gate passes. |
| Unsupported/unknown usage | Persistence `unknown_currency_and_overflow_are_distinct_fail_closed_denials`, terminal-cost contracts, and the actual counting model fixture | Unknown input bounds refuse entry with zero fixture requests. Both supplied Bonsai profiles reach the same pre-entry refusal. |
| Authority and acceptance | Control activation/authority tests; installed separate controller/approver actors and revoked approval credential; deterministic model acceptance lane | Installed repair stays unentered until authorized approval; no self-approval or fabricated success. Full gate passes. |
| Mutation | Changed activation, account projection, origin authorization, recovery, final-entry and child-pin functions; review's child-creation guard | Retained 46 selected: 36 caught, five exact contract classifications, five ill-typed substitutions; the additional exact child-creation fault is caught. No unresolved result; this is not the full controller shard. |
| Longevity | Release control lifecycle/admission tests named below | Both pass in this checkout. |
| Operational and full gate | Repository workflow gate and three actual-binary lanes | The review full gate and all three binary lanes pass; installed report: `target/controller-review-scenario/report.json`. |
| Real external controller loop | Current model adapter hard admission envelope and the explicitly supplied endpoint/profiles | Blocked: unknown hard input/output/cost bounds prevent controlled model entry. No bounded real external loop is qualified. |

The deterministic loop retains failed work and Assignment 01 rejection, reads exact evidence,
submits repair through an ordinary workflow-control task, requires separate authorized approval,
prospectively repairs an unentered task, and independently verifies accepted remediation. Four
process entries exhaust the shared allowance; the repeat refuses another child and the root ends
in explicit failure. This is useful remediation followed by a truthful cumulative stop.

The full Windows/MSVC gate passes 737 workspace tests, 24 doctests, all 24 repository contracts,
formatting, checking, warning-denying Clippy/rustdoc, dependency audits, and test discovery. Five
manual longevity tests remain ignored in the ordinary gate; the two required controller release
lanes were executed separately and passed. Exact commands, source/binary hashes, and reports are
retained in `target/controller-review-verification.json`; [the evidence guide](../../verification-evidence.md)
owns the reproducible commands and scope. No independent operational acceptance is claimed.

Release commands executed successfully:

```sh
cargo test --release -p milkdrift-control --test control_service --all-features revision_and_lifecycle::release_controller_longevity_stops_once_across_checkpoints_and_restart -- --ignored --exact --nocapture
cargo test --release -p milkdrift-control --test control_service --all-features admission::release_controller_admission_longevity_turns_over_reservations_artifacts_and_restart -- --ignored --exact --nocapture
```

## API and compatibility review

Default/all-feature inventories for daemon, control, control-protocol, persistence, and runtime
are retained under `target/public-api/controller/`. `ControllerActivation` and its config field
are external configuration contracts; the development variant is refused without its feature.
`ControllerAccountingRead`, status/run fields, and `NotActivated` are control-owned application
contracts consumed by daemon reads. `remaining_allowance` is a persistence-owned workspace
contract consumed by that projector. Inspection implementation remains private.

Protocol 2.5 adds run accounting and bounded attempt terminal detail, consumed by daemon/CLI and
the installed evidence scenario. Legacy missing accounting decodes as unavailable (`null`), while
modern ordinary runs explicitly report inactive. Older strict readers accepting the new response
are not claimed. CLI JSON remains 2; daemon config remains 9 with an optional default-disabled
field. No event, blueprint, model, or account schema changed. Runtime test-only surfaces remain
separate from the default inventory; no new runtime export is introduced.

## Exact remaining external blocker

`http://127.0.0.1:1234/v1/models` lists `prism-ml/bonsai-27b` and `prism-ml/bonsai-27b:2`.
The isolated profiles register and prepare, then fail controlled admission on unknown hard input
units; neither request limits nor earlier ordinary model successes establish the missing hard
input/output/cost envelope. Production requires supported enforceable bounds plus an authorized,
pinned, bounded real controller loop with retained approval, remediation, acceptance, and restart
evidence. This assignment does not weaken unknown-usage policy or add a provider family to obtain it.
Thinking and other effective server settings remain unknown. Windows process-kill evidence does
not establish power-loss durability, escaped-descendant containment, or another platform.
