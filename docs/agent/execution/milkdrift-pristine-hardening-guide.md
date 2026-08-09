# Milkdrift pristine-state hardening guide

## Purpose

This is a post–Phase 12 hardening program, not another numbered product phase and not a collection of cosmetic cleanup tasks.

Phase 12 established valuable transactional loading and mixed-dtype support, but the closure review identified several decisions that should be corrected before they become permanent foundations:

- model artifacts are still passed around primarily as filesystem paths;
- preparation hashes every tensor payload and materialization reads and hashes it again;
- execution precision can be influenced by tensors the backend does not use;
- supported but unused tensors are converted and transferred unnecessarily;
- the Candle loader has become a 1,700+ line subsystem hidden in one module;
- the generic loading contract may conflate drop-safe preparation with ownership-bearing failed materialization;
- E1 retains overlapping cleanup state machines;
- the current task graph and corrective workflow encode domain-specific flow assumptions that conflict with the operator-programmable workflow vision;
- the canonical GitHub quality job builds several large profiles into retained target trees and exhausted the hosted runner disk;
- current documentation contains stale CUDA workflow facts and live `llm-app` names.

The objective is not merely to make CI green. The objective is to leave a coherent core that future workflow, workspace, plugin, provider, and peer implementations can build on without inheriting avoidable debt.

## Execution order

Run these prompts sequentially:

1. `milkdrift-pristine-model-artifact-trust.md`
2. `milkdrift-pristine-candle-loading-subsystem.md`
3. `milkdrift-pristine-runtime-api-boundary.md`
4. `milkdrift-pristine-orchestration-foundations.md`
5. `milkdrift-pristine-repository-closure.md`

Do not run them concurrently. Each prompt is expected to start from the committed result of the previous one.

The split is based on ownership rather than individual tasks:

| Prompt | Primary ownership |
|---|---|
| Model artifact trust | artifact acquisition, immutable identity, verified source handoff |
| Candle loading subsystem | Safetensors inspection, dtype policy, materialization, memory planning, cleanup |
| Runtime/API boundary | portable loading contracts, E0 ownership, E1 translation, persistence-facing semantics |
| Orchestration foundations | portable task graph and corrective workflow incubation |
| Repository closure | workspace shape, publishability, frontend boundary, CI, tooling, evidence, documentation |

## Shared non-negotiable standards

Every agent must preserve these rules:

- Work directly in the local checkout. Do not provide a patch or code block as the deliverable.
- Read the canonical repository documents named in the prompt before editing.
- Preserve unrelated user changes. Do not reset, discard, or rewrite history.
- Do not push.
- Do not add project-authored unsafe code.
- Keep CPU mandatory and default. CUDA remains explicit and non-default with no automatic CPU fallback.
- Keep tensor/token hot paths statically dispatched and bounded.
- Keep frontends thin and replaceable.
- Do not leak Candle, Hugging Face, redb, Slint, filesystem, or transport types into portable contracts without a proven architectural reason.
- Do not create “temporary” compatibility paths that become permanent parallel APIs.
- Do not retain an inferior algorithm merely because existing tests encode it. Correct the algorithm and update the tests.
- Prefer explicit ownership, typestate, checked arithmetic, bounded resource use, and fail-closed behavior.
- Avoid speculative generic frameworks. Generalize only where at least two real consumers or a durable boundary justify it.
- Remove duplicated or obsolete code rather than preserving it behind deprecation layers when there are no external consumers.
- Add no TODO/FIXME markers for work identified by the prompt. Complete it or report a concrete blocker.
- Keep execution-history material historically accurate; update current-state authorities separately.

## Commit and handoff discipline

Each prompt should end with:

1. focused tests and checks for its owned area;
2. formatting and `git diff --check`;
3. one coherent local commit with a descriptive message;
4. a concise handoff containing:
   - commit and tree identities;
   - durable contract changes;
   - tests executed and results;
   - any genuine external-evidence gap;
   - no offer to push.

Do not spend tokens reproducing long command logs in the final response. The code, tests, and commit are the deliverable.

## Program-level acceptance criteria

The repository is not pristine until all of the following are true:

- model loading consumes verified immutable artifact identities rather than trusting raw paths;
- a normal load performs no redundant full payload verification pass;
- only required tensors are materialized and transferred;
- full observed artifact dtype evidence remains distinct from required execution dtype evidence;
- execution precision cannot be changed by an unused tensor;
- exact final and peak memory plans reflect the actual loading algorithm;
- partial native ownership remains explicitly retained, retryable, and accounted;
- the generic loading API distinguishes safe preparation from failed-materialization ownership cleanly;
- E1 contains no duplicate conversion policy and no overlapping cleanup trackers representing the same lower ownership;
- portable orchestration primitives do not hardcode one corrective flow or local-model selection policy;
- the corrective flow is represented as a template/example over general primitives or is explicitly quarantined as an experiment;
- the hosted quality workflow cannot exhaust disk by retaining multiple complete target profiles;
- portable-target checks execute in isolated, bounded jobs;
- benchmark compilation is limited to actual benchmark packages/targets;
- all current-state documentation reflects the successful Phase 12 GitHub CUDA run and the disk-exhausted quality run accurately;
- live product names, application title, tooling help, and data paths use Milkdrift, with safe migration from legacy paths;
- the engine/public crate boundary and optional application host are obvious from the workspace layout and README;
- the canonical local and GitHub validation paths are green.
