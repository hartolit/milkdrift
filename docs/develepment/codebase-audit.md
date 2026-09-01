# Codebase audit

This audit applies `docs/development/engineering-rules.md` to the current production source,
workspace manifests, tests, repository contracts, and canonical architecture/product documents.
It is an actionable engineering review, not a second owner for product status or roadmap facts.

## Priority summary

| Priority | Finding | Main rule at risk |
| --- | --- | --- |
| High | Peer v1.2 is advertised but the daemon negotiates v1.1 and decoders accept other minors | One coherent design; compatibility; prove the rule |
| High | Peer authority and expiry can consume a fabricated zero timestamp | Nondeterminism boundaries; explicit authority; truthful failure |
| Medium | Public compatibility and test-support paths have no current production consumer | Narrow interfaces; remove obsolete compatibility |
| Medium | `CapabilityAdapter` has several production implementations but no reusable conformance suite | Prove and enforce the rule |
| Medium | Cohesion review is deferred until the 2,000-line backstop | Organize by responsibility; review large files |
| Low | Common bounded-text and digest-format mechanics are reimplemented repeatedly | Abstract shared meaning |

## Findings

### 1. High — Peer version negotiation contradicts the v1.2-only wire contract

The canonical contract says major 1/minor 2 is the only implemented peer version and earlier
minors are refused (`docs/reference/peer-protocol.md:1-3`). The package agrees:
`crates/peer-protocol/src/session.rs:8-21` defines `PROTOCOL_MINOR_V1 = 2` and
`ProtocolVersion::V1_2`.

Production composition does something different:

- `apps/daemon/src/config.rs:382-384` defaults both relationship bounds to minor 1.
- `apps/daemon/src/config.rs:903-907` rejects a minimum above 1, so an exact v1.2-only
  relationship cannot be configured.
- `apps/daemon/src/host.rs:1242-1247` declares the serving daemon's supported range as exactly
  v1.1.
- `crates/peer-protocol/src/document.rs:100-112` validates only the major version, so v1.1 and
  unknown future major-1 minors can be decoded as the current shape.
- `adapters/peer-http/src/client.rs:474-496` discards the decoded envelope version without checking
  that it equals the negotiated version.
- `apps/daemon/tests/two_daemon_peer.rs:428-429` pins the vertical test to v1.1, which explains why
  two identical daemons interoperate despite the contract mismatch.

This is not harmless metadata drift. Minor 2 added the exact queried request identity to lookup
results. A compliant v1.2-only peer should reject v1.1, while this daemon selects it; this daemon
also accepts a future minor whose meaning it cannot know. The current vertical test proves only
that both sides share the same bypass.

Recommended correction:

1. Make the daemon/server and relationship defaults use `ProtocolVersion::V1_2`; remove the raw
   `1` literals. If only one version is supported, remove operator-configurable minor ranges until
   there is a real compatibility contract.
2. Make envelope decoding reject every version except v1.2, or pass the exact negotiated version
   into decoding and require equality. Do not accept an arbitrary major-1 minor.
3. Make the client verify the response envelope version before returning the message.
4. Change the two-daemon test to assert v1.2 selection, add v1.1/future-minor refusal tests at both
   decoder and HTTP boundaries, and extend the repository contract to trace the source version
   constant into the daemon composition root rather than checking documentation text alone.

### 2. High — Clock failure can fail open at peer authority and expiry boundaries

`PeerClock` is injected, but it cannot report failure. `SystemPeerClock::now_unix_ms` maps a clock
before the Unix epoch to `0` and overflow to `u64::MAX`
(`adapters/peer-http/src/service.rs:49-66`). The zero fallback is then used directly to:

- authenticate bearer credentials (`adapters/peer-http/src/service.rs:234-247`);
- decide whether a peer relationship is expired
  (`adapters/peer-http/src/service/authority.rs:22-38`);
- timestamp authority evaluation and rate limiting
  (`adapters/peer-http/src/service/authority.rs:85-132`).

When the production clock reports a pre-epoch error, `now = 0` makes
`now > expires_at_unix_ms` false. That can treat an expired relationship, grant, catalog, or
artifact offer as current. The same adapter also bypasses its injected service clock in
`adapters/peer-http/src/remote.rs:763-769` and `adapters/peer-http/src/artifact.rs:579-585`, where
separate helpers repeat the zero/maximum fallback. The daemon has another zero-fallback helper in
`apps/daemon/src/host/read_model.rs:472-478`, and uses it for durable receipt/control timestamps and
identifier seeds.

This differs from `SystemBoundaryClock` and `SystemArtifactClock`, which return typed errors instead
of manufacturing time (`crates/runtime/src/boundary.rs:20-32` and
`adapters/redb-store/src/store/config.rs:23-34`).

Recommended correction:

1. Make peer and daemon clock acquisition fallible and propagate an unavailable/internal error;
   authority, authentication, expiry, and durable timestamps must fail closed.
2. Inject the same clock owner into remote capability and peer artifact lifecycles; remove their
   direct `SystemTime` helpers.
3. Add deterministic tests for pre-epoch/failing clock behavior, expiry at the exact boundary,
   backwards clock movement, and overflow. Assert that no authentication, authority, catalog, or
   transfer operation is admitted from a fabricated sentinel timestamp.

### 3. Medium — Public compatibility and test-support APIs remain in the normal product surface

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

### 4. Medium — The open adapter interface has no shared conformance suite

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

### 5. Medium — The cohesion guard starts much later than the engineering rule

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

### 6. Low — Repeated contract mechanics still have competing implementations

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

1. Correct and vertically test the peer v1.2 boundary.
2. Make security- and expiry-relevant clocks fallible and consistently injected.
3. Remove/gate unused compatibility and test-only public APIs.
4. Establish adapter conformance tests.
5. Apply targeted cohesion refactors and strengthen the repository guard.
6. Consolidate repeated lexical/bounding mechanics when touching their owners.

The first two items should be completed before making any new peer interoperability claim. The
remaining items are independently shippable cleanup slices and should not be bundled into one
large architectural rewrite.
