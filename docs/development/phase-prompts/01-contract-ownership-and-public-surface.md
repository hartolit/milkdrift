# Pass 1 — Contract semantic ownership and public surface

Implement a whole-workspace contraction of semantic ownership, package boundaries, and public APIs. This pass establishes the stable contracts that later cleanup must organize around. It is not an invitation to rename every type, create a new core crate, or collapse packages for cosmetic line-count reduction.

Follow `00-pristine-readiness-contract.md` in full.

## Primary outcome

Leave the repository with:

```text
one semantic owner per cross-package type
    + only earned crate boundaries
    + only consumed public items
    + one implementation of shared contract mechanics
    + no committed cleanup/prompt history
```

Do not change product behavior except where an existing ownership inconsistency or accidental API permits conflicting behavior.

## 1. Remove repository process history and stale secondary owners

Delete `docs/development/phase-prompts/` from the active repository. Its own README says not to commit the prompt package, and it now preserves obsolete execution instructions rather than current product truth.

Review `docs/development/codebase-audit.md`. Migrate any still-current enforceable rule or factual limitation into its canonical owner only when it is absent there, then delete the audit. Do not replace either location with a new cleanup diary.

Search all documentation, repository-contract tests, README links, CI references, and indexes for those paths. Remove or update every stale reference. Add a repository contract that rejects committed prompt-package/pass-history directories or files without forbidding legitimate product guides and ADRs.

## 2. Reproduce the actual contract graph

Use `cargo metadata`, manifests, source search, rustdoc/public-API inventories, and production consumers to build a temporary external working inventory of:

- all packages and internal dependency edges;
- every package-root export;
- every re-export whose semantic owner is another crate;
- test-only or evidence-only public items;
- compatibility methods, aliases, constructors, and readers;
- types used only by an adjacent package that always changes with their current owner;
- mechanisms duplicated across three or more semantic owners.

Do not commit the inventory. Use it to perform actual source contraction.

## 3. Audit blueprint, capability, and contract mechanics precisely

Review the complete contents and consumers of:

- `milkdrift-blueprint`;
- `milkdrift-capability`;
- `milkdrift-contracts`;
- their use by workspace, authority, model, persistence, runtime, control, protocols, adapters, daemon, CLI, and evidence tooling;
- ADR 0006 and the architecture dependency section.

Apply these rules:

- Blueprint continues to own immutable definition structure, schema references, context policies, revisions, mutations, and validation.
- Capability continues to own provider-neutral executable requirements, descriptors, exact resolution, invocations, observations, cancellation, and side-effect contracts.
- Contracts contains only proven cross-domain mechanics with multiple production consumers. It owns no workflow, capability, authority, storage, runtime, or provider meaning.
- `SchemaRef` and an executable `SchemaContract` remain distinct when one is a definition-time reference and the other is a complete advertised operation contract.
- Do not move semantic identities into contracts merely because they use the same validated-string implementation.

Audit questionable low-level ownership individually, including `PeerId`, `SchemaId`, `ExtensionKey`, `BoundedJson`, trust/locality primitives, and execution/reference identities. A move is permitted only when it:

1. gives the type a more accurate semantic owner;
2. preserves acyclic dependency direction;
3. removes more dependency/re-export/conversion complexity than it adds;
4. avoids a new generic identity/types package;
5. migrates all producers, consumers, fixtures, wire/durable schemas, and documentation atomically.

If the present owner is the least harmful cycle-free owner, keep it and remove misleading re-exports/import paths instead of inventing a new crate.

## 4. Contract repetitive construction and decoding mechanics

The supplied checkout contains approximately 62 private `*Wire` types, 92 manual `Deserialize` implementations, and hundreds of strict serde annotations. These may protect real invariants, but repeated boilerplate can conceal inconsistent policy.

Classify every manual reader into:

- required because construction must verify a digest, schema version, private invariant, or cross-field relationship;
- required hostile-input lexical preflight;
- ordinary validated conversion that can use one existing domain constructor or `TryFrom`/serde conversion;
- repeated mechanical boilerplate that can be removed through a small private helper or narrowly scoped declarative macro.

Contract the latter two categories. Preserve:

- strict unknown-field and duplicate-key behavior;
- pre-allocation bounds where currently required;
- exact error vocabulary at each semantic owner;
- canonical bytes and digest domains;
- supported legacy readers and fixtures.

Do not create a generic document framework, reflective validation layer, derive macro crate, or one abstraction spanning unrelated semantic policies. A helper is justified only when it removes multiple production implementations of the same mechanic.

## 5. Contract public Rust APIs

For every library package, classify each exported item under the existing public-API policy. Then:

- make accidental exposure private;
- move test fakes and fault controls into tests or explicit non-default test-support features;
- remove compatibility methods and aliases with no supported production consumer;
- stop re-exporting canonical identities from convenience crates unless that re-export is the intentional product entry point;
- replace public fields with validated construction where callers can create invalid state;
- remove constructors that bypass the canonical reader/builder;
- delete old and new paths that perform the same responsibility.

Give special attention to the broad runtime and persistence roots, adapter profile internals, daemon library exports, and evidence-only access. Do not hide genuine adapter ports, durable schemas, protocol DTOs, or application contracts merely to lower an item count.

Add or strengthen repository checks for named test-helper leakage and obsolete compatibility paths. Keep raw API reports under `target/public-api` only.

## 6. Re-evaluate physical package boundaries

Apply the engineering-rule crate test to every package, not only the three named above. A crate may remain when it enforces at least one real semantic, durable/wire, adapter, dependency-isolation, lifecycle, or independent-consumer boundary.

Merge a package into a parent only when:

- it has no independent consumer or implementation boundary;
- it changes atomically with the parent;
- its public interface mostly forwards, renames, or mirrors parent types;
- the merge reduces public concepts and dependency edges;
- all workspace callers and manifests are migrated in the same pass.

Do not add packages. Do not merge control protocol with its HTTP client, semantic contracts with adapters, or persistence ports with redb merely because they are adjacent. Do not preserve empty compatibility crates after a merge.

## 7. Prepare narrow owner APIs for the later CLI pass

The current CLI directly depends on ten Milkdrift packages. Do not redesign CLI behavior in this pass, but identify every direct semantic dependency that exists only because the CLI performs owner logic locally.

Where appropriate, expose the smallest already-justified operation from the actual owner so that a later CLI can depend on protocol/client plus an intentional authoring façade rather than authority, runtime, persistence, workspace, capability, control, and blueprint internals.

Do not add a broad “CLI service” or duplicate protocol type. New owner APIs must have a non-CLI semantic reason or replace existing duplicated construction immediately.

## Required proof

Add or update tests that prove:

- canonical bytes and supported readers remain exact after boilerplate contraction;
- moved identities cannot be confused or constructed through old import paths;
- package dependency direction remains acyclic and follows architecture;
- default-feature public APIs contain no named test fakes or compatibility drivers;
- every remaining shared mechanic has multiple real production consumers;
- deleted prompt/audit paths cannot silently return through documentation or repository checks.

Run the full gate and public-API inventory for every changed package. Run mutation shards for every semantic owner whose validation, identity, authority, or reader logic changed.

## Completion threshold

This pass is complete only when:

- no package was added;
- package count and internal dependency edges are unchanged or lower;
- public API items are materially lower in the affected packages;
- committed prompt/pass-history files are gone;
- repeated decode/construction mechanics are actually removed rather than wrapped;
- blueprint, capability, and contracts have an explicit, code-supported ownership result;
- no old import, constructor, reader, or compatibility path remains.
