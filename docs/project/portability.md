# Portable Feature Targets

The portability contract is established by [ADR-0007](../agent/decisions/0007-portability-targets.md). It applies to feature libraries only; adapters, engines, applications, tests, and benchmarks are excluded from cross-target claims.

## Target matrix

| Crate | Host `std` tests (`x86_64-unknown-linux-gnu`) | `wasm32-unknown-unknown` library | `thumbv7em-none-eabihf` library | Allocation evidence in project-owned production code |
|---|---:|---:|---:|---|
| `domain-contracts` | Yes | Yes | Yes | Named allocation contract tests cover selected operations; no crate-wide external allocation claim |
| `tokenization` | Yes | Yes | Yes | APIs use caller-owned token/byte storage; no crate-wide measured allocation claim |
| `context-planner` | Yes | Yes | Yes | Fixed caller-provided entries and output storage; no crate-wide measured allocation claim |
| `sampling` | Yes | Yes | Yes | Allocation test covers the prepared sampling pipeline and reusable workspace |
| `task-graph` | Yes | Yes | Yes | Allocation test covers topology validation, artifact provenance, ready discovery, and attempt transitions after caller preparation |

“Yes” for a cross target means the library compiles for that exact target with the committed lockfile. It does not claim browser integration, JavaScript bindings, firmware integration, target execution tests, or support for every WebAssembly/embedded target.

## Reproducible commands

```text
cargo test --workspace --locked
cargo check --locked --target wasm32-unknown-unknown --lib -p domain-contracts -p tokenization -p context-planner -p sampling -p task-graph
cargo check --locked --target thumbv7em-none-eabihf --lib -p domain-contracts -p tokenization -p context-planner -p sampling -p task-graph
```

Cross-compilation deliberately selects `--lib` so host-only development dependencies such as Criterion and allocation instrumentation do not broaden the production portability boundary.
