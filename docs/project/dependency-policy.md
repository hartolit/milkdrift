# Dependency and Repository Policy

## Architecture enforcement

`cargo run --locked --bin llm-app -- architecture` loads the actual workspace through the typed `cargo_metadata` API. Unknown workspace locations and unresolved local path targets fail closed.

Runtime and platform roles are explicit. `inference-runtime`, `corrective-workflow`, and `application-runtime` are the recognized E0, capability, and E1 packages; `host-runtime` is the only recognized package under `crates/platform`. Additional runtime or platform crates fail closed instead of inheriting authority from their directory.

Normal and build dependencies first use the complete production layer matrix. Production edges from a runtime to platform/adapters or another runtime then require an exact source/target/kind review entry with a narrow composition justification. This makes concrete runtime/infrastructure coupling deliberate without requiring per-edge registration for ordinary domain dependencies.

Development dependencies are reviewed separately because compatibility tests and benchmarks may need edges that production code must not acquire. External production exceptions are also exact and justified; there are no wildcard portable exceptions.

The initial external policy is:

- F0 production code has no external dependencies;
- F1 production code may use only reviewed portable dependencies, currently `sampling -> libm`;
- platform/adapters may use implementation dependencies appropriate to their integration boundary;
- runtimes may use only reviewed external orchestration dependencies and no frontend toolkit;
- apps depend on E1 in production rather than directly on E0 or adapters;
- external and workspace-local development dependencies require separate exact review.

Validator unit tests cover all 64 source/target layer combinations, explicit runtime/platform classification, reviewed runtime composition edges, and external-policy failures. Integration fixtures prove that an unregistered runtime, an unregistered platform crate, and an otherwise layer-valid but unreviewed E1-to-capability edge all fail closed.

## Supply-chain policy

`deny.toml` configures `cargo-deny` to check the full workspace for advisories, licenses, registry/Git sources, and duplicate versions. Duplicate versions are warnings and an audit input, not an automatic requirement to collapse semantically distinct dependency trees. Cargo-deny 0.20 reports workspace-inherited declarations as wildcards even though their versions/paths are centralized in the root manifest, so its wildcard lint is allowed; the typed architecture validator independently rejects unreviewed local paths and portable external dependencies.

The project source is available under `MIT OR Apache-2.0`; canonical texts are in `LICENSE-MIT` and `LICENSE-APACHE`. Slint dependencies are reviewed under `LicenseRef-Slint-Royalty-free-2.0`; distribution must continue to satisfy Slint's attribution and license terms. The automated policy does not replace review of licenses bundled in native C/C++ source distributions.

Only the crates.io registry is accepted by default. A Git dependency or alternate registry requires an explicit policy change and review.

The advisory policy contains five exact, justified exceptions. `paste`, `ttf-parser`, and `rustybuzz` are unmaintained transitive dependencies with no safe compatible update. `quick-xml 0.39` has two advisories but is pinned by `wayland-scanner`; in this graph it parses trusted Wayland protocol XML during the build rather than runtime or user input. These exceptions must be reviewed whenever Candle/tokenizers, Slint, or Wayland dependencies update.

## Documentation links

`lychee.toml` defines Markdown link checking. Pull requests and pushes run `lychee` offline so repository-local paths and fragments are deterministic blocking checks. External HTTP links run in the scheduled CI job because third-party availability must not make an otherwise valid pull request nondeterministic.

## Reproducibility and audit reports

`Cargo.lock` is committed. CI starts with locked metadata and uses `--locked` for architecture, compile, test, lint, documentation, benchmark, portability, and dependency-policy commands. Cargo-deny evaluates that committed resolution with `cargo deny --workspace --locked check advisories bans licenses sources`.

`cargo tree -d --locked` is emitted as an audit report. It is intentionally not a policy that every duplicate version must be eliminated. Large generated logs are not committed; the canonical status records summarized evidence.
