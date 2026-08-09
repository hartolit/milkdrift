# Milkdrift model-artifact trust and immutable source boundary

## Objective

Replace path-based model source trust with one durable immutable-artifact boundary that supports both Hub-resolved and local artifacts, preserves exact provenance, and enables the Candle loader to verify and materialize each shard in one normal payload pass.

This work owns artifact acquisition and source identity. Do not redesign tensor conversion or the E0 scheduler in this prompt.

## Read first

Read these before editing:

- `README.md`
- `docs/vision.md`
- `docs/project/architecture.md`
- `docs/project/candle-backend.md`
- `docs/project/application-runtime.md`
- `docs/agent/decisions/0020-transactional-prepared-model-loading.md`
- `crates/adapters/hf-hub/src/lib.rs`
- `crates/adapters/candle-backend/src/source.rs`
- the model-resolution and persistence path in `application-runtime` and `redb-storage`

Treat Phase 12 as implemented. Preserve its declared/observed/planned/actual fact separation and ownership guarantees.

## Owned area

Primary ownership:

- `crates/adapters/hf-hub`
- the model-source identity types consumed by `candle-backend`
- a narrowly scoped model-artifact component under `crates/adapters/` or `crates/platform/` if a separate package is justified
- artifact handoff through `application-runtime`
- persistence changes required to retain stable artifact identity
- deterministic tests and fixture metadata for this boundary

Touch portable domain contracts only for vendor-neutral identity/value types. Do not place paths, `File`, Hugging Face DTOs, SHA implementation details, or Candle types into `domain-contracts`.

## Problems to eliminate

The present source object carries config and weight paths. Candle preparation opens those paths, parses headers, hashes every tensor payload, and materialization later rereads and rehashes payloads.

This has several weaknesses:

- immutable identity is an application convention rather than a source capability;
- path replacement and same-inode mutation are handled inside one backend instead of one reusable artifact boundary;
- every load performs an avoidable full payload verification pass before materialization;
- the same provenance and digest logic will otherwise be reimplemented for future local backends and external artifact sources;
- current configuration parsing treats an unrecognized modern `dtype` similarly to an absent declaration and may fall back to `torch_dtype`.

## Required architecture

Implement one explicit verified-artifact model with these properties:

1. Every accepted file has a stable logical role, exact byte length, cryptographic content identity, and provenance.
2. A Hub artifact is bound to the immutable resolved commit, repository-relative path, and verified local bytes.
3. A local artifact enters through the same verification/canonicalization process rather than bypassing it with a raw path constructor.
4. Verification is reusable across loads. Do not recompute a full artifact digest during preparation merely because the backend is loading it again.
5. Normal loading must be able to consume a verified artifact through a retained lease/handle or equivalently strong source object without reopening an arbitrary caller path.
6. The artifact boundary must have an explicit threat model for accidental replacement, truncation, same-inode mutation, symlink escape, cache corruption, interrupted writes, and concurrent readers.
7. Use atomic publication into a content-addressed or equivalently strong immutable store. Do not trust path, size, mtime, or revision text alone.
8. Do not duplicate model bytes on every load. One-time ingestion/canonicalization is acceptable; repeated loads must reuse the verified artifact.
9. Keep the implementation safe Rust. Do not introduce an unsafe mmap wrapper.
10. Do not build a generic workflow artifact system here. This component owns immutable model/config/tokenizer bytes and their provenance only.

The preferred normal flow is conceptually:

```text
external or Hub file
    -> verify/canonicalize once
    -> immutable artifact identity + retained local source
    -> adapter preparation parses bounded metadata only
    -> adapter materialization streams current bytes once while checking expected whole-file identity
```

The exact type design is yours, but the resulting API must make raw path trust difficult or impossible in production composition.

## Configuration declaration semantics

Correct the Hugging Face configuration parser so these facts remain distinct:

- modern `dtype` absent;
- modern `dtype` present and recognized;
- modern `dtype` present but unsupported or unknown;
- legacy `torch_dtype` present and recognized;
- both fields present and contradictory.

Do not allow an unsupported modern declaration to be silently hidden by a recognized legacy field. Preserve absent metadata as `None`; represent unsupported or contradictory declarations as explicit compatibility failure where appropriate. Actual Safetensors headers remain authoritative artifact evidence.

## Persistence and migration

If verified identities are persisted:

- version the record explicitly;
- retain repository/revision/immutable commit separately from local content identity;
- migrate existing records deterministically;
- never restore a path as though it proves the same immutable bytes;
- do not silently discard old records or reinterpret old scalar codes.

## Required tests

Add deterministic, download-free coverage for at least:

- identical bytes deduplicate to one content identity;
- different bytes at the same original path do not reuse an identity;
- symlink and path traversal cannot escape the accepted artifact namespace;
- interrupted or partial ingestion never publishes a valid artifact;
- length/digest mismatch fails closed;
- config, tokenizer, and weight files from different immutable selections cannot be combined;
- a renamed/replaced source path does not alter an already verified artifact;
- mutation/corruption of the verified store is detected before successful load publication;
- modern/legacy dtype absent, recognized, unknown, and contradictory cases;
- old persisted records migrate without fabricating verified identity.

Use small project-authored fixtures. Do not add downloaded model blobs.

## Validation

Run the focused package tests and Clippy for every package changed. At minimum include the Hub adapter, artifact component, Candle source construction, application resolution tests, and storage migration tests.

Also run:

```text
cargo fmt --all -- --check
cargo check --locked -p hf-hub-adapter -p candle-backend -p application-runtime -p redb-storage
cargo clippy --locked -p hf-hub-adapter -p candle-backend -p application-runtime -p redb-storage --all-targets -- -D warnings
git diff --check
```

Adjust package names if the implementation introduces a new narrowly scoped crate.

## Finish

Leave the repository compiling with a coherent handoff for the Candle-loading prompt. Do not leave a parallel raw-path production path merely to reduce the diff. Do not push.
