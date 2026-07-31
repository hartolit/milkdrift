# Dependency and repository policy

## Architecture enforcement

`cargo run --locked --bin llm-app -- architecture` loads the actual workspace through the typed `cargo_metadata` API and runs both architecture and repository-hygiene validation. Unknown workspace locations and unresolved local path targets fail closed.

Runtime and platform roles are explicit. `inference-runtime`, `corrective-workflow`, and `application-runtime` are the recognized E0, capability, and E1 packages; `host-runtime` is the only recognized package under `crates/platform`. The private local composition in `application-runtime/src/local.rs` is an internal module, not another package or layer.

Normal and build dependencies first use the complete production layer matrix. Production edges from a runtime to platform/adapters or another runtime then require an exact source/target/kind review entry with a narrow composition justification. Development dependencies are reviewed separately because compatibility tests and benchmarks may need edges that production code must not acquire.

The current external policy is:

- F0 production code has no external dependencies;
- F1 production code may use only reviewed portable dependencies, currently `sampling -> libm`;
- platform/adapters may use implementation dependencies appropriate to their integration boundary;
- runtimes may use only reviewed external orchestration dependencies and no frontend toolkit;
- apps depend on E1 in production rather than directly on E0 or adapters;
- external and workspace-local development dependencies require separate exact review.

The reviewed E1 production composition edges cover `candle-backend`, `hf-hub-adapter`, `hf-tokenizer`, `host-runtime`, `inference-runtime`, and `redb-storage`. They authorize the one current Candle/Hub/Safetensors/CPU composition, bounded workers/output, and application persistence; they do not authorize arbitrary adapters or a public generic backend surface. The only workspace-local E0 development edge is `inference-runtime -> candle-backend` for executable compatibility coverage.

`desktop-slint` has one workspace-local production dependency, `application-runtime`. No `application-api` package exists.

## Rust-owned operational hygiene

The independently runnable policy command is:

```text
cargo run --locked --bin llm-app -- hygiene
```

The check inspects Git-tracked files, direct Cargo declarations, and the locked selected dependency graph. It rejects tracked project-owned secondary-language tooling artifacts, maintained operational invocations outside the Rust/Cargo path, prohibited direct manifest packages, and selected packages from the removed native-engine or embedded-runtime families. Dated analysis, superseded ADR rationale, the user-provided cleanup brief, and execution history may retain explanatory text; they are not executable workflow guidance.

`cargo run --locked --bin llm-app -- architecture` and the canonical `verify` command include this hygiene policy. [ADR-0014](../agent/decisions/0014-rust-cargo-native-operational-tooling.md) owns the rationale and exact policy boundary.

## Supply-chain policy

`deny.toml` configures `cargo-deny` to check the full workspace for advisories, licenses, registry/Git sources, duplicate versions, and narrow exact package bans. Duplicate versions are warnings and an audit input, not an automatic requirement to collapse semantically distinct dependency trees. Cargo-deny 0.20 reports workspace-inherited declarations as wildcards even though versions/paths are centralized in the root manifest, so its wildcard lint is allowed; the typed architecture validator independently rejects unreviewed local paths and portable external dependencies.

The project source is available under `MIT OR Apache-2.0`; canonical texts are in `LICENSE-MIT` and `LICENSE-APACHE`. Slint dependencies are reviewed under `LicenseRef-Slint-Royalty-free-2.0`; distribution must continue to satisfy Slint's attribution and license terms.

Only the crates.io registry is accepted by default. A Git dependency or alternate registry requires an explicit policy change and review.

The advisory policy contains five exact, justified exceptions. `paste`, `ttf-parser`, and `rustybuzz` are unmaintained transitive dependencies with no safe compatible update. `quick-xml 0.39` has two advisories but is pinned by `wayland-scanner`; in this graph it parses trusted Wayland protocol XML during the build rather than runtime or user input. Review these exceptions whenever Candle/tokenizers, Slint, or Wayland dependencies update.

## Linux CI prerequisites

The current Ubuntu CI installs only the selected native build prerequisites and Slint system development packages:

```text
build-essential
cmake
libfontconfig1-dev
libxcb-shape0-dev
libxcb-xfixes0-dev
libxkbcommon-dev
```

`build-essential` and `cmake` remain because the selected Rust dependency graph builds the `aws-lc` TLS path used by Hub access. The fontconfig/XCB/XKB packages belong to Slint windowing and font integration. `clang` and `libclang-dev` were removed with the former native engine path. Rust/Cargo-native project tooling does not imply that every selected Rust dependency is implemented without a native build step.

## Documentation links

`lychee.toml` defines Markdown link checking. Pull requests and pushes run `lychee` offline so repository-local paths and fragments are deterministic blocking checks. External HTTP links run in the scheduled CI job because third-party availability must not make an otherwise valid pull request nondeterministic.

## Reproducibility and audit reports

`Cargo.lock` is committed. CI starts with locked metadata and uses `--locked` for architecture, hygiene, compile, test, lint, documentation, benchmark, portability, and dependency-policy commands. Cargo-deny evaluates that committed resolution with:

```text
cargo deny --workspace --locked check advisories bans licenses sources
```

`cargo metadata --locked --format-version 1`, `cargo tree -d --locked`, and `cargo tree -e features --locked` are audit inputs. Duplicate packages are reviewed rather than rejected categorically. Large generated logs are not committed; [implementation status](implementation-status.md) records summarized evidence.
