# Runtime benchmarks

`runtime-benchmarks` is the sole non-production cross-crate measurement package. It observes reviewed public production APIs and has three current surfaces:

- the normal `baseline` binary runs bounded, download-free synthetic hosted-E0 scenarios plus fresh E1 startup/shutdown cycles;
- the separate `external-baseline` binary is the sole opt-in external E1 CPU product-baseline path;
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

## Run the external CPU baseline

The external binary fixes `TinyLlama/TinyLlama-1.1B-Chat-v1.0` at immutable revision `fe8a4ea1ffedaf415f4da2f062534de366a451e6`; callers cannot substitute either identity. It requires explicit network authorization and an already-existing canonical cache beneath the repository-root `target/` or outside the repository. It never reads a default global Hub cache implicitly.

```text
mkdir -p target/phase10-external-cache target/phase10-evidence
cargo build --release --locked \
  -p runtime-benchmarks \
  --bin external-baseline

target/release/external-baseline \
  --allow-network \
  --cache-dir target/phase10-external-cache \
  > target/phase10-evidence/external.json
```

Build first, then execute the binary directly so no compiler process overlaps model residency. Stdout contains one external-schema JSON report; progress and the compact summary use stderr. Ordinary tests and CI compile this path but never execute it or access the network. Resource preflight, cache policy, and canonical evidence procedure are in [validation](../../docs/project/validation.md#external-cpu-product-baseline); curated results live only in [performance evidence](../../docs/project/performance.md#external-product-evidence).

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

The synthetic fixture proves deterministic integration and lifecycle behavior, not model quality, product-model speed, representative scale, production serving throughput, allocation freedom, or GPU/device behavior. Synthetic E1 cycles cover startup and bounded shutdown without Hub resolution, model loading, or generation.

The normal `baseline` CLI remains synthetic-only and rejects product/network options. The external binary has a separate opt-in, report schema, and evidence contract; one local CPU/model observation does not establish model quality, serving capacity, cross-host performance, GPU capability, or an optimization threshold. Canonical interpretation is in [performance evidence](../../docs/project/performance.md#external-product-evidence).
