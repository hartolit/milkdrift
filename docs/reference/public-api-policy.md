# Public API policy

Milkdrift is unreleased. Its daemon, clients, peers and workspace libraries are upgraded together.
Rust APIs, wire protocols and durable development formats may change in one atomic workspace
revision. Previous protocol generations do not require compatibility readers, negotiation fallbacks
or migration paths merely because they existed during development. Update current producers,
consumers, fixtures and documentation together, and refuse unsupported versions explicitly.

Current formats still have exact versions, bounded readers, canonical encodings and refusal tests.
Restart durability, immutable history, and exact replay/conflict behavior apply within those
supported formats. An incompatible upgrade must not reinterpret saved facts or silently give old
requests new authority. Any future promise to support multiple versions needs an explicit supported
contract and corresponding evidence.

## Classification

Every exported item must have at least one of these owners:

1. **External product contract** — intentionally usable by an application or adapter outside its
   defining package.
2. **Workspace adapter contract** — public because a separate workspace package implements or
   consumes the boundary.
3. **Durable schema contract** — a versioned serialized shape, reader, or semantic
   fact required to interpret durable or exchanged data.
4. **Accidental exposure** — no current consumer or invariant; remove or narrow it.
5. **Test-only exposure** — fault, inspection, or fixture support; gate it behind an explicit test
   feature or keep it in tests.

Workspace use can justify visibility without making a type a stable third-party API. Root
re-exports exist only when the root is the semantic owner or the re-export is the intentional
package entry point. Consumers otherwise import the canonical owner directly.

## Surface ownership

The [architecture package map](../architecture.md#owners-and-dependency-direction) identifies each
production consumer boundary. Public semantic documents, adapter ports, and application entry
points must meet the classification above; private provider payloads, storage rows, daemon routes,
and projections do not need exports merely for tests.

Runtime `ManualClock`/`DeterministicExecutor` and capability-host secret/conformance helpers,
including unjournaled `SystemPeerClock` and `execute_exact` fixture entry, require non-default
`test-support`. Production serving receives the embedding owner's durable clock. Redb fault hooks
require `test-admin`; the provider parser measurement driver requires `operational-evidence`.
Review these separately from default product surfaces.
The evidence package is unpublished and must remain a development leaf. The CLI has no library API.
The unpublished Slotbook example's `Candidate` and `serve` exports are application entry points
shared by its corrected and explicitly defective fixture binaries. No product package consumes
them. Its booking state and HTTP implementation remain private. Protected recipe schema 2's
verifier executable/digest fields are durable adapter inputs consumed by managed-linux and daemon;
the earlier interpreter/source fields have been removed, with unsupported versions refused.

## Review method

Review all library roots and generated rustdoc JSON, then trace each item through production, test,
documentation, and external protocol consumers. Keep raw reports out of Git:

```sh
mkdir -p target/public-api
cargo public-api -p PACKAGE -sss --all-features --color never \
  > target/public-api/PACKAGE.all-features.txt
cargo public-api -p PACKAGE -sss --color never \
  > target/public-api/PACKAGE.default.txt
cargo machete
cargo tree --workspace --duplicates
```

The default-feature comparison is required for packages with test/evidence features. A new export
needs a named category, real consumer, validating construction where invalid state is possible,
and tests at the owning boundary. A smaller item count is diagnostic evidence, not permission to
hide an actual port, schema, or semantic type.

The managed boundary adds three intentional surfaces: `capability::managed` is the portable
request/inspection and descriptor-binding contract used by CLI, client and adapters;
`persistence::managed` is durable inventory plus transactional use/lifecycle ports implemented by
redb; `capability_host::managed` is the workspace semantic owner, platform/publication ports and
ordinary lifecycle adapter consumed by daemon and managed-linux. Linux recipe/platform/worker/model
constructors are adapter contracts consumed by daemon. Redb tables, indexes and Linux command/unit
helpers stay private. `PreparedAdapterExecution::with_entry_wrapper` lets the managed model adapter
add a use claim around frozen provider bytes; only final host entry receives that wrapper. The
entered-serving fixture helper and optional prepared-serving allowance assertion remain under
`test-support`. Review their default/all-feature inventories with those consumers; none promises a
separately versioned third-party Rust API.

The published-method boundary adds durable schemas and store ports under `persistence::published`;
redb keeps its inventory/index mechanics private. `control::PublishedWorkflowService` and its store
composition bound are workspace adapter contracts consumed by daemon. Capability-host owns
`PublishedWorkflowContinuation` and the prepared pending-work alternative; runtime and serving both
consume that port, while control implements it without a reverse dependency. Runtime's exact
create/bind/start and cancellation methods are workspace ports for that implementation, guarded by
the authoritative saved association. `InvocationCounts`, `NestedWorkUsage`, publication ancestry,
managed `NoExternalEntry` proof and published child links are durable cross-owner contracts; they
cannot be ordinary UI state. The output range and method commands are external control/peer
contracts, retaining exact caller and version checks. No extra public graph engine, mutable method
registry API or unjournaled invocation entry is introduced.
`PublicationAncestor` is a validated durable contract consumed by runtime dispatch, host context,
publication admission and peer delegation. It preserves each accepted depth ceiling.
`TaskExecutor::published_serving_entry_allowed` is a workspace adapter port: runtime invokes it
at internal final entry and capability-host routes it to its existing serving owner. Its default
refuses unavailable authority; the serving attachment and policy checks remain private.
