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

## Current package surfaces

| Package | Intentional categories and consumers |
| --- | --- |
| `milkdrift-authority` | External product and durable-schema contracts used by control, runtime, persistence, adapters, protocols, and daemon policy. |
| `milkdrift-blueprint` | External product and durable-schema contracts for immutable definitions, validation, documents, and revisions. |
| `milkdrift-capability` | External product and durable-schema contracts shared by hosts, runtime, adapters, peers, and applications. |
| `milkdrift-capability-host` | Workspace adapter ports and the daemon/runtime execution bridge; no durable storage ownership. |
| `milkdrift-contracts` | Workspace adapter mechanics for bounded canonical JSON and validated strings; it owns no semantic identifiers. |
| `milkdrift-control` | Workspace application contracts plus durable proposal/controller documents used by daemon and evidence. |
| `milkdrift-control-client` | External product contract for typed authenticated HTTP/SSE clients, including CLI and future clients. |
| `milkdrift-control-protocol` | External product and durable wire contracts for protocol 2.2; transport/runtime/storage types are excluded. |
| `milkdrift-model` | External product and durable-schema contracts for provider-neutral model requests, responses, and context manifests. |
| `milkdrift-peer-protocol` | External product and durable wire contracts for peer protocol 1.1; HTTP/runtime/storage types are excluded. |
| `milkdrift-persistence` | Workspace adapter ports and durable event/application/peer/snapshot schemas used by runtime and redb. Canonical identities are imported from their owners, not re-exported for compatibility. |
| `milkdrift-prompt-sequence` | External product and durable-schema contracts for schema-2 imports, compilation, inspection, and remediation. |
| `milkdrift-runtime` | Workspace adapter contracts for commands, projection, scheduling, execution, recovery, reconciliation, and context discovery. |
| `milkdrift-workspace` | External product and durable semantic contracts for scopes, values, artifacts, provenance, and budgets. |
| `milkdrift-local-process` | Workspace adapter contract and durable schema-2 profile reader; OS/process internals stay private. |
| `milkdrift-model-provider` | Workspace adapter contract and versioned endpoint profiles; provider wire payloads stay private. `operational-evidence` is evidence-only. |
| `milkdrift-peer-http` | Workspace adapter contract used by the daemon; transport, worker, and storage projections stay with private modules. |
| `milkdrift-redb-store` | Workspace adapter implementations and configuration. Fault injection and mutation inspection are test-only under `test-admin`. |
| `milkdrift-secret-env` | Workspace adapter contract for explicitly configured secret references; values and environment enumeration are not exposed. |
| `milkdrift-daemon` | Workspace application boundary used by its executable, integration tests, and evidence. Its HTTP router and read projections are internal. |
| `milkdrift-cli` | No library surface. Its command behavior and schema-1 JSON output are external application contracts. |
| `milkdrift-evidence` | Unpublished development/test-only contract. Reports belong under `target/evidence`. |

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
