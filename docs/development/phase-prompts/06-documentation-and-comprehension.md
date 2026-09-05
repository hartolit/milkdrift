# Pass 6 — Compress documentation and expose the final implementation map

Rewrite the active documentation so it teaches one finished pre-UI system instead of preserving design evolution, duplicated architecture, cleanup history, and evidence chronology.

Follow `00-pristine-readiness-contract.md` in full. This pass is not a prose-only excuse to avoid source cleanup; it runs after the prior implementation passes and must describe their exact result.

## Primary outcome

A new maintainer should be able to answer, without reading every file:

- what Milkdrift is and is not;
- what works now;
- what remains unsupported;
- which owner decides each important fact;
- how one command becomes one durable external effect;
- how to run the product from a fresh directory;
- where to begin reading source.

Each fact appears in one canonical document and other documents link to it rather than paraphrasing it.

## 1. Remove historical and generated documentation residue

Confirm that the committed phase-prompt package and stale codebase audit were deleted in pass 1. Remove any remaining:

- prompt or pass histories;
- audit snapshots whose findings are already fixed or canonicalized elsewhere;
- generated inventories and API reports;
- dated benchmark narratives that belong in CI artifacts or Git history;
- repeated implementation-status sections in guides;
- obsolete configuration/protocol/schema examples;
- references to test fixtures as operator setup.

Do not delete ADRs merely because their decision was superseded. Mark their status accurately and preserve historical rationale. Do not create a new cleanup report.

## 2. Give every canonical document one job

### `README.md`

Make it a concise product entry point and working headless quick start. It should include:

- one-paragraph product purpose;
- current maturity and major unsupported claims;
- the supported fresh-directory daemon/CLI setup;
- one ordinary process or model workflow pointer;
- a compact repository/source reading map;
- links to canonical detail.

Remove dense restatements of runtime, authority, peer, storage, and protocol architecture. Correct stale claims, including any statement that controller final-entry resource accounting is still unimplemented when the current status says it exists but is not yet externally qualified/activated.

### `docs/product/vision.md`

Keep enduring product intent, user experience, three truths, prospective editing, causal context, capabilities, authority, and success criteria. Move current package names, version numbers, implementation procedures, and detailed evidence mechanics to their proper owners. Avoid repeating the engineering rules.

### `docs/architecture.md`

Keep normative terminology, ownership, dependency direction, durability/effect/uncertainty semantics, compatibility, and the exact current logical-to-physical mapping. Replace long narrative repetition with explicit invariants, owner tables, and focused diagrams. Do not omit difficult semantics merely to shorten the file.

Add one concise executable-source map inside this document or `docs/README.md`, not a new competing overview. It must trace exact current source locations for:

1. command/authentication/admission;
2. scheduling/final entry/reporting;
3. journal/projection/recovery;
4. artifact publication/read;
5. reconciliation;
6. daemon startup/shutdown.

### `docs/product/status.md`

Keep only current implementation facts, limitations, exact current versions, and the latest qualified evidence state. Remove architecture tutorials, historical chronology, superseded measurements, and aspirational design. Claims must match current tests/CI and distinguish configured evidence from executed evidence.

### `docs/product/roadmap.md`

Keep a short ordered list of genuinely unfinished product slices. Cleanup that is complete belongs nowhere in the roadmap. Do not use “further polishing” or “future audit” as an indefinite item.

### Development and evidence docs

Keep executable commands, fixture ownership, evidence meaning, and qualification limits. Remove dated local-performance prose and repeated product/status explanations. Ensure all command names and ignored-test filters match current source.

### Reference and operation guides

Retain exact operator, wire, schema, and security details only. Consolidate repeated setup and lifecycle instructions through links to the canonical guide.

## 3. Reduce prose volume without dense compression

Measure words and lines before editing. Reduce the combined active non-ADR documentation—especially README, vision, architecture, status, workflow, and verification evidence—by at least 25 percent unless independently reviewed unique semantics prevent that exact figure.

Do not achieve reduction by:

- joining paragraphs into longer dense lines;
- deleting refusal/safety semantics;
- replacing precise rules with slogans;
- moving the same prose to a new file;
- using large tables as compressed duplicate narrative;
- hiding details solely in code comments.

A successful reduction lowers the number of facts a maintainer must reconcile, not merely the line count.

## 4. Make source learning deliberate

Add a short “learning the implementation” route to `docs/README.md` or the architecture document. It should guide the owner through a small sequence of real source paths and black-box commands:

```text
1. run the fresh-directory CLI scenario
2. trace one command request
3. trace one external attempt
4. trace one artifact
5. restart and trace recovery
6. apply one prospective revision
```

For each step, name the canonical package/module and the independent test that proves it. Do not create a tutorial that copies implementation code or another crate map.

## 5. Enforce documentation ownership

Strengthen repository checks so they verify:

- canonical documents exist and local links resolve;
- exact schema/protocol/version statements agree with source constants;
- prompt/pass-history paths are absent;
- README quick-start files are production examples, not test fixtures;
- current status does not contain pass diaries or stale claims;
- command examples parse against current binaries where practical;
- each maintained example is validated by the owning production reader.

Do not create brittle tests that match entire prose paragraphs.

## Required proof

Run:

- Markdown/link/reference checks in the repository;
- full formatting/check/test/clippy/rustdoc gates;
- repository contracts;
- every command/example validation introduced by the docs;
- a fresh-directory quick start through actual daemon and CLI binaries;
- exact fixture/version tests for any example or contract text changed.

Review the final documents as a new contributor and search for duplicated explanations of definition/execution/control truth, capability entry, persistence history, authority, and UI non-ownership.

## Completion threshold

This pass is complete only when:

- each canonical document has one clear purpose;
- prompt histories, stale audits, generated reports, and chronology residue are absent;
- the combined active documentation is materially smaller and easier to scan;
- no unique safety or product invariant was lost;
- README provides a real supported quick start;
- one concise source-learning map points to current code and independent tests;
- all facts agree with current executable behavior and evidence.
