# ADR 0027: Production continuous controllers require final-entry resource reservations

- Status: accepted
- Date: 2026-09-01

## Context

ADR 0026 defined one durable controller policy, projection-derived progress, and atomic cycle
assessment. That closes restart-reset and duplicate-assessment failures, but projection-time totals
do not reserve every cumulative process, model, artifact, cost, and usage ceiling beside the final
external-entry decision. Concurrent entries, retries, cancellation, and delayed usage observations
can therefore pass individually while their combined eventual total exceeds a controller policy.
A hard resource ceiling cannot depend only on a prior read of current projection state.

## Decision

The production daemon leaves `ControllerLifecycleOwner` uninstalled. A revision marked with the
controller-policy extension fails closed at activation rather than running under a ceiling that the
host cannot enforce atomically. The library owner, durable assessments, read models, commands, and
focused integration tests remain available to develop and validate the contract; presets grant
ordinary operations and do not enable a hidden runtime role.

Production support requires one durable reservation/accounting owner adjacent to the runtime's
final exact-generation adapter-entry boundary. It must atomically account for concurrent admitted
work, retries, terminal usage, artifact publication, cancellation, uncertainty, restart, and
legacy conservative facts without trusting provider or model claims. The controller cycle
assessment may consume that truth but cannot duplicate it.

## Rejected alternatives

- Install the existing lifecycle and describe its totals as advisory, because controller policy
  exposes them as hard autonomy bounds.
- Serialize every controller to one worker, because delayed external outcomes and retries still
  cross cycle boundaries and serialization would not own artifact or usage commits.
- Reserve worst-case values only in controller code, because adapter entry and artifact
  publication are owned by other boundaries and could disagree.
- Remove the controller contracts, because their durable policy, attribution, assessment, and
  recovery semantics remain useful foundations for the missing admission owner.

## Consequences

The daemon supports ordinary human, service, process, and model proposals through the shared
authorized control path, but does not support continuous autonomous controller execution. A CLI
continue command cannot install the lifecycle or bypass the refusal. Documentation and status must
distinguish the tested library integration from production support.

## Reconsideration triggers

Install the lifecycle in the daemon only after the final-entry reservation owner has hostile tests
for concurrent exact-bound admission, retry, cancellation, unknown usage, artifact publication,
uncertainty, compaction, and restart, plus a current longevity and external-evidence run. UI demand
or a provider-side quota is not sufficient.
