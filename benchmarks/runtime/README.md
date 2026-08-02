# Runtime benchmarks

`runtime-benchmarks` is the sole non-production cross-crate measurement package. It observes reviewed public production APIs and has two current surfaces:

- the normal `baseline` binary runs bounded, download-free synthetic hosted-E0 scenarios plus fresh E1 startup/shutdown cycles;
- `benches/runtime.rs` runs two hosted-E0 Criterion submission-to-event measurements.

Operational timeouts stop hangs; they are not performance thresholds. Fixture, identity, output, lifecycle, cleanup, accounting, or join mismatches fail. A slow valid sample does not.

Canonical methodology, environment, curated results, RSS interpretation, and limitations live in [Phase 10 performance evidence](../../docs/project/performance.md). Repository-wide procedures live in [validation](../../docs/project/validation.md).

## Package contract

- Run Cargo from the repository root with the committed root `Cargo.lock`.
- Use only the root `target/`; never create `benchmarks/runtime/Cargo.lock`, `benchmarks/runtime/target`, or a source-tree results/cache directory.
- Production, application, tooling, and test packages do not depend on this package.
- Benchmark support remains private to this package; no helper is added to a production public API solely for measurement.
- The committed Candle fixture is referenced in place and its byte sizes, hashes, parsed configuration, and loaded descriptor are verified before measurement.

## Test and compile

```text
cargo check --locked -p runtime-benchmarks --all-targets
cargo test --locked -p runtime-benchmarks
cargo clippy --locked -p runtime-benchmarks --all-targets -- -D warnings
cargo bench --workspace --no-run --locked
```

These commands compile the current benchmark targets without claiming a product baseline.

## Run the synthetic baseline

```text
mkdir -p target/phase10-evidence
cargo run --release --locked \
  -p runtime-benchmarks \
  --bin baseline \
  -- \
  --mode synthetic \
  --warmup 1 \
  --cycles 3 \
  > target/phase10-evidence/synthetic.json
```

The runner writes one schema-versioned JSON document to stdout and progress plus a compact summary to stderr. It excludes generated text, token IDs, credentials, secrets, and broad environment dumps.

## Run focused Criterion targets

```text
cargo bench --locked \
  -p runtime-benchmarks \
  --bench runtime \
  -- e0_hosted_checked_prefill/4_tokens

cargo bench --locked \
  -p runtime-benchmarks \
  --bench runtime \
  -- e0_hosted_incremental_decode/1_token_after_2_token_prefill
```

Criterion has no repository regression threshold. Raw samples and reports are local generated artifacts; only curated values belong in [performance evidence](../../docs/project/performance.md).

## Artifacts and temporary state

Temporary redb state for E1 lifecycle checks is created beneath root `target` and removed where possible. Captured JSON, Criterion data/HTML, profiles, heap dumps, and caches remain beneath root `target` or outside the repository and are not committed.

## Evidence limits

The synthetic fixture proves deterministic integration and lifecycle behavior, not model quality, product-model speed, representative scale, production serving throughput, allocation freedom, or GPU/device behavior. E1 cycles cover startup and bounded shutdown without Hub resolution, model loading, or generation.

The CLI contract is synthetic-only; product/network options are intentionally rejected. The canonical external-evidence status and future requirement are in [performance evidence](../../docs/project/performance.md#external-product-evidence).
