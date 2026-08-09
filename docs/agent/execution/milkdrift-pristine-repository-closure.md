# Milkdrift repository, CI, public structure, evidence, and documentation closure

## Objective

Complete the pristine-state program by making the workspace shape, publishable engine boundary, optional frontend role, quality workflows, tooling, evidence, naming, and documentation agree with the corrected code.

This prompt is the final integration and project-truth owner. Do not reopen model-loading or orchestration algorithms unless validation reveals a concrete defect.

## Read first

Read:

- every commit/handoff from the preceding hardening prompts;
- `README.md` and `docs/vision.md`;
- `docs/project/architecture.md`, `workspace.md`, `dependency-policy.md`, `implementation-status.md`, `validation.md`, and `performance.md`;
- `docs/agent/execution/current.md`, `execution-plan.md`, and `history.md`;
- `.github/workflows/quality.yml` and `cuda-hardware.yml`;
- `tools/xtask` architecture/hygiene/verify code;
- root and package manifests;
- the desktop host path/title/data-directory code.

## Current GitHub evidence to reconcile

On commit `181a069ce81525e9c144fe8de051ced8e3c0b9d7`:

- CUDA hardware run `31281013243` completed successfully on the registered RTX 5070 Ti runner.
- Quality run `31281013257` failed after the canonical native gate had succeeded because the hosted runner retained a large clean target, compiled the full workspace bench profile, reached roughly 49 MB free space, and then failed the subsequent WASM check with `No space left on device`.

Do not describe that run as a WASM source incompatibility. Preserve the local Phase 12 evidence and add the later GitHub CUDA evidence to current-state authorities. Keep historical statements historically scoped.

## Workspace and public product boundary

Make the engine-centered monorepo structure obvious.

Review and implement the cleanest layout for:

```text
portable core/domain crates
hosted runtime crates
backend/artifact/storage adapters
optional application services
reference applications
benchmarks
tools
experiments/templates
```

Requirements:

- applications are visibly outer consumers, preferably under a top-level `apps/` root rather than hidden among publishable engine crates;
- the Slint host remains optional and non-published;
- the primary engine/core packages have clear Milkdrift package names and metadata suitable for eventual crates.io publication;
- package descriptions, repository/homepage/documentation fields, license, categories, keywords, and `publish` policy are deliberate;
- portable `no_std` foundations and hosted `std` runtimes are not conflated;
- do not create a generic `crates/core/` dumping ground; use it only if its contents are strictly portable, vendor-neutral foundations;
- application convenience services are not presented as the only public engine API;
- experiments/templates are clearly classified and cannot silently become production dependencies.

Because there are no established external consumers, prefer one clean API and package identity over compatibility aliases.

## Live naming and data migration

Remove live `llm-app` identity from:

- xtask help;
- UI title;
- application data directory;
- test temporary names where they represent current product identity;
- current documentation diagrams and descriptions.

Preserve historical names only in explicitly historical records.

Implement a safe one-time data migration from the legacy application directory to the Milkdrift directory:

- never overwrite a populated new location silently;
- define conflict behavior;
- make migration idempotent;
- test missing, legacy-only, new-only, and conflicting states;
- do not lose preferences or model catalogue data.

## Quality workflow redesign

Fix the disk-exhaustion cause structurally rather than adding one `rm -rf` at the failing line.

Design jobs around resource ownership:

- portable WASM and embedded checks run in their own minimal job/target and should execute early;
- native workspace check/test/Clippy/docs use a bounded target lifecycle;
- benchmark compilation is restricted to actual benchmark packages and declared benchmark targets, not `cargo bench --workspace --no-run` across every package;
- clean-target validation still proves reproducibility;
- target directories are named `milkdrift-*`, removed with `if: always()` or equivalent cleanup, and never duplicated accidentally;
- disk use is observed before/after heavy steps so a regression fails with a clear diagnostic before LLVM crashes;
- scheduled nursery and external-link jobs remain non-blocking only where intentionally exploratory;
- default CI remains download-free for model artifacts;
- CUDA hardware stays isolated, trusted, feature-exact, and fail-closed.

Review `cargo xtask verify`. The canonical developer gate should be complete but not wastefully compile every package in a release bench profile. Keep ordinary Cargo commands transparent; do not turn xtask into a hidden CI scripting framework.

## Architecture validator and hygiene

Simplify architecture enforcement where it mirrors the entire current dependency graph.

Keep durable rules:

- dependency direction by layer;
- portable crates cannot depend on native infrastructure;
- adapters cannot depend upward on runtimes/apps;
- apps consume public application/runtime boundaries;
- benchmarks are outer observers;
- CUDA feature forwarding is explicit and non-default;
- unknown workspace roots/roles fail closed.

Require exact review only for exceptional or high-risk edges. Do not require a hand-maintained duplicate registry for every ordinary legal dependency. Update tests so the validator remains strict without becoming a second Cargo manifest.

## Documentation spine

Reconcile and reduce documentation around clear ownership:

1. root README — identity, differentiation, current implementation, architecture summary, roadmap;
2. `docs/vision.md` — authentic durable purpose and values;
3. project architecture — responsibility and dependency boundaries;
4. operation guide — end-to-end load/generate/release flow;
5. API/integration guide — portable core, hosted runtime, adapters, application services;
6. implementation status — exact support/evidence matrix;
7. validation — repeatable commands;
8. performance — measurements and limitations;
9. ADRs — durable decisions;
10. agent execution documents — history and temporary work, not public identity.

Remove redundant current-state prose and stale duplicated matrices. Archive or clearly mark superseded Phase 12 prompts; do not make contributors read all of them to understand the project.

Update current authorities for the corrected artifact, loader, runtime, and orchestration contracts. Preserve old schema reports and historical run claims under their original meaning.

## Final validation

Run the complete corrected local gate from absent target directories.

At minimum:

```text
cargo xtask architecture
cargo xtask hygiene
cargo xtask verify
cargo check --locked --target wasm32-unknown-unknown --lib <portable packages>
cargo check --locked --target thumbv7em-none-eabihf --lib <portable packages>
cargo deny --workspace --locked check advisories bans licenses sources
lychee --config lychee.toml --offline '**/*.md'
cargo tree -d --locked
git diff --check
git status --short
```

Also compile the exact CUDA graph and test targets. Execute hardware tests only on the accepted guarded runner.

Measure and report target-directory sizes for the redesigned local/CI command sequence. The hosted quality design must fit comfortably rather than passing with only a few megabytes free.

## Closure criteria

Do not declare completion unless:

- every prior hardening commit is integrated;
- all known findings in the hardening guide are resolved or disproved with tests and documented reasoning;
- current docs contain no stale “Phase 12 self-hosted workflow unrun” claim;
- CI cannot recreate the observed target duplication;
- no live product path/title/help text says `llm-app`;
- workspace layout communicates engine-first scope;
- publishability/no_std claims are precise;
- the worktree is clean after one final coherent commit.

Do not push. Provide a concise final closure report with commit/tree identities, exact validation results, and any external evidence that genuinely remains unavailable.
