# Sampling component

The `sampling` crate owns deterministic production sampling and stop-pattern matching. Its Criterion target, [`benches/sampling_pipeline.rs`](benches/sampling_pipeline.rs), measures those public operations as component-regression evidence; it does not measure E0/E1 generation or product-model throughput.

## Sampling matrix

The target registers 48 sampling cases:

```text
{sample_only,restore_and_sample}/<case>/{8192,32768,131072}
```

The eight case segments are:

- `greedy` — highest-logit selection scaling;
- `default_top_k_top_p` — the current default top-k 40/top-p 0.95 policy;
- `min_p_0_05_full_vocabulary` — min-p 0.05 without top-k pre-truncation;
- `repetition_disabled_history_256` — supplied history with penalty disabled;
- `repetition_enabled_empty` — enabled repetition processing with empty history;
- `repetition_short_unique_8` — eight distinct history tokens;
- `repetition_medium_unique_64` — 64 distinct history tokens;
- `repetition_repeated_heavy_256` — 256 entries cycling over four token IDs.

Every case runs at 8,192, 32,768, and 131,072 logits. The vocabulary sizes are benchmark parameters, not claims about a supported model.

### Timed boundaries

- `sample_only`: mutable logits are restored before timing. The measured region contains public workspace-view construction, `Sampler::sample`, result validation, and `black_box`.
- `restore_and_sample`: timing starts before baseline logits are copied into the mutable buffer, so restoration is intentionally included before the same sampling boundary.

Deterministic logits, histories, sampler state, vectors, and reserved capacity are prepared once outside the measured loop. Iterations reuse those allocations and do not clone inputs. Criterion records elapsed time and throughput metadata; it does not count allocations. The deterministic Rust-global-allocation contract remains in `tests/allocation.rs`.

## Stop matching

The same target measures public `match_stop_suffix` with all caller-owned inputs prepared outside timing:

- `stop_matching/token_hit/1_pattern_generated_128` — one-token hit against one pattern;
- `stop_matching/pattern_hit_last/8_patterns_generated_128` — four-token hit in the last of eight patterns;
- `stop_matching/pattern_miss/8_patterns_generated_128` — no match across eight patterns.

Only the match call and `black_box` are timed. These targets do not observe allocator events or native/device resources.

## Evidence and commands

Run one selected target with:

```text
cargo bench --locked -p sampling --bench sampling_pipeline -- \
  sample_only/default_top_k_top_p/32768
```

Compile the complete benchmark matrix without timing it with:

```text
cargo bench --workspace --no-run --locked
```

There are no hard timing thresholds. Exact executed targets, compact observed intervals, deferred component candidates, environment metadata, and evidence limitations are recorded in the canonical [performance guide](../../../docs/project/performance.md).
