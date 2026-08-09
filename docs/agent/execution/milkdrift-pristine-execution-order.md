# Milkdrift post-Phase-12 pristine-state execution order

## Purpose

This remediation program is not a minimal CI repair and is not a new product phase. It is a sequential hardening and restructuring pass over the completed Phase 12 implementation so the local execution foundation is correct, efficient, maintainable, and honest before workflow/workspace development begins.

The work is divided by ownership boundary rather than by small task:

1. **Artifact and tensor-loading subsystem** — Hugging Face artifact evidence, Safetensors inspection, scalar policy, source identity, memory planning, selective materialization, and Candle loader structure.
2. **Generic runtime ownership subsystem** — portable prepared-load contracts, E0 admission, retained ownership/accounting, cleanup, snapshots, and backend-independent fault injection.
3. **Application integration subsystem** — E1 artifact semantics, persistence, model state, cleanup visibility, and thin Slint projection.
4. **Repository infrastructure and project truth** — architecture enforcement, canonical verification, benchmarks/evidence, GitHub Actions, documentation, and exact support claims.
5. **Independent closure review** — whole-tree integration audit and clean-target validation after the four owned-area changes.

Run the prompts in that order. Do not run them concurrently.

## Why this order

The first prompt establishes the authoritative artifact and loading semantics. The second makes the generic runtime correctly own and account for those semantics. The third translates the completed lower boundary into application-facing state without duplicating policy. The fourth can then make CI, benchmarks, architecture validation, and documentation describe the actual final implementation. The last prompt acts as an independent reviewer rather than trusting the preceding agents' completion statements.

## Working-tree discipline

Each agent should:

- inspect the existing tree and prior commit before editing;
- preserve unrelated user work and never reset or discard changes;
- make one coherent commit only after its owned validation passes;
- not push;
- report the commit SHA and tree SHA, exact validation performed, and any evidence that could not be executed;
- leave the tree clean for the next prompt.

If the tree is unexpectedly dirty, the agent should identify and preserve the changes rather than hiding them in a stash or overwriting them.

## Program-wide invariants

All prompts share these requirements:

- No “good enough for now” implementation, deferred correctness TODO, or knowingly temporary compatibility rule.
- Optimize cold paths where the cost will scale with real model size, but preserve explicit ownership and source-integrity guarantees.
- No project-authored unsafe code.
- No silent CPU/device/target fallback.
- No backend- or Safetensors-specific types in portable workflow/domain layers without a demonstrated generic need.
- No frontend policy ownership.
- Exact declared, observed, required, planned, actual, retained, sampled, and historical facts must remain distinct.
- Tests must assert behavior and invariants, not implementation trivia or brittle test-name registries.
- Historical evidence must not be rewritten as evidence for a newer tree.
- Do not begin the future workflow/workspace implementation during this remediation.

## Files

- `milkdrift-pristine-01-artifact-loading.md`
- `milkdrift-pristine-02-runtime-ownership.md`
- `milkdrift-pristine-03-application-boundary.md`
- `milkdrift-pristine-04-infrastructure-truth.md`
- `milkdrift-pristine-05-independent-closure.md`
