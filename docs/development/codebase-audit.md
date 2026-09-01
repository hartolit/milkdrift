# Codebase audit

This audit applies `docs/development/engineering-rules.md` to the current production source,
workspace manifests, tests, repository contracts, and canonical architecture/product documents.
It is an actionable engineering review, not a second owner for product status or roadmap facts.

## Priority summary

| Priority | Finding | Main rule at risk |
| --- | --- | --- |
| Medium | Public compatibility and test-support paths have no current production consumer | Narrow interfaces; remove obsolete compatibility |
| Medium | `CapabilityAdapter` has several production implementations but no reusable conformance suite | Prove and enforce the rule |
| Medium | Cohesion review is deferred until the 2,000-line backstop | Organize by responsibility; review large files |
| Low | Common bounded-text and digest-format mechanics are reimplemented repeatedly | Abstract shared meaning |

## Findings

### 1. Medium — Public compatibility and test-support APIs remain in the normal product surface

The pre-1.0 public API policy says test-only exposure must be feature-gated or kept in tests, and
compatibility is retained only for an explicit supported contract. Several current exports do not
meet that standard:

- `RuntimeService::effect_tick`, `drive_once`, and `tick` are documented as blocking compatibility
  drivers (`crates/runtime/src/engine/effects.rs:171-212` and
  `crates/runtime/src/engine/scheduling.rs:182-190`). Repository search finds no production caller;
  `tick` is used by integration tests and `effect_tick` by one integration helper. They preserve a
  second host-driving path that the production daemon intentionally avoids.
- `RunQueryStore::nonterminal_runs` and `RunQueryStore::runnable` are documented compatibility
  shorthands (`crates/persistence/src/journal/query.rs:242-270`). Neither has a production caller;
  only one test store unnecessarily overrides the first.
- `DeterministicExecutor` (`crates/runtime/src/executor.rs:584-652`), `ManualClock`
  (`crates/runtime/src/boundary.rs:37-70`), and `InMemorySecretResolver`
  (`crates/capability-host/src/secret.rs:23-59`) explicitly exist for tests, but are unconditionally
  exported by their package roots. The in-memory resolver is also used by the unpublished evidence
  tool, which can opt into a test-support feature explicitly.

Recommended correction:

1. Move the blocking tick aggregation into shared integration-test support built from the
   canonical `scheduler_tick` / `claim_effects` / `execute_effect` operations, then delete the
   compatibility methods and `EffectTickResult` if it has no remaining consumer.
2. Remove the unused query shorthands and migrate the test fake to the canonical page methods.
3. Gate deterministic fakes behind an explicit non-default `test-support` feature, or keep them in
   test/evidence support code. Add repository checks that production-default API inventories do not
   expose named test helpers.

### 2. Medium — The open adapter interface has no shared conformance suite

`CapabilityAdapter` is an open production interface with lifecycle, exact execution,
cancellation, health, and authority semantics (`crates/capability-host/src/adapter.rs:280-319`). It
has four distinct production implementations:

- `LocalProcessAdapter` (`adapters/local-process/src/process.rs:625`);
- `ModelEndpointAdapter` (`adapters/model-provider/src/adapter.rs:953`);
- `RemoteCapabilityAdapter` (`adapters/peer-http/src/remote.rs:319`);
- `WorkflowControlAdapter` (`crates/control/src/adapter.rs:138`).

The capability-host tests exercise fake adapters and each concrete adapter has focused tests, but
there is no reusable suite that every production implementation runs. Lifecycle defaults are
no-ops, and concrete implementations currently differ in start/drain/shutdown state handling. That
may be valid where semantics genuinely differ, but the shared contract is not independently proved
and those differences are not named by the interface.

Recommended correction:

1. Define the minimum common lifecycle/execution invariants precisely: exact invocation identity,
   observation sequence/terminal shape, cancellation correlation, supplied health timestamp,
   post-drain admission behavior, and shutdown ownership.
2. Add a reusable factory-driven conformance suite under capability-host test support and run it
   against all four production adapters, with mechanism-specific fixtures layered on top.
3. Replace default lifecycle methods with required methods unless a no-resource lifecycle is an
   explicit, tested semantic variant.

### 3. Medium — The cohesion guard starts much later than the engineering rule

The engineering rules require a cohesion review as production files approach roughly 1,000 lines.
The repository contract in `tools/evidence/tests/repository_contracts.rs:432-452` enforces only a
hard `< 2,000` line limit. There are currently 23 production files at or above 1,000 lines. A
diagnostic Clippy pass over workspace library/binary targets with `too_many_lines` and
`cognitive_complexity` enabled reports 127 long functions across 85 files and 12 high-complexity
functions.

The counts are diagnostic, not a request to split exhaustive reducers mechanically. The clearest
first review targets are responsibilities that already have natural phases or command families:

- `apps/cli/src/main.rs:377` — a 438-line command dispatcher, cognitive complexity 62;
- `crates/runtime/src/context/source/discovery.rs:19` — a 472-line function combining projection
  seeding, history reconstruction, event classification, explicit-source resolution, branch/join
  exposure, and final policy validation;
- `apps/daemon/src/host/commands.rs:16` — a 347-line external command dispatcher;
- `apps/daemon/src/host/attempts.rs:190` — a 239-line historical reconstruction path, cognitive
  complexity 34;
- `adapters/redb-store/src/admin/service.rs:120` — a 266-line integrity phase driver, cognitive
  complexity 34.

Recommended correction:

1. Replace the single backstop with a reviewed-exception mechanism: fail new/expanded production
   files above the review threshold, and require a local `#[expect]` rationale for cohesive long
   functions that should remain intact.
2. Refactor the listed dispatchers into private command-family or phase functions. For context
   discovery, introduce one private state object and separate projection seeding, bounded journal
   folding, explicit-source completion, and final validation without changing the owning boundary.
3. Do not split the exhaustive projection event reducers solely to satisfy a metric; review them
   for duplicated transition mechanics first, as required by the engineering rules.

### 4. Low — Repeated contract mechanics still have competing implementations

The repository has already established `milkdrift-contracts` as the owner of cross-domain
implementation mechanics, but two small mechanics remain repeatedly hand-written:

- UTF-8-safe byte truncation loops occur in 18 production files, including three separate
  capability-host modules, two peer-HTTP modules, three daemon modules, and three redb-store
  modules. Examples are `crates/capability-host/src/adapter.rs:87-102`,
  `adapters/peer-http/src/service.rs:780-789`, and `apps/daemon/src/http.rs:358-367`.
- The canonical lowercase `b3_` plus 64-hex format is independently checked in authority, control,
  model, peer protocol, local-process, and redb code. Examples are
  `crates/authority/src/identity.rs:108-126`, `crates/model/src/context.rs:23-43`,
  `crates/peer-protocol/src/execution.rs:750-755`, and
  `adapters/local-process/src/config.rs:1018-1024`.

The semantic digest types should remain domain-owned. The duplicated lexical mechanics should not:
minor differences already exist (`is_ascii_hexdigit` plus an uppercase check versus an explicit
`0-9a-f` range), increasing the chance that one reader drifts.

Recommended correction:

1. First consolidate repeated truncation within each crate behind one private function.
2. Add the smallest shared lexical helpers to `milkdrift-contracts` only where doing so removes
   multiple real production implementations; keep semantic error types and digest newtypes in
   their existing owners.
3. Add table-driven contract tests with zero limits, multibyte boundaries, uppercase hex,
   wrong prefixes, wrong lengths, and non-ASCII input.

## Suggested remediation order

1. Remove/gate unused compatibility and test-only public APIs.
2. Establish adapter conformance tests.
3. Apply targeted cohesion refactors and strengthen the repository guard.
4. Consolidate repeated lexical/bounding mechanics when touching their owners.

The remaining items are independently shippable cleanup slices and should not be bundled into one
large architectural rewrite.
