# Milkdrift pre-UI execution contract

**Purpose:** Shared task contract for the execution prompts in this package. The repository remains the source of truth. Do not commit this file, copy it into project documentation, or create a competing design document from it.

Use this contract together with exactly one numbered execution prompt. Each numbered pass should start in a fresh agent context unless the prompt explicitly says otherwise.

## 1. Required reading

Before changing code, read the current versions of:

1. `AGENTS.md`;
2. `docs/development/engineering-rules.md`;
3. `docs/product/vision.md`;
4. `docs/architecture.md`;
5. `docs/product/status.md`;
6. `docs/product/roadmap.md`;
7. `docs/development/workflow.md`;
8. `docs/development/verification-evidence.md`;
9. `docs/development/codebase-audit.md`;
10. `docs/reference/public-api-policy.md`;
11. every ADR, guide, source module, test, fixture, configuration path, and composition root relevant to the numbered pass.

Inspect the current Git head, working tree, recent history, and current CI state. The prompts name known evidence from the 2026-09-01 checkout, but those observations are regression targets rather than permission to assume the source is unchanged.

Preserve unrelated operator or agent changes. Do not reset, clean, discard, or rewrite work outside the pass.

## 2. This is an implementation task

Do not stop after producing an audit, plan, TODO list, design note, or documentation-only patch. Reproduce the problem, identify its owner and complete migration scope, implement the coherent correction, migrate all applicable producers and consumers, remove the superseded path, and prove the resulting behavior.

If the current source has already corrected a named symptom, verify the complete intended invariant and continue through the remaining scope. Do not reintroduce an obsolete form merely to make the prompt appear applicable.

Do not optimize for the smallest diff. Optimize for the simple, complete codebase that remains after the pass.

## 3. Global architectural constraints

Preserve these invariants throughout every pass:

- Definition truth, execution truth, and control truth remain separate.
- Workflow definitions and revisions are immutable. Accepted history is append-only.
- Reconciliation changes only future work.
- Runtime is the sole owner of workflow execution state and transition meaning.
- Adapters report external observations; they do not mutate workflow state or invent a second scheduler.
- One concept has one semantic owner, one canonical representation, and one normal operation path.
- Every accepted command and externally visible effect has exact idempotency identity and conflict behavior.
- Pre-entry failure, known external entry, terminal evidence, cancellation acknowledgement, and post-entry uncertainty remain distinguishable.
- Uncertain external work remains uncertain until authorized evidence resolves it. Absence of an observation is not proof that an effect did not occur.
- Humans, AIs, services, the CLI, and peers continue through the same command and authority machinery.
- Context remains causally selected, bounded, authority-filtered, provenance-bearing, and derived from durable facts. Do not replace it with chronological transcript accumulation.
- Models remain external capabilities. Do not add model loading, weight management, tokenization, provider discovery, pricing catalogs, or a bundled inference runtime.
- The CLI remains a storage-free client and document adapter. It must not become a second runtime, persistence owner, authority evaluator, or workflow model.
- No GUI, TUI, browser client, graphical editor, UI framework, or presentation-owned semantics may be added in these passes.

## 4. Abstraction and package discipline

Apply `engineering-rules.md` literally:

- A crate must earn a real boundary. Otherwise use a private module.
- Do not create `common`, `core`, `types`, `framework`, `plugin`, `manager`, or generic resource/lifecycle crates to relocate uncertainty.
- Use the smallest suitable abstraction: function, function with closure, private trait, then public trait.
- A new abstraction is incomplete while equivalent implementations remain in its declared scope.
- Public APIs are commitments. Keep test helpers and implementation mechanics private or explicitly feature-gated.
- Do not retain obsolete aliases, readers, factories, fallbacks, compatibility drivers, or dual representations without an explicit supported contract.
- Do not solve a cycle by duplicating the same identity or semantic type in several packages.
- Do not move a type merely because its mechanics are generic. Move it only when another existing semantic owner is clearly correct and remains below all legitimate consumers.

## 5. Compatibility and durable data

Milkdrift is pre-1.0 at the Rust source level, but durable documents and wire protocols are explicit contracts.

When serialized meaning changes:

1. identify the exact durable or wire family;
2. decide whether the current version can truthfully accept the change;
3. bump only the affected version when required;
4. update bounded readers, canonical writers, digest domains, fixtures, refusal behavior, tests, references, and ADRs together;
5. delete superseded writers and unsupported development compatibility paths.

Never regenerate a fixture merely to silence a test. Review the semantic change first.

## 6. Evidence rules

Run focused tests while implementing, then the full local gate from `docs/development/workflow.md` when the pass is complete:

```sh
cargo fmt --all -- --check
cargo check --workspace --all-targets --all-features
cargo test --workspace --all-features --no-fail-fast
cargo clippy --workspace --all-targets --all-features -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --all-features --no-deps
cargo deny check
cargo machete
cargo tree --workspace --duplicates
cargo test --workspace --all-features -- --list
```

Run every focused, mutation, longevity, operational, hosted, or external-evidence lane required by the numbered prompt.

Evidence must be truthful:

- Do not describe a local Linux result as hosted Windows/macOS evidence.
- Do not describe fixture or mock mode as real interoperability.
- Do not claim a full gate passed when later commands were skipped after an earlier failure.
- Do not convert missing operator resources into a passing test or weaker acceptance rule.
- Generated evidence belongs under `target/` or another untracked operator directory, not source control.

A timing-sensitive or nondeterministic test is a defect to correct, not a candidate for `#[ignore]`, longer arbitrary sleeps, or repeated retries until green.

## 7. Required final report

At the end of each pass, report only:

1. the canonical owner and invariant established;
2. the superseded paths, APIs, types, or duplicated rules removed;
3. any durable/wire compatibility decision;
4. focused and full commands actually run, with exact results;
5. external or hosted evidence that could not run and why;
6. any remaining blocker that is genuinely outside the pass.

Do not claim completion while an old and new design both remain valid paths for the same responsibility.
