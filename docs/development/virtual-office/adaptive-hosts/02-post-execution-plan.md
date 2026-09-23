# Post-02 execution plan — reusable managed Linux environments

Prepared 2026-09-23 against `6e09339` following the design review of assignment 02.
State: executed against `b3c698c`, 2026-09-23. Execution owner: Codex.
Completion and qualification evidence: [02 handoff](handoffs/02.md).

This is corrective work within [02](02-managed-linux-environments.md), to complete before advancing
to 03. The [02 handoff](handoffs/02.md) retains the implemented behavior and executed qualification.
The findings below describe the pre-correction implementation at `6e09339` and retain the assignment
criteria. Their implemented resolution and executed checks are in the handoff; the separately owned
hardware qualifications remain open.

## Intended result and scope

An operator can prepare a useful managed working environment, select an owned or attached model,
set appropriate operating budgets, and use the complete lifecycle without changing Rust code or
adopting Slotbook-specific application content. Slotbook remains the sprint's maintained example
and uses the same public configuration and execution paths as any other workload.

Keep the existing durable resource owner, scoped authority, use holds, recovery, preservation and
ownership-safe cleanup. Correct the Linux configuration and composition that surround them. Reuse
Podman, systemd and the existing model-provider boundary; do not introduce a recipe interpreter,
plugin registry, arbitrary engine/unit arguments, package manager, custom supervisor or new provider
family. A small second workload used to test reuse is not another sprint application.

Read [AGENTS.md](../../../../AGENTS.md), the canonical product and architecture documents in its
reading order, [implementation practice](../../practices/implementation.md),
[documentation practice](../../practices/documentation.md), [ADR 0039](../../../decisions/0039-managed-resource-ownership.md)
and the [verification policy](../../workflow.md#choose-verification-for-the-change). Inspect current
source and consumers before choosing replacement types or versions. This plan is scoped to the
demonstrated problems below, not general architectural cleanup.

## Findings at the original review

### 1. The example controls production behavior

[LinuxRecipe](../../../../adapters/managed-linux/src/recipe.rs) requires `family = "slotbook-v1"`.
[Worker preparation](../../../../adapters/managed-linux/src/worker.rs) writes Slotbook-specific
instructions and embeds the application brief from a development guide in the compiled adapter.
It also requires Git, Rust and C tools as part of platform verification. These decisions prevent
the same managed environment from serving a workload with different content or tools without
accepting irrelevant assumptions or changing production code.

Separate platform enforcement from example initialization and toolchain expectations. Keep the
Slotbook brief, initial files and selected tool image with the example. Use existing authorized
artifact/worker operations or the approved image for initialization where sufficient. If a new
initialization contract is necessary, justify that need, bind its exact inputs to the approved
generation, and preserve edited files on reapply. Do not add a general provisioning language.
Changing a development guide must not silently change the contents of an otherwise identical
approved installation.

### 2. Owned model identity is coupled to one test model

The owned server's alias is always `ornith`. That value is repeated in
[unit generation](../../../../adapters/managed-linux/src/units.rs),
[service verification](../../../../adapters/managed-linux/src/service.rs) and the
[provider profile](../../../../adapters/managed-linux/src/model.rs). The attached variant already
accepts an operator-selected model alias. An arbitrary approved GGUF can therefore be served under
a misleading identity, and changing the alias requires several source edits.

Give owned model identity one validated configuration owner and use it consistently for generated
arguments, verification, descriptors and requests. Preserve exact image/model provenance separately
from the API alias. Validation must permit legitimate model identifiers without allowing argument
or unit interpolation. Keep inference external and retain the provider's strict response mapping.
The documented per-request reasoning option remains a compatibility choice; this correction does
not authorize an automatic reasoning fallback or a new generation-policy subsystem.

### 3. Worker and service budgets are conflated

The recipe exposes one memory, CPU and PID budget. Both the worker and owned service receive those
limits independently. The memory field is described as shared, but no combined cgroup limit
enforces it. [Prerequisite diagnosis](../../../../adapters/managed-linux/src/platform.rs) compares
one copy of the memory amount with available host memory. An operator cannot give a build worker
and a larger inference service different budgets, and the current explanation can be mistaken for
an aggregate reservation.

Define separate worker and owned-service operating budgets and state exactly what each enforcement
boundary covers. Check simultaneous demand and observed host headroom coherently, including updates
and identical reapply while a service is already running. Do not double-count existing allocations
or describe a preflight memory observation as a guaranteed reservation. Any claimed combined limit
needs actual aggregate enforcement; otherwise describe and verify the independent limits. Treat
UM790 RAM as one physical pool, without adding an iGPU reservation to it. An attached endpoint's
resource policy remains with its external owner.

### 4. Hard-coded operating choices lack a clear owner or rationale

The recipe currently caps memory/model size at 128 GiB, CPU quota at 128 CPUs, worker duration at
one hour, context at 131,072 tokens and threads at 128. The managed provider constructs fixed
120-second request/idle deadlines and fixed payload limits. Unit generation fixes temporary storage,
service timeouts and GPU-layer arguments. Some choices constrain useful workloads, yet the operator
cannot change them through the recipe. Generic validation errors often conceal which field failed.

Review these values and their consumers rather than mechanically converting literals to constants.
For each retained restriction, determine whether it is a platform/protocol requirement, a product
safety ceiling, an operator choice, a default, or an example input. Document its actual reason and
consequence; do not invent a rationale after the fact. Derive values from existing validated inputs
where possible, delete unsupported restrictions, and expose only choices an operator needs to make.
Reuse existing provider configuration contracts where they own the meaning. Preserve finite
timeouts, output bounds, checked arithmetic and truthful timeout/overflow outcomes.

The same review must distinguish justified bounds from accidental assumptions. Bounded document
readers and private UID mappings serve real protection needs; they should have one coherent owner
across generation, prerequisite checks and verification. Host-specific probes, including the literal
`/run/user/1000` socket check, must derive the actual identity or test the relevant isolation property.
Tests must demonstrate the rule across different inputs, not merely repeat a chosen numeric value.

## Execution sequence

Complete these steps as one assignment. Internal checkpoints are not separate permission requests.

1. **Trace and settle the configuration boundary.** Follow each finding from recipe parsing through
   bootstrap, approved deployment, generated definitions, descriptors, execution and recovery.
   Inspect tests and actual consumers. Select the smallest typed design that separates workload
   content, model identity and resource policy. Resolve whether any aggregate budget is actually
   promised. Record durable decisions with their canonical owner; keep raw investigations under
   ignored `target/`. Do not preselect schema numbers from this plan.
2. **Remove example assumptions end to end.** Move Slotbook content/tool expectations to maintained
   inputs and normal operations. Remove the application-specific family gate unless a demonstrated
   mechanism distinction requires a replacement. Migrate setup, reapply and verification together,
   with no alternate privileged example path. A neutral workload must not receive Slotbook files
   or require a Rust toolchain merely to pass platform protection checks.
3. **Make model and resource choices coherent.** Implement the selected alias, budget and timeout
   contracts through both owned and attached paths where applicable. Use one source for generated
   settings and their verification. Give invalid input field-specific diagnostics and actionable
   prerequisite failures. Preserve bounds without silently clamping or weakening operator intent.
4. **Complete compatibility and operator use.** Update all constructors, daemon bootstrap/configuration,
   serialization, examples, fixtures, CLI checks and documentation affected by the final design.
   Follow the [public API policy](../../../reference/public-api-policy.md): unsupported development
   formats must refuse before effects; supported saved generations must keep their original meaning.
   Existing installations retain an inspection/preservation path. Do not rewrite stores, reset data
   or silently adopt resources. Explain the normal approve/prepare/use/reapply/remove path without
   requiring operators to discover hidden constants or manually patch many files.
5. **Verify reuse, enforcement and failure behavior.** Run the acceptance cases below through the
   production owners and binaries. Retain exact inputs and distinguish deterministic evidence from
   real host/model results. Fix findings within this responsibility before closeout.
6. **Review the result as an operator and maintainer.** Follow the maintained instructions from a
   fresh configuration, trace each remaining restriction to its owner, and search for superseded
   aliases, family checks, initialization paths and duplicated defaults. Update the existing 02
   handoff and sprint state with actual coverage and remaining qualification. Passing tests alone
   cannot close a design or usability gap identified by this review.

Likely owners are `adapters/managed-linux`, the existing provider configuration boundary,
`apps/daemon` configuration/bootstrap, maintained managed-Linux inputs, and their tests/docs.
Change shared lifecycle, authority or persistence contracts only where the correction requires it;
do not add a second owner for their existing rules.

## Acceptance and evidence

The final implementation must establish all of the following:

- Slotbook still works through the ordinary product path. A small unrelated workload using an
  approved image without the Git/Rust/C suite also prepares, runs, retains edited data across
  reapply/restart and removes safely. No production code changes or Slotbook files are needed for
  that workload. This checks reuse without adding another application-development assignment.
- A differently named owned model works through generated configuration, readiness/inference
  checks and normal provider invocation. A second approved model input can be selected through
  configuration alone. Malformed aliases and injection attempts refuse before effects. Report
  separately any model combinations that were exercised only deterministically.
- Different worker and service budgets are represented, generated and observed correctly. Real
  Linux tests verify actual limits for both while they coexist. Exercise insufficient headroom,
  reapply and an authorized update without treating two independent caps as one shared reservation.
- Legitimate long-running work can select a suitable finite deadline through supported configuration.
  Deterministic tests use short varied deadlines to prove expiry, cancellation and retained
  uncertainty; acceptance need not sleep for an hour. Oversized input/output, arithmetic overflow
  and invalid limits still refuse or fail truthfully at their owning boundary.
- The normal CLI/API path covers initial setup, use, inspection, reapply, an incompatible change,
  recovery and preservation-aware removal. Retain busy-use refusal, exact replay, manager/worker
  separation, supervisor recovery, foreign-container collision protection, and attached-service
  preservation from 02. Re-run the affected physical lanes; deterministic tests cannot substitute
  for changed platform enforcement.
- Every remaining numeric restriction has an owner and a defensible purpose. Configuration controls
  real operator decisions, diagnostics identify the offending choice, and examples explain use
  without presenting arbitrary fixture values as established model or hardware facts.

Executable changes require the full [local gate](../../workflow.md#full-local-gate), relevant
focused suites, default/all-feature public API review, production-reader/CLI example checks and
`git diff --check`. Use the repository-pinned toolchain and report the actual build profile. Preserve
negative and passing evidence under ignored `target/adaptive-hosts/`; update canonical operations,
architecture, status, ADR and evidence documents according to what actually changed. Keep exact
commands and source identities in the existing handoff rather than creating a second status report.

The existing [hardware qualification issue](../whiteboard/issues/managed-linux-hardware-qualification.md)
continues to own managed Vulkan, UM790 pressure and reboot/power-loss gaps. This plan neither closes
them nor authorizes disrupting native model servers or changing host privileges. Coordinate access
to real model servers and retain their stated single-generation limits. New configuration must not
invalidate earlier evidence silently; identify which physical cases need to be repeated.

## Completion condition

Close this corrective assignment when the same narrow Linux mechanism supports the maintained
example and an unrelated workload through configuration and normal operations, the model and
resource policies are coherent, old conflicting paths are removed, and the behavior and operator
instructions satisfy the checks above. Leave any genuinely unavailable hardware qualification
explicitly open with its existing owner. Do not defer a known design, ownership or usability defect
to 06 merely because the original example passes.

Execution is complete for the corrective scope above. The [02 handoff](handoffs/02.md) records
configuration-only reuse, actual product paths, physical enforcement, the full local gate and the
remaining hardware qualification.
