# Milkdrift pristine-readiness contract

## Mission

Contract and qualify the current Milkdrift implementation until the owner can learn and maintain one stable system instead of reverse-engineering accidental implementation history.

This is a cleanup and readiness program over the product that already exists. Do not broaden the product to make the cleanup appear useful. The expected result is less code, fewer public concepts, fewer dependency edges, fewer exceptional paths, and a stronger normal headless workflow.

Every numbered pass is an implementation pass. Inspect, modify, test, and leave a reviewable repository result. A report-only response, documentation-only patch, generated inventory, or list of recommendations is not completion.

## Governing material

Read in this order before editing:

1. `AGENTS.md`
2. `docs/product/vision.md`
3. `docs/architecture.md`
4. `docs/development/engineering-rules.md`
5. `docs/product/status.md`
6. `docs/product/roadmap.md`
7. `docs/development/workflow.md`
8. `docs/development/verification-evidence.md`
9. `docs/reference/public-api-policy.md`
10. relevant ADRs, references, manifests, source, tests, fixtures, CI workflows, and current Git history

When prose and executable behavior disagree, determine which side drifted. Do not silently choose the cheaper side. Preserve unrelated user work and do not reset, rebase, rewrite history, or copy these prompts into the repository.

## Scope freeze

Unless a numbered prompt explicitly requires it, do not add:

- a graphical or browser UI;
- a new provider family;
- a new workflow primitive;
- a plugin framework or generic service registry;
- provider discovery, pricing catalogs, tokenizers, or model management;
- storage migration or compatibility for unsupported development formats;
- peer discovery, NAT traversal, VPN, overlay, or coordinator behavior;
- another scheduler, event truth, authority path, context model, or persistence sidecar;
- a new crate merely to give a concept a name.

Do not enable production continuous controllers unless every existing documented activation condition is genuinely satisfied by current evidence.

## Required end state

### 1. One operational spine

The following path must be singular, traceable, and testable through public executables:

```text
operator input
  -> CLI presentation/parsing
  -> control client
  -> versioned control protocol
  -> daemon command/read owner
  -> authority decision
  -> runtime command and scheduler
  -> exact capability selection and final entry
  -> adapter execution and observations
  -> atomic durable facts
  -> projection/read model
  -> CLI inspection
```

For each transition, one source location owns the decision, one durable fact proves it, one failure vocabulary describes refusal, and one recovery rule governs restart. A second implementation used only by tests is not acceptable.

### 2. Proportionate ownership

Each semantic fact and lifecycle has one owner, one canonical representation, and one normal operation path. Package boundaries must satisfy the engineering-rule crate test. Private modules are preferred when a responsibility has no independent consumer, wire/durable contract, adapter implementation boundary, dependency isolation, or lifecycle owner.

Do not merge `blueprint`, `capability`, and `contracts` simply to reduce the package count. Current evidence suggests that:

- blueprint owns immutable definition truth;
- capability owns provider-neutral requirements, descriptors, exact resolution, invocation, observation, and cancellation contracts;
- contracts owns only cross-domain implementation mechanics with multiple real consumers.

Keep that division unless whole-workspace analysis proves a simpler dependency graph with the same semantic ownership. Do not respond to questionable primitive ownership by creating a broad `core`, `common`, or `types` crate.

### 3. Narrow public and dependency surfaces

Every exported item must have a real production, adapter, application, wire, or durable-schema consumer. Test helpers stay in tests or an explicit non-default test-support feature. Compatibility exists only for an explicit supported contract.

Applications depend on the smallest stable surfaces needed for their role. The CLI must not become a second semantic composition root. Evidence tooling must not force test-only machinery into product surfaces.

### 4. Comprehensible implementation structure

A maintainer must be able to trace these paths without opening dozens of unrelated modules:

- one blueprint import and run start;
- one exact external entry and terminal observation;
- one uncertain external outcome and later resolution;
- one artifact publication and verified read;
- one daemon restart and recovery;
- one prospective revision reconciliation.

Large modules are permitted only when they remain one cohesive exhaustive owner after repeated mechanics have been removed. A generic sentence such as “one owner implements the complete contract” is not enough to justify a 1,500–2,000-line file that contains separable models, validation, transactions, queries, fixtures, or dispatch families.

### 5. Independent tests and evidence

Tests establish behavior from outside the implementation under test. They may share package-local builders and contract suites, but they must not contain another scheduler, ledger, protocol, or persistence algorithm that can agree with production code for the same wrong reason.

Real application evidence launches actual binaries and crosses the same protocol used by an operator. An in-process service test does not substitute for daemon/CLI composition evidence.

### 6. Routine headless product use

From a fresh directory, a maintainer or coding agent must be able to discover the supported setup, validate configuration, start the daemon, use the CLI, execute at least one real local process and one separately managed local model endpoint when available, inspect exact state, restart, and continue. Test fixtures and evidence-only binaries must not be the only practical setup route.

### 7. Concise source-of-truth documentation

The documentation owners remain:

- vision: enduring intent;
- architecture: normative ownership and invariants;
- status: current implementation and current evidence only;
- roadmap: ordered unfinished product work;
- engineering rules: implementation policy;
- workflow/evidence guides: commands and interpretation;
- ADRs: durable historical decisions.

Prompt histories, pass diaries, stale audits, generated inventories, repeated crate maps, and historical benchmark narratives do not belong in the active source tree.

### 8. Architecture freeze

Once the independent closure pass succeeds, broad AI-led architectural construction stops. Subsequent work begins with operation and source tracing, measured defects, or a bounded product requirement. “Another cleanup pass might improve things” is not an unfinished roadmap item.

## Non-negotiable implementation discipline

### Measure before changing

At the start and end of every pass, record outside source control:

- Rust and Markdown file/line counts by production, test, fixture, and evidence category;
- package count and internal dependency edges;
- direct Milkdrift dependencies per application/tool package;
- public API inventory for changed packages, default and all features;
- production files above 1,000 and 1,500 lines;
- cohesion exceptions and their exact rationales;
- lint allowances, especially `too_many_arguments`;
- repeated helper/fake families affected by the pass;
- relevant gate and test runtime observations.

Metrics diagnose structure. Do not improve them by minifying code, combining statements, deleting assertions, moving product code into tests or generated files, using `include!`, wildcard re-exports, or hiding responsibilities behind an opaque data bag.

### Prefer deletion before abstraction

Before adding a type, trait, façade, helper crate, schema, or compatibility reader, search the complete workspace for:

- equivalent representations;
- duplicate validation and construction;
- forwarding layers;
- public items with no real consumer;
- obsolete paths and aliases;
- test-only production exports;
- repeated default literals and error mappings.

Use the smallest mechanism that gives one rule one owner. Fully migrate applicable producers and consumers, then delete the superseded path. A new abstraction beside the old implementations is a regression.

### Require net contraction

Passes 1–4 and 6 are cleanup passes. They must leave a net reduction in the concepts, public items, dependency edges, duplicated mechanics, or source lines within their declared scope. Moving lines among files is not contraction. When correctness requires new code, remove the obsolete code it replaces and report the net result.

A pass may finish with a small line increase only when it closes an independently observable safety or product gap that cannot be expressed by existing owners. The completion report must identify the new invariant, deleted alternatives, and why the resulting system still requires less knowledge to modify safely.

### Preserve difficult truth

Never simplify by converting:

- uncertainty into success or failure;
- authorization denial into absence;
- provider differences into fabricated support;
- running or entered work into pending work;
- historical facts into current definitions;
- branch-local evidence into global context;
- missing usage into zero;
- tags, names, or credentials into authority.

### Preserve compatibility deliberately

Rust source APIs are pre-1.0 and may change atomically across the workspace. Durable documents and wire protocols change only through their owning version, strict reader, canonical fixture, refusal behavior, and relevant ADR/reference update.

Do not bump a schema merely because code moved. Do not retain an obsolete internal API because tests once used it.

## Required verification

Use focused suites while iterating. Before completing a pass, run the complete current local gate from `AGENTS.md`:

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
cargo test -p milkdrift-evidence --test repository_contracts --all-features
```

Run every relevant mutation shard and operational/longevity lane named by the numbered prompt. Fix real surviving mutants with independent assertions. Use only the repository’s exact accepted classification policy; do not classify timeouts or inconvenient behavior casually.

Run `cargo public-api` for every changed library package under both default and all features. Keep generated reports under `target/`; do not commit them.

If a command cannot run because a tool, hosted runner, real model endpoint, or credential is unavailable, state the exact unavailable prerequisite and run every in-repository substitute that does not weaken the claim. Never report an unexecuted gate as passing.

## Completion report for every pass

Return a concise evidence-based report containing:

1. starting and ending commit/tree/clean status;
2. the canonical owner and path that remain;
3. obsolete paths, exports, dependencies, helpers, and files removed;
4. before/after structural metrics relevant to the pass;
5. schema/protocol changes and exact compatibility behavior, or explicit confirmation that none changed;
6. focused tests, full gates, mutation/evidence commands, and exact outcomes;
7. any unavailable external prerequisite;
8. one exact remaining blocker, if the pass is not complete.

Do not create a report, audit, prompt copy, or generated inventory in the repository.
