# Pass 2 — Contract-boundary contraction and public-surface truth

Use this prompt with `00-shared-execution-contract.md`. Run it only after Pass 1 is applied and reviewed.

## Objective

Apply the concrete public-API and shared-mechanics corrections in `docs/development/codebase-audit.md`, then verify that `milkdrift-blueprint`, `milkdrift-capability`, and `milkdrift-contracts` retain only the responsibilities justified by the current dependency graph.

This pass is not permission to collapse the three packages speculatively. It must reduce public concepts, duplicate mechanics, and misleading ownership while preserving the real distinction between definition truth, capability/invocation truth, and cross-domain implementation mechanics.

## 1. Remove obsolete compatibility driving paths

Trace all current consumers, then remove the unused blocking compatibility methods identified by the audit:

- `RuntimeService::effect_tick`;
- `RuntimeService::drive_once`;
- `RuntimeService::tick`;
- `EffectTickResult` if no canonical consumer remains;
- `RunQueryStore::nonterminal_runs`;
- `RunQueryStore::runnable`.

Requirements:

1. Build shared integration-test support from the canonical scheduler/effect operations rather than retaining product methods solely for tests.
2. Migrate every test, fixture, evidence tool, example, and document to the canonical operations.
3. Delete the obsolete methods and any forwarding helpers, aliases, result types, or compatibility prose that become unused.
4. Do not create a new public “test driver” trait or product service to preserve the same second path under another name.
5. Production daemon composition must continue using its one owner/worker path.

## 2. Gate deterministic test implementations

`DeterministicExecutor`, `ManualClock`, and `InMemorySecretResolver` are test/evidence implementations, not unconditional product contracts.

Create the smallest explicit non-default `test-support` feature at each owning package where a separate workspace test/evidence consumer genuinely needs the type. Otherwise keep the implementation entirely in test support.

Requirements:

- default-feature public API inventories must not expose named deterministic/test helpers;
- integration tests and the unpublished evidence package opt into the feature explicitly through dev/test dependencies;
- production packages and normal default consumers do not enable it transitively;
- all-feature test builds remain supported;
- no test helper is moved into a generic shared crate;
- repository-contract tests fail if these named helpers return to a default product surface.

Review every root re-export touched by this change. Keep only intentional product, adapter, durable-schema, or wire-contract exports under `docs/reference/public-api-policy.md`.

## 3. Finish migration of proven shared mechanics

`milkdrift-contracts` already owns cross-domain mechanics, not domain meaning. Complete the two remaining proven mechanics from the audit:

### UTF-8-safe byte truncation

Create the smallest allocation-conscious helper or helpers that express the actually shared rule. Cover zero, exact boundaries, one-byte overflow, multibyte boundaries, and already-short input.

First distinguish genuinely different semantics—for example borrowed truncation, owned redacted detail, and suffix/ellipsis policy. Consolidate only identical rules. Then migrate every applicable production copy and delete the local loops. A wrapper that merely calls the shared function while retaining duplicate boundary logic is incomplete.

### Canonical `b3_` lexical validation

Create one mechanic for validating or decomposing the canonical lowercase `b3_` plus 64-hex lexical form. It must not own a semantic digest type, digest domain, hashing policy, error enum, or schema version.

Migrate authority, control, model, peer protocol, local-process, redb, and every other applicable production reader to the shared lexical mechanic. Each semantic owner continues mapping failure into its own error vocabulary and constructing its own newtype.

Table-driven tests must cover wrong prefixes, lengths, uppercase characters, non-hex ASCII, non-ASCII, empty input, and the exact valid form.

When adding these mechanics, split the current `milkdrift-contracts` implementation into clear private modules if that improves ownership—for example canonical JSON, JSON bounds, lexical text, and validated-string generation. Keep a small explicit root surface. Do not turn the crate into a utility collection.

## 4. Audit blueprint, capability, and contract ownership

Inspect every public type and re-export in:

- `milkdrift-blueprint`;
- `milkdrift-capability`;
- `milkdrift-contracts`;
- their direct consumers and dependency-direction repository checks.

Preserve these default owners unless current source proves a contradiction:

```text
milkdrift-contracts
    cross-domain deterministic mechanics only

milkdrift-capability
    provider-neutral capability descriptions, requirements, resolution, invocation,
    cancellation, observations, terminal evidence, and shared executable schemas

milkdrift-blueprint
    immutable workflow definitions, revisions, mutations, validation, definition-time
    ports, context policy, and semantic fingerprints
```

In particular, verify rather than assume the ownership of `PeerId`, `SchemaId`, `ExtensionKey`, `BoundedJson`, `TrustZone`, and any similar low-level type.

For each questioned type:

1. enumerate real production and durable/wire consumers;
2. identify the lowest existing semantic owner that can be depended upon without a cycle;
3. distinguish semantic ownership from generic construction mechanics;
4. measure whether moving it removes dependencies/public concepts or merely relocates them;
5. inspect serialized identity and digest compatibility;
6. implement a move only if one existing owner is unambiguously better.

Do not create a new identity/core/common crate merely to make names appear cleaner. `PeerId` currently participates in capability locality, authority, peer protocol, persistence, adapters, and daemon composition; cycle avoidance alone is not sufficient reason to duplicate it. If the current owner remains the least harmful correct owner, make that canonical import/re-export policy explicit and remove accidental alternate import paths instead of moving the type.

`SchemaRef` in blueprint and `SchemaContract` in capability are not duplicates merely because both mention schemas. Preserve the distinction between pinning an identity/version in a definition and advertising the executable contract unless source evidence proves otherwise.

The audit must produce code/API cleanup, not only prose. The removals, feature-gating, and mechanic migration above are mandatory regardless of whether any semantic type moves.

## 5. Enforce the resulting dependency and API policy

Update repository contracts so that they verify at least:

- `milkdrift-contracts` does not gain domain-package dependencies or semantic schema constants;
- capability does not depend on blueprint;
- blueprint may depend inward on capability but does not import host/runtime/adapter state;
- default public surfaces exclude test helpers and removed compatibility methods;
- canonical semantic identities are not independently redefined;
- root re-exports remain explicit and intentional.

Run default-feature and all-feature `cargo-public-api` inventories for every touched package, store raw output under `target/public-api`, and review actual consumers rather than optimizing for a numeric decrease.

## 6. Compatibility and documentation

Record an ADR only if semantic ownership or durable meaning changes. Do not add an ADR to restate existing ADR 0006.

Update the public API policy, architecture dependency map, rustdoc, tests, and examples to show only the resulting design. Delete stale compatibility language.

## Scope exclusions

Do not implement controller admission, redesign adapter lifecycle, perform broad file-size refactors, extend the CLI, alter context selection, add a provider family, or add UI.

## Acceptance criteria

The pass is complete only when:

- obsolete runtime/query compatibility paths are gone;
- test-only implementations are absent from default product APIs;
- shared UTF-8 truncation and canonical BLAKE3 lexical mechanics have one implementation across their proven scope;
- semantic digest types and error policies remain domain-owned;
- blueprint, capability, and contract-mechanics ownership is either preserved with explicit canonical imports or atomically improved without a new common crate;
- dependency/API repository checks enforce the result;
- focused and full local gates pass.
