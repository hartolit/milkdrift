# Milkdrift pre-UI execution prompt sequence

These prompts extend the current repository without authorizing graphical UI work. They are designed for sequential execution with a fresh coding-agent context for each pass.

Do not commit this prompt package into Milkdrift. The repository’s canonical documents remain authoritative.

## Execution order

### 1. Restore the baseline

Give a fresh agent:

- `00-shared-execution-contract.md`
- `01-current-head-integrity-and-portability.md`

Apply and review the complete result before continuing.

### 2. Contract and public-surface contraction

Give a fresh agent the resulting repository plus:

- `00-shared-execution-contract.md`
- `02-contract-boundaries-and-public-surface.md`

This pass deliberately preserves blueprint/capability separation unless current dependency evidence proves a better existing owner. It guarantees concrete code cleanup regardless of that ownership conclusion.

### 3. Adapter conformance

Give a fresh agent the resulting repository plus:

- `00-shared-execution-contract.md`
- `03-capability-adapter-conformance.md`

Apply and review the result.

### 4. Execute the existing controller-admission package

Use the prompts already present in the repository:

1. Give one fresh implementation agent:
   - `docs/development/phase-prompts/00-controller-admission-contract.md`
   - `docs/development/phase-prompts/01-implement-controller-admission.md`
2. Apply and review that result.
3. Give a different fresh agent:
   - `docs/development/phase-prompts/00-controller-admission-contract.md`
   - `docs/development/phase-prompts/02-independent-closure-and-activation.md`

Activation must remain closed when the existing ADR/evidence gate is not satisfied. Do not weaken that result to keep the sequence moving.

If this controller package has already been executed on the current branch, skip this step and require Pass 4 to inspect the implementation that actually exists.

### 5. Cohesion after semantic changes

Give a fresh agent:

- `00-shared-execution-contract.md`
- `04-cohesion-and-owner-structure.md`

This is intentionally after controller work so large files are organized around the final current responsibilities rather than refactored twice.

### 6. Complete the headless product surface

Give a fresh agent:

- `00-shared-execution-contract.md`
- `05-thin-cli-headless-product-surface.md`

The CLI must become comprehensive and automation-safe without owning workflow semantics or storage.

### 7. Run local model dogfood

After a small loopback OpenAI-compatible server is available, give a fresh agent:

- `00-shared-execution-contract.md`
- `06-local-model-and-external-effect-dogfood.md`

The agent must not install or manage the server from Milkdrift. When the real endpoint is absent, deterministic implementation and tests may proceed, but real evidence remains explicitly blocked.

### 8. Independent closure

Give a final fresh agent that did not implement the earlier passes:

- `00-shared-execution-contract.md`
- `07-independent-pre-ui-closure.md`

This pass independently repairs findings and runs the full local, mutation, operational, longevity, hosted-as-available, and external-as-available evidence campaign.

## Handoff discipline

- Use the repository produced by the prior pass, not a parallel branch with stale assumptions.
- Review each result before starting the next pass.
- Preserve one commit or otherwise clearly isolated diff per pass so regressions can be attributed.
- Do not give later agents the earlier agent’s persuasive summary as evidence; give them source, tests, logs, and the next prompt.
- Never accept documentation-only completion for an implementation pass.
