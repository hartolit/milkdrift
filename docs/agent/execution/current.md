# Current execution context

**Reviewed baseline:** `797ba0f` plus the current Phase 8 working tree
**Current target:** Phase 9 — simplify the architecture using integration evidence
**Entry state:** Phase 8 is complete and canonically validated
**Gate state:** the canonical full locked gate passes on the exact uncommitted Phase 8 working tree
**Canonical plan:** [execution-plan.md](execution-plan.md)
**Current product truth:** [project implementation status](../../project/implementation-status.md)
**Phase 8 decision:** [ADR-0012](../decisions/0012-local-native-composition.md)
**Phase 8 evidence:** [execution history](history.md#phase-8--gguf-parity-and-native-composition-evidence)

This file is the derived dense handoff for Phase 9. The execution plan owns roadmap and acceptance criteria, implementation status owns current support and validation provenance, accepted ADRs own architectural decisions, and history owns closed-phase evidence. Do not rewrite the execution plan from this handoff or treat repeated facts here as a second authority.

## Validated Phase 9 entry

The Phase 8 working tree based on `797ba0f90b3eac154fe44ec871f4c7bf755a06ef` passed `cargo run --locked --bin llm-app -- verify`, including a rerun after the final validation-status updates. Phase 9 may use this uncommitted working tree as its validated input; a later commit changes provenance and requires its own exact-tree evidence.

## Immediate objective

Use the now-integrated Candle/GGUF product path to simplify only boundaries for which there is evidence:

```text
validated Phase 8 baseline
→ replace purity rules with a reviewed acyclic dependency graph
→ narrow E1 without inventing a second coordinator
→ split oversized internals by invariant and responsibility
→ move repository-specific maintenance to xtask
→ make mandatory lint policy explicit and stable
```

Phase 9 is structural reconciliation, not a new product/backend phase. Preserve working CPU generation, immutable model compatibility, bounded output, deterministic cleanup, and the closed local product surface while reducing accidental coupling and oversized implementation units.

## Phase 9 work packages

### 9.1 — Replace the absolute F1 rule with an approved DAG

- Inventory the actual dependency needs of tokenization, context planning, sampling, task graph, prompt rendering, and workflows before changing policy.
- Keep the graph acyclic and review dependency direction explicitly.
- Permit one feature to depend on another only when the lower feature owns a stable concept.
- Move types into shared F0 only when they cross a real engine/backend boundary or have multiple stable consumers; do not use `domain-contracts` as a vocabulary escape hatch.
- Consider `foundation-types` / `inference-contracts` only if current `domain-contracts` demonstrably changes for unrelated reasons often enough to justify a split.
- Update architecture enforcement and fixtures with the accepted DAG rather than weakening validation globally.

### 9.2 — Narrow `application-runtime`

- Keep `corrective-workflow` outside E1 and do not re-export its internals.
- Make model lifecycle and generation the primary documented façade; retain conversation/context coordination only where it is application semantics.
- Apply ADR-0012: do not extract private local composition merely to lower E1's dependency count. Review extraction only if composition changes independently, gains another consumer, or obscures E1 semantics.
- Keep redb in E1 while it owns application preferences/catalogue state.
- Keep future stateful capability engines outside E1 once an independent lifecycle/reuse boundary is proven.
- Do not create `application-api` without a real transported consumer.

### 9.3 — Split oversized modules internally

- Review the plan's `task-graph/` and `inference-runtime/runtime/` candidates against current source responsibilities.
- Split by invariant and ownership—graph validation, artifact flow, attempt state, model registry, request registry, generation, transactions, operations, shutdown—not by arbitrary line count.
- Prefer `pub(crate)` or `pub(super)` helpers and preserve existing public APIs unless the work package provides evidence for a deliberate change.
- Keep each split reviewable and maintain focused tests around the moved invariant.

### 9.4 — Convert the maintenance runner to `xtask`

- Move repository-specific architecture validation and maintenance orchestration into `tools/xtask` with the virtual workspace and Cargo alias arrangement specified by the plan.
- Continue to use Cargo directly for ordinary formatting, checking, testing, Clippy, and simple benchmark selection.
- Preserve the architecture validator's fail-closed package roles and exact reviewed composition edges.
- Remove the misleading product-like root binary name only when replacement commands, documentation, and CI references are updated together.

### 9.5 — Review lint policy

- Keep strong mandatory lints.
- Decide explicitly which stable Clippy rules block CI and which exploratory/nursery rules should report without making toolchain upgrades arbitrarily fail.
- Do not broadly allow warnings or discard meaningful code to silence diagnostics.

## Phase 8 invariants to preserve

1. `application-runtime` remains one public frontend-neutral, non-generic E1 façade.
2. Candle/Hugging Face and GGUF/llama.cpp production composition remains isolated in private closed `local.rs` unless ADR-0012's review trigger is met.
3. The two native E0 paths remain concretely monomorphized and statically dispatched; no dynamic per-token backend interface is introduced.
4. Public selection remains closed to exactly Hugging Face Hub + Candle + Safetensors + CPU and local file + llama.cpp + GGUF + CPU. Hosted and peer targets are not local backend variants.
5. E1 remains single-model with one lifecycle, generation, conversation, context, output, unload, and shutdown state machine even though it owns two E0 workers.
6. Explicit shutdown requests termination and joins both E0 workers plus the Hub worker; bounded cleanup/exhaustion semantics remain intact.
7. GGUF prompt encoding and streaming decode come from the selected GGUF vocabulary, tied to the exact model bytes by SHA-256 before/after inspection and tokenizer load. Vocabulary-size coincidence is never compatibility evidence.
8. Resolved identity, tokenizer identity, inspected metadata, load source, E0 descriptor/capabilities, scalar/quantization values, vocabulary, and context limits must agree before publication.
9. Candle and GGUF continue to pass the same E0 generation contract and the same E1 direct-completion scenario, including release and unload.
10. Direct completion remains available for both products. Chat remains limited to immutable Hugging Face TinyLlama Chat v1 commit `fe8a4ea1ffedaf415f4da2f062534de366a451e6` with tokenizer `</s>` → ID 2; GGUF chat is not inferred.
11. Slint constructs only application-owned closed selections, derives Chat versus Direct completion from E1 evidence, and retains bounded frame-aligned output pulling without backend construction logic.
12. E0 owns native resources, token scheduling/sampling, cancellation boundaries, cleanup, and unload; response terminal semantics remain independent from later native cleanup/release.
13. Raw conversation provenance, turn-atomic context planning, bounded exact correction, pinned-content rules, regeneration/supersession, and in-memory-only history remain unchanged by structural work.
14. `corrective-workflow` remains an independent capability runtime with bounded service-port output and explicit artifact lifecycle.
15. redb remains E1-owned under the current application persistence semantics; no speculative local runtime or `application-api` is introduced.

## Explicit non-goals

- no GPU path, hosted-provider implementation, peer implementation, or browser/network transport;
- no new native backend or model architecture;
- no generic public application façade or plugin registry;
- no local-runtime extraction without ADR-0012 review evidence;
- no speculative `application-api`;
- no `domain-contracts` split without measured change-coupling evidence;
- no module splitting by line count alone;
- no weakening lifecycle, capacity, backend-contract, immutable-identity, or architecture enforcement to make a refactor easier;
- no performance optimization program before Phase 10;
- no rewrite of the canonical execution plan.

## Phase 9 acceptance criteria

- Architecture rules describe a real reviewed DAG rather than a purity diagram.
- `domain-contracts` has a clear inclusion rule.
- E1 has a narrow, coherent public API while ADR-0012 remains respected or is explicitly superseded with evidence.
- Large modules are split by invariant/responsibility rather than arbitrary size.
- `cargo xtask architecture` enforces the resulting policy.
- Ordinary Cargo commands are no longer needlessly reimplemented.
- Phase 8 shared Candle/GGUF E0 and E1 behavior remains green.

## Validation and recording rule

Start Phase 9 from the validated Phase 8 record above. During work, validate the narrow package or architecture slice first, then run the current canonical full gate on the exact resulting tree. Before the xtask migration, that command remains:

```sh
cargo run --locked --bin llm-app -- verify
```

After work package 9.4 establishes and documents the replacement, use the canonical xtask verification command defined by the resulting repository. Record exact commit/working-tree provenance in implementation status and append Phase 9 evidence to history; keep this file as the mutable next-work handoff rather than the evidence archive.
