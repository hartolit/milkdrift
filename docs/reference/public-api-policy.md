# Public API policy

Milkdrift is pre-1.0. Its Rust source API follows validated current consumers and may change in one
atomic workspace revision. No general ecosystem stability is promised. Durable JSON documents and
wire protocols are different: their explicit version, bounded reader, canonical encoding, fixture,
and refusal behavior remain compatibility contracts even when their Rust representation changes.

## Classification

Every exported item must have at least one of these owners:

1. **External product contract** — intentionally usable by an application or adapter outside its
   defining package.
2. **Workspace adapter contract** — public because a separate workspace package implements or
   consumes the boundary.
3. **Durable schema compatibility contract** — a versioned serialized shape, reader, or semantic
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

Runtime `ManualClock`/`DeterministicExecutor` and capability-host secret/conformance helpers require
non-default `test-support`. Redb fault hooks require `test-admin`; the provider parser measurement
driver requires `operational-evidence`. Review these separately from default product surfaces.
The evidence package is unpublished and must remain a development leaf. The CLI has no library API.

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
