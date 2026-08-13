# Dependency and repository policy

## Architecture enforcement

`cargo xtask architecture` loads the virtual workspace through locked typed `cargo_metadata`. Every tracked non-fixture Cargo package must be a root workspace member, every member must declare one known `[package.metadata.milkdrift] role`, and that declaration must occupy the compatible direct-child root. Roles are not inferred from names or prefixes; omitted members, unresolved local path targets, missing/unknown roles, and incompatible locations fail closed. The verification planner uses that same locked, role-validated workspace inventory to produce exact sorted package selections for check, test, Clippy, and rustdoc. Architecture and hygiene remain independently runnable, and `cargo xtask verify` runs both first.

The generic inward role DAG is canonical in [project architecture](architecture.md). It keeps portable domain code below platform/adapters, E0 below capability/E1, applications above E1, benchmark observers outside production, and tooling isolated. Same-role runtime peers are denied. Normal/build Cargo edges that obey the role DAG are ordinary legal edges and are not copied into another registry.

The actual normal/build F0/F1 graph is derived from Cargo declarations and must remain acyclic, including any allowed peer/foundation edge. This preserves domain ownership without making every future workflow or SDK crate edit a package-name constitution. `TaskId` remains owned by `task-graph`; shared-foundation vocabulary still requires real cross-boundary ownership rather than policy convenience.

The root `[workspace.metadata.milkdrift]` namespace and exact integer `policy-version = 1` are mandatory; omission or an unknown version fails closed. Its exception registry is intentionally small. Each record has a stable ID, exact source/target/scope/kind, and nonempty rationale. Restricted external edges, every workspace-local/external development edge, and CUDA forwards require records. Records are validated in both directions: duplicates, stale declarations, wrong kinds, missing packages, empty rationales, and unnecessary exceptions fail. An exception cannot override an upward role edge or any tooling/observer absolute denial.

The current workspace-local development exception is `inference-runtime -> candle-backend`; executable E0 compatibility and dedicated CUDA suites use it without creating a production edge. The current external policy is:

- F0/F1, runtimes, tooling, and observers require exact review for production external dependencies;
- current portable production review is `sampling -> libm`; allocation-test
  development reviews cover `domain-contracts`, `sampling`, and `task-graph`
  through `stats_alloc`;
- platform/adapters/apps may use ordinary implementation dependencies appropriate to their boundary, but sensitive CUDA and every development dependency still require exact review;
- E0's external development `candle-core` edge supports download-free mixed-fixture conversion only;
- tooling reviews `cargo_metadata`, `serde_json`, and `toml` for policy parsing,
  plus development-only `serde_yaml_ng` for maintained-workflow syntax tests; and
- observer external reviews cover `serde`, `serde_json`, `sha2`, and development-only `criterion`.

A package with role `benchmark-observer` is an outer consumer. Cargo's `normal` label does not place it in production. It cannot use build dependencies/custom-build targets or be depended upon by product, tooling, tests, applications, or another observer. Unknown benchmark manifests fail closed. `runtime-benchmarks` currently observes `application-runtime`, `candle-backend`, `domain-contracts`, `host-runtime`, and `inference-runtime`; those ordinary legal outgoing local edges come from Cargo rather than a duplicate list.

Maintained Cargo benchmark targets are separately registered in owning package metadata and compared bidirectionally with Cargo metadata and exactly one explicit owning-manifest `[[bench]]` entry with `harness = false`. The complete inventory is `runtime-benchmarks/runtime` and `sampling/sampling_pipeline`. The canonical gate compiles only those exact targets; an implicit, harnessed, new, or missing target fails policy rather than being silently omitted or compiling a zero-benchmark libtest binary.

The exact product opt-in chain is `desktop-slint/cuda -> application-runtime/cuda -> candle-backend/cuda`; `runtime-benchmarks/cuda` observes through E1, and `inference-runtime/cuda` is development-only. Every default feature set is empty. The validator rejects aliases, direct dependency CUDA features, unreviewed forwards, and `cudnn`, `flash-attn`, or `nccl` as feature names, forwarded references, dependency package names, or dependency aliases.

`candle-backend` is the sole declared CUDA provider. Its feature contains exactly optional `cudarc` plus the three reviewed Candle forwards. The direct `cudarc` edge is exact `=0.19.8`, optional, unrenamed, default-feature-free, and limited to the reviewed feature set. `cuda-hardware-tests = ["cuda"]` is permitted only as a package-local non-default alias for one explicit harness-free `cuda_hardware` target; it cannot be selected or forwarded by dependencies.

`desktop-slint` has one workspace-local production dependency, `application-runtime`. No `application-api` package exists.

## Rust-owned operational hygiene

The independently runnable policy command is:

```text
cargo xtask hygiene
```

The check inspects Git-tracked artifacts and operational text surfaces, direct Cargo declarations, and the locked selected dependency graph. It rejects tracked project-owned secondary-language tooling artifacts, maintained operational invocations outside the Rust/Cargo path, prohibited direct manifest packages, and selected packages from the removed native-engine or embedded-runtime families.

Repository artifact rules additionally reject:

- any tracked path component named `target`;
- a nested `Cargo.lock` or `build.rs` under `benchmarks/`;
- tracked benchmark/criterion result trees, generated reports, flamegraphs, profiler output, heap dumps, and package-local result directories;
- tracked model/download cache directories;
- any tracked non-fixture Cargo package manifest that is not a root workspace member;
- a benchmark member without an explicit compatible observer role, or a Cargo bench target without exact metadata registration and one explicit `harness = false` manifest entry.

Documentation hygiene also requires the current authority-spine files and their
map entries, forbids the retired analyzer/implementation-plan/free-floating
application warning paths, and permits only `archive/README.md` under the completed
prompt archive. Original prompt and analysis bodies remain available through Git
history rather than tracked active-tree copies.

The root `.gitignore` uses `target/` so Cargo output is ignored at every repository depth. Ordinary local Cargo work may use root `target`; clean acceptance and CI instead set one named isolated `CARGO_TARGET_DIR` per job, verify that root `target` was not created, observe disk use, and remove the isolated directory. Raw Criterion/profiler data and model caches remain under an approved target or outside the repository. Curated conclusions belong in canonical documentation rather than generated result trees.

There is no filename-, directory-, document-status-, or whole-file bypass for a tracked operational surface. Historical names, execution-history locations, and superseded ADR status do not exempt content. Negative policy examples are accepted only when the parser can identify them as prohibitions rather than instructions. Any future exception must be exact, narrowly reviewed, and covered by tests; a broad path or status exemption is not acceptable.

`cargo xtask architecture` does not implicitly run hygiene. The canonical `cargo xtask verify` command iterates the same `structure`, `check`, `test`, `clippy`, `docs`, and `benches` plans exposed by `cargo xtask verify-component`. Structure owns architecture, hygiene, format, and locked metadata validation; the remaining plans own their exact Cargo operation. The composite and hosted components therefore cannot maintain independent package registries. Exact registered benchmark compilation never falls back to a workspace-wide release bench build. [ADR-0014](../agent/decisions/0014-rust-cargo-native-operational-tooling.md) owns the rationale and policy boundary.

## Model fixture and external artifact policy

A committed model fixture must have a reviewed provenance record with origin/redistribution basis, architecture, scalar type, deterministic generation method, exact sizes and SHA-256 hashes, license, and test scope. Small size or synthetic-looking tensors are not ownership evidence.

Project-owned synthetic fixtures are generated through Rust/Cargo-native tooling without external base-model weights, tokenizer assets, network access, or training data. They remain beside their integration owner and governed generator until multiple real consumers plus a clearer shared ownership boundary justify relocation. The E0 integration test remains the Candle fixture's primary owner; `application-runtime` tests already load the files for E1 coverage, and the current `runtime-benchmarks` observer is an additional consumer. Both additional consumers reference the bytes in place without copying them, so relocation would not remove duplication. The benchmark recomputes the exact hashes before setup and uses the fixture only for synthetic integration and lifecycle measurements, not product performance. If provenance or redistribution permission cannot be established, the bytes are replaced rather than reused or expanded.

Real-model measurements name an external identifier and immutable revision, use an explicit opt-in cache or artifact path, perform no ordinary-CI download, and do not redistribute model/tokenizer files through the repository. Any product baseline must define and enforce its network/cache contract, actually execute through public product behavior, and keep generated artifacts under root `target` or outside the repository. Download availability or successful compilation alone is not evidence or redistribution permission. See [ADR-0018](../agent/decisions/0018-benchmark-and-model-fixture-policy.md) and the current [performance evidence](performance.md#external-product-evidence).

## Supply-chain policy

`deny.toml` configures `cargo-deny` to check the full workspace for advisories, licenses, registry/Git sources, duplicate versions, and narrow exact package bans. Duplicate versions are warnings and an audit input, not an automatic requirement to collapse semantically distinct dependency trees. Cargo-deny 0.20 reports workspace-inherited declarations as wildcards even though versions/paths are centralized in the root manifest, so its wildcard lint is allowed; the typed architecture validator independently rejects unreviewed local paths and portable external dependencies.

Milkdrift project-authored source code and documentation are available under
`MIT OR Apache-2.0`, at the recipient's option; the canonical texts are in
[`LICENSE-APACHE`](../../LICENSE-APACHE) and
[`LICENSE-MIT`](../../LICENSE-MIT).

Third-party dependencies retain their own license terms. In particular, Slint
remains reviewed under `LicenseRef-Slint-Royalty-free-2.0` and is not
relicensed by Milkdrift's licenses. Distributed applications must continue to
satisfy Slint's applicable attribution, licensing, and distribution
requirements. `webpki-root-certs` packages certificate-root data under the
Linux Foundation's `CDLA-Permissive-2.0`; that data license is allowed for this
transitive TLS trust-store input, and redistribution must preserve the license
text or link required by its terms.

Only the crates.io registry is accepted by default. A Git dependency or alternate registry requires an explicit policy change and review.

The advisory policy contains five exact, justified exceptions. `paste`, `ttf-parser`, and `rustybuzz` are unmaintained transitive dependencies with no safe compatible update. `quick-xml 0.39` has two advisories but is pinned by `wayland-scanner`; in this graph it parses trusted Wayland protocol XML during the build rather than runtime or user input. Review these exceptions whenever Candle/tokenizers, Slint, or Wayland dependencies update.

## Linux CI prerequisites

The canonical CPU CI remains CUDA-toolkit-free and installs only the selected native compiler and Slint system development packages:

```text
build-essential
libfontconfig1-dev
libxcb-shape0-dev
libxcb-xfixes0-dev
libxkbcommon-dev
```

The selected non-FIPS `aws-lc` path uses its CC builder, so `build-essential` supplies the required native compiler. CI does not install a system CMake executable. The Rust `cmake` crate may remain in `Cargo.lock` as part of an upstream build-dependency set, but the selected build does not require the system executable. The fontconfig/XCB/XKB packages belong to Slint windowing and font integration; `clang` and `libclang-dev` remain absent with the former native engine path.

Each native Linux quality component builds from its own fresh `CARGO_TARGET_DIR`, explicitly selects the non-system, non-CMake `aws-lc` path where native compilation needs it, and places failing shims ahead of compilation for CMake executables and the prohibited Python and Hugging Face command-line families. Structure installs no native packages; exact benchmark compilation installs only `build-essential`; UI/backend compilation adds the documented Slint packages. Those external tools must not be invoked; any attempted invocation fails the job instead of being satisfied by the runner image.

## Documentation links

`lychee.toml` defines Markdown link checking. Pull requests and pushes run `lychee` offline so repository-local paths and fragments are deterministic blocking checks. External HTTP links run in the scheduled CI job because third-party availability must not make an otherwise valid pull request nondeterministic.

## Reproducibility and audit reports

`Cargo.lock` is committed. The `cargo xtask` alias itself selects the tooling package with `--locked`, and CI uses locked resolution for architecture, hygiene, compile, test, lint, documentation, benchmark, portability, and dependency-policy commands. Cargo-deny evaluates that committed resolution with:

```text
cargo deny --workspace --locked check advisories bans licenses sources
```

`cargo metadata --locked --format-version 1`, `cargo tree -d --locked`, and `cargo tree -e features --locked` are direct Cargo audit inputs. Duplicate packages are reviewed rather than rejected categorically. CI prints the checked-out commit and Git tree identifiers immediately before the canonical gate, so the runtime log carries provenance. A tracked evidence document need not embed the resulting hash of the tree that contains that document, which would create a self-referential update. Large generated logs are not committed; [implementation status](implementation-status.md) records summarized evidence.
