# Milkdrift controller-admission execution prompts

These prompts resolve ADR 0027 without splitting one architectural boundary into partially adopted stages.

## Execution order

1. Give a fresh implementation agent the repository, `00-controller-admission-contract.md`, and `01-implement-controller-admission.md`.
2. Apply and review that result.
3. Give a different fresh agent the resulting repository, `00-controller-admission-contract.md`, and `02-independent-closure-and-activation.md`.

The shared contract is task-specific guidance, not a new repository source of truth. Do not commit these prompt files or copy them into project documentation.

Pass 1 implements the complete accounting boundary but deliberately leaves production controller activation disabled. Pass 2 independently audits and repairs the result, runs the required hostile and longevity evidence, and installs the lifecycle in the daemon only when every ADR 0027 reconsideration trigger is satisfied. If qualifying external evidence cannot run because operator resources are absent, the second pass must leave activation closed and report that exact remaining blocker rather than weakening the gate.
