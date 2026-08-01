# Dependency and repository policy

## Architecture enforcement

`cargo xtask architecture` loads the virtual workspace through the typed `cargo_metadata` API. Unknown workspace locations and unresolved local path targets fail closed. Architecture and hygiene are independently runnable checks; `cargo xtask verify` runs both before the composite Cargo gate.

`tools/xtask` is the sole recognized tooling package and has the exact reviewed external edge to `cargo_metadata`. Runtime and platform roles are also explicit. `inference-runtime`, `corrective-workflow`, and `application-runtime` are the recognized E0, capability, and E1 packages; `host-runtime` is the only recognized package under `crates/platform`. The private local composition in `application-runtime/src/local.rs` is an internal module, not another package or layer.

Normal and build dependencies first use the complete production layer matrix. Every domain-to-domain production edge then requires an exact source/target/kind review entry with a nonempty rationale, and the complete reviewed domain graph must remain acyclic. The current registry contains only the four F1 → F0 edges from `tokenization`, `context-planner`, `sampling`, and `task-graph` to `domain-contracts`. The coarse matrix can admit a reviewed F1 peer, but no unreviewed domain peer is allowed. F0 vocabulary itself requires a backend/runtime crossing or at least two stable, distinct domain consumers; `TaskId` therefore remains owned by `task-graph`.

Production edges from a runtime to platform/adapters or another runtime likewise require an exact review entry with a narrow composition justification. Development dependencies are reviewed separately because compatibility tests and benchmarks may need edges that production code must not acquire.

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
cargo xtask hygiene
```

The check inspects Git-tracked artifacts and operational text surfaces, direct Cargo declarations, and the locked selected dependency graph. It rejects tracked project-owned secondary-language tooling artifacts, maintained operational invocations outside the Rust/Cargo path, prohibited direct manifest packages, and selected packages from the removed native-engine or embedded-runtime families.

There is no filename-, directory-, document-status-, or whole-file bypass for a tracked operational surface. Historical names, execution-history locations, and superseded ADR status do not exempt content. Negative policy examples are accepted only when the parser can identify them as prohibitions rather than instructions. Any future exception must be exact, narrowly reviewed, and covered by tests; a broad path or status exemption is not acceptable.

`cargo xtask architecture` does not implicitly run hygiene. The canonical `cargo xtask verify` command runs architecture and hygiene in sequence before format/build/test/lint/documentation/benchmark compilation. [ADR-0014](../agent/decisions/0014-rust-cargo-native-operational-tooling.md) owns the rationale and policy boundary.

## Supply-chain policy

`deny.toml` configures `cargo-deny` to check the full workspace for advisories, licenses, registry/Git sources, duplicate versions, and narrow exact package bans. Duplicate versions are warnings and an audit input, not an automatic requirement to collapse semantically distinct dependency trees. Cargo-deny 0.20 reports workspace-inherited declarations as wildcards even though versions/paths are centralized in the root manifest, so its wildcard lint is allowed; the typed architecture validator independently rejects unreviewed local paths and portable external dependencies.

Milkdrift project-authored source code and documentation are licensed under `Apache-2.0`; the canonical text is in [`LICENSE`](../../LICENSE), with attribution in [`NOTICE`](../../NOTICE). Third-party dependencies retain their own license terms. In particular, Slint remains reviewed under `LicenseRef-Slint-Royalty-free-2.0` and is not relicensed by Milkdrift's Apache license. Distributed applications must continue to satisfy Slint's applicable attribution, licensing, and distribution requirements.

Only the crates.io registry is accepted by default. A Git dependency or alternate registry requires an explicit policy change and review.

The advisory policy contains five exact, justified exceptions. `paste`, `ttf-parser`, and `rustybuzz` are unmaintained transitive dependencies with no safe compatible update. `quick-xml 0.39` has two advisories but is pinned by `wayland-scanner`; in this graph it parses trusted Wayland protocol XML during the build rather than runtime or user input. Review these exceptions whenever Candle/tokenizers, Slint, or Wayland dependencies update.

## Linux CI prerequisites

The current Ubuntu CI installs only the selected native compiler and Slint system development packages:

```text
build-essential
libfontconfig1-dev
libxcb-shape0-dev
libxcb-xfixes0-dev
libxkbcommon-dev
```

The selected non-FIPS `aws-lc` path uses its CC builder, so `build-essential` supplies the required native compiler. CI does not install a system CMake executable. The Rust `cmake` crate may remain in `Cargo.lock` as part of an upstream build-dependency set, but the selected build does not require the system executable. The fontconfig/XCB/XKB packages belong to Slint windowing and font integration; `clang` and `libclang-dev` remain absent with the former native engine path.

Required Linux quality CI builds from a fresh `CARGO_TARGET_DIR`, explicitly selects the non-system, non-CMake `aws-lc` path, and places failing shims ahead of the environment for CMake executables and the prohibited Python and Hugging Face command-line families. Those external tools must not be invoked; any attempted invocation fails the job instead of being satisfied by the runner image.

## Documentation links

`lychee.toml` defines Markdown link checking. Pull requests and pushes run `lychee` offline so repository-local paths and fragments are deterministic blocking checks. External HTTP links run in the scheduled CI job because third-party availability must not make an otherwise valid pull request nondeterministic.

## Reproducibility and audit reports

`Cargo.lock` is committed. The `cargo xtask` alias itself selects the tooling package with `--locked`, and CI uses locked resolution for architecture, hygiene, compile, test, lint, documentation, benchmark, portability, and dependency-policy commands. Cargo-deny evaluates that committed resolution with:

```text
cargo deny --workspace --locked check advisories bans licenses sources
```

`cargo metadata --locked --format-version 1`, `cargo tree -d --locked`, and `cargo tree -e features --locked` are direct Cargo audit inputs. Duplicate packages are reviewed rather than rejected categorically. CI prints the checked-out commit and Git tree identifiers immediately before the canonical gate, so the runtime log carries provenance. A tracked evidence document need not embed the resulting hash of the tree that contains that document, which would create a self-referential update. Large generated logs are not committed; [implementation status](implementation-status.md) records summarized evidence.
