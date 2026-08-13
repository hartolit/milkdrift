# Runtime benchmarks

`runtime-benchmarks` is a non-production outer observer. It exercises public E0
and E1 boundaries without becoming part of product composition. Canonical
methodology and curated results live in
[performance evidence](../../docs/project/performance.md); acceptance procedures
live in [validation](../../docs/project/validation.md).

## Maintained surfaces

| Surface | Purpose | Output |
|---|---|---|
| `runtime` Criterion bench | Hosted-E0 checked prefill and incremental decode component timing | Criterion artifacts under root `target/` |
| `baseline --mode synthetic` | Download-free E0 lifecycle/accounting/loader/process observation plus cold E1 start/shutdown | Synthetic schema-6 JSON |
| `external-baseline` | Fixed TinyLlama public-E1 resolve/load/chat/completion/cancel/unload/shutdown observation on CPU or approved CUDA | External schema-6 JSON |
| Unit tests | Report shape, fixed identities, parsers, lifecycle plans, summaries, and observer invariants | Test results only; not measurements |

The package shares private support for digest formatting, checked deadlines,
typed application-event waits, bounded polling/disconnection, and cleanup. CPU and
CUDA external runs use the same lifecycle, observer, validation, and report code.

## Compile and test

```sh
cargo check --locked -p runtime-benchmarks --all-targets
cargo test --locked -p runtime-benchmarks
cargo clippy --locked -p runtime-benchmarks --all-targets -- -D warnings
cargo bench --locked -p runtime-benchmarks --bench runtime --no-run
```

CUDA compile-only coverage is explicit:

```sh
CUDA_COMPUTE_CAP=120 cargo check --locked \
  -p runtime-benchmarks --all-targets --features cuda
```

These commands do not establish a product baseline, external-model compatibility,
or hardware execution.

## Synthetic runner

```sh
mkdir -p target/runtime-evidence
cargo run --release --locked \
  -p runtime-benchmarks --bin baseline -- \
  --mode synthetic --warmup 1 --cycles 3 \
  > target/runtime-evidence/synthetic-schema6.json
```

The runner references the committed Candle fixture in place and verifies its size,
hash, configuration, and public descriptor. One fixed-size observation channel is
attached to the actual hosted load transaction; it does not perform a shadow
preparation.

Synthetic schema 6 has three non-duplicated owners:

- `prepared` records declaration, observed categories, planned execution/device,
  and exact final/loading footprints from the actual preparation;
- `receipt` records E0-verified actual execution/device and accepted final
  reservation; and
- `loader` records preparation/materialization duration, bytes read, transfer
  batches, and loading synchronizations.

Process RSS/HWM remains a separate sampled observation. A failed or retained
cleanup load aborts rather than becoming a normal performance sample.

## External runner

The external binary fixes `TinyLlama/TinyLlama-1.1B-Chat-v1.0` at immutable
revision `fe8a4ea1ffedaf415f4da2f062534de366a451e6`. It requires an explicit cache,
network authorization, clean commit, and exactly `--device cpu` or
`--device cuda:0`; it never uses a default global cache or CPU fallback.

External schema 6 observes only the public E1 product boundary plus qualified
process and `nvidia-smi` whole-device data. It does not independently call adapter
preparation, reconstruct E0 planning, or claim process-attributed VRAM. The fixed
profile is homogeneous BF16 and is not mixed-checkpoint evidence.

Use the exact build/run/review procedure in
[validation](../../docs/project/validation.md#controlled-external-product-evidence).

## Criterion targets

```sh
cargo bench --locked -p runtime-benchmarks --bench runtime -- \
  e0_hosted_checked_prefill/4_tokens

cargo bench --locked -p runtime-benchmarks --bench runtime -- \
  e0_hosted_incremental_decode/1_token_after_2_token_prefill
```

Criterion has no repository performance threshold. Fixture/model setup, lifecycle
teardown, and semantic validation outside the named public boundary are excluded
as documented in the performance guide.

## Artifact policy

Reports are serialize-only; the package has no legacy report parser. Historical
schemas retain their recorded meaning through curated documentation rather than
migration code. Generated JSON, Criterion output, model caches, temporary redb
state, and compiler artifacts remain under root `target/` or outside the
repository and are never committed.
