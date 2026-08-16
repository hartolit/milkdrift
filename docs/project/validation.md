# Validation

This document owns repeatable commands, prerequisites, evidence classification,
failure interpretation, and the procedure for accepting a new run. It contains no
historical run diary. Current accepted run IDs and support state live in
[implementation status](implementation-status.md); chronology lives in
[execution history](../agent/execution/history.md); measurements live in
[performance evidence](performance.md).

## Evidence rules

Accept evidence only from a reviewed tree and record:

```sh
git status --short --untracked-files=all
git rev-parse HEAD
git rev-parse 'HEAD^{tree}'
```

- A result proves only the exact commit/tree and command that ran.
- Working-tree, clean committed-tree, hosted, self-hosted hardware, external-model,
  and manual UI results are distinct evidence classes.
- Compilation is not hardware execution. A deterministic fixture is not an
  external checkpoint. Process RSS is not owner-attributed memory.
- Run heavy Cargo processes sequentially with `CARGO_INCREMENTAL=0`.
- Do not run `cargo clean`. Do not reuse a target when clean-target evidence is
  required.
- Keep ordinary local artifacts under root `target/`, or place validation artifacts
  in one explicitly named isolated `CARGO_TARGET_DIR` outside the repository. Never
  create package-local targets.
- A documentation-only change does not rerun product evidence retroactively; it
  still requires its own documentation and policy gates.

## Canonical repository gate

The normal local composite is:

```sh
cargo xtask verify
```

It consumes six typed plans shared with hosted CI:

```sh
cargo xtask verify-component structure
cargo xtask verify-component check
cargo xtask verify-component test
cargo xtask verify-component clippy
cargo xtask verify-component docs
cargo xtask verify-component benches
```

`structure` runs architecture, hygiene, formatting, and locked metadata.
`benches` compiles only package-metadata-owned maintained targets:
`runtime-benchmarks/runtime` and `sampling/sampling_pipeline`. It does not run
statistics. `verify-component nursery` is exploratory and non-blocking; it is not
part of ordinary acceptance.

Run the composite from one new isolated target when exact clean-target evidence is
needed:

```sh
test ! -e /tmp/milkdrift-validation-target
export CARGO_TARGET_DIR=/tmp/milkdrift-validation-target
export CARGO_INCREMENTAL=0
cargo xtask verify
```

Choose another validated direct child if that path exists. After recording size
and filesystem observations, remove only that exact target through the reviewed
environment cleanup mechanism. Confirm that no unexpected root target was created:

```sh
find . -path './.git' -prune -o -type d -name target -print
git status --short --untracked-files=all
git status --short --ignored
```

Root `target/` and its descendants are valid for ordinary local work; clean CI or
isolated acceptance must not accidentally use them.

## Focused download-free CPU diagnostics

Use focused commands to diagnose a subsystem or establish a named component
boundary. They do not replace the canonical gate:

```sh
cargo test --locked -p domain-contracts

cargo test --locked -p candle-backend
cargo test --locked -p hf-hub-adapter
cargo test --locked -p inference-runtime
cargo test --locked -p application-runtime
cargo test --locked -p redb-storage
cargo test --locked -p runtime-benchmarks
cargo test --locked -p xtask
```

Important named boundaries include:

- Candle CPU fixtures: required/observed/declaration/execution scalar separation,
  retained-file identity, required-only materialization, load footprints, and
  sequence reservation;
- E0 native/fault tests: prepared-load admission, generation, backpressure,
  cancellation, exact/unverified retention, cleanup retry/exhaustion, unload, and
  shutdown;
- E1 tests: immutable resolution/load correlation, persistence, chat/completion,
  output, cleanup coordination, disconnection, and shutdown;
- benchmark/xtask schema, metadata, workflow, resource, and command-plan tests.

Fixture regeneration is a maintenance operation, not ordinary validation:

```text
cargo test --locked -p candle-backend --test generate_synthetic_fixture -- --ignored --exact regenerate_committed_candle_fixture
```

Run it only when deliberately replacing fixture bytes; review the generated files
and provenance hashes in the same change.

## Portable domain targets

The maintained portable package set is derived from metadata. Run both plans in
separate fresh targets:

```sh
cargo xtask portable wasm32-unknown-unknown
cargo xtask portable thumbv7em-none-eabihf
```

The expected product set is `domain-contracts`, `tokenization`, `context-planner`,
and `sampling`. No adapter, native runtime, storage, UI, observer, or tool package
may leak into these library checks. A compile proves only the named target/library
boundary, not browser or firmware integration.

## Architecture, hygiene, dependencies, and links

Run policy halves directly when diagnosing:

```sh
cargo xtask architecture
cargo xtask hygiene
cargo deny --workspace --locked check advisories bans licenses sources
lychee --config lychee.toml --offline '**/*.md'
git diff --check
```

Offline Lychee is the deterministic pull-request gate for local files/fragments.
External HTTP checking runs separately when network access is available:

```sh
lychee --config lychee.toml '**/*.md'
```

An unrelated third-party outage does not invalidate local source closure. Record
the affected URL/status and distinguish it from broken repository-local links.

Useful dependency audits are:

```sh
cargo metadata --locked --format-version 1
cargo tree --workspace --locked
cargo tree -d --locked
cargo tree -e features --locked
```

Duplicate versions are review inputs, not automatic failures.

## Exact current-checkout remote acceptance

Remote acceptance is an external property of one commit, not a status prediction
stored in that commit. It is intentionally absent from the offline
`cargo xtask verify` gate. The procedure requires network access, an authenticated
GitHub CLI with repository Actions read access, and `jq`:

```sh
gh auth status
repository="$(gh repo view --json nameWithOwner --jq .nameWithOwner)"
test -z "$(git status --short --untracked-files=all)"
head_sha="$(git rev-parse HEAD)"
head_tree="$(git rev-parse 'HEAD^{tree}')"

quality_runs="$(gh api \
  "/repos/${repository}/actions/workflows/quality.yml/runs?head_sha=${head_sha}&event=push&status=completed&per_page=100")"
quality_run_id="$(jq -er --arg sha "${head_sha}" '
  [.workflow_runs[] |
    select(.head_sha == $sha and
           .path == ".github/workflows/quality.yml" and
           .event == "push" and
           .status == "completed" and
           .conclusion == "success")][0].id
' <<<"${quality_runs}")"
quality_jobs="$(gh api \
  "/repos/${repository}/actions/runs/${quality_run_id}/jobs?filter=latest&per_page=100")"

while IFS= read -r required_job
do
  jq -e --arg name "${required_job}" '
    [.jobs[] | select(.name == $name)] |
    length == 1 and all(.status == "completed" and .conclusion == "success")
  ' <<<"${quality_jobs}" > /dev/null
done <<'QUALITY_JOBS'
Native structure, format, and metadata
Native workspace check
Native workspace tests
Native strict Clippy
Native warning-denied rustdoc
Native maintained benchmark compilation
Portable domain crates (WebAssembly)
Portable domain crates (embedded no_std)
Dependency and documentation policy
QUALITY_JOBS
jq --argjson id "${quality_run_id}" '
  .workflow_runs[] | select(.id == $id) |
  {id, head_sha, path, event, status, conclusion, html_url}
' <<<"${quality_runs}"
jq -r '.jobs[] | [.name, .status, .conclusion] | @tsv' <<<"${quality_jobs}"

cuda_runs="$(gh api \
  "/repos/${repository}/actions/workflows/cuda-hardware.yml/runs?head_sha=${head_sha}&status=completed&per_page=100")"
cuda_run_id="$(jq -er --arg sha "${head_sha}" '
  [.workflow_runs[] |
    select(.head_sha == $sha and
           .head_branch == "main" and
           .path == ".github/workflows/cuda-hardware.yml" and
           (.event == "push" or .event == "workflow_dispatch") and
           .status == "completed" and
           .conclusion == "success")][0].id
' <<<"${cuda_runs}")"
cuda_jobs="$(gh api \
  "/repos/${repository}/actions/runs/${cuda_run_id}/jobs?filter=latest&per_page=100")"
jq -e '
  [.jobs[] | select(.name == "RTX 5070 Ti correctness")] |
  length == 1 and all(.status == "completed" and .conclusion == "success")
' <<<"${cuda_jobs}" > /dev/null
jq --argjson id "${cuda_run_id}" '
  .workflow_runs[] | select(.id == $id) |
  {id, head_sha, head_branch, path, event, status, conclusion, html_url}
' <<<"${cuda_runs}"
jq -r '.jobs[] | [.name, .status, .conclusion] | @tsv' <<<"${cuda_jobs}"
printf 'accepted commit %s, tree %s\n' "${head_sha}" "${head_tree}"
```

The workflow-file API endpoints establish workflow identity; display titles and
the repository's globally latest run do not. Inspect the selected run objects and
record their `head_sha`, `path`, `event`, `status`, `conclusion`, job conclusions,
and URLs in external completion output or a platform-bound release/attestation.
For a Quality push, only the explicitly schedule-only `Scheduled Clippy nursery`
and `External Markdown links` jobs may be skipped; every job listed above must
succeed. Current CUDA acceptance additionally requires the maintained CUDA job to
succeed for the same `head_sha`. A signed release attestation is an alternative
only when it is cryptographically or platform-bound to that exact commit and names
the required gates.

Missing network access, authentication, permission, or a matching completed run
means remote evidence could not be obtained; it is not a source failure. Never
substitute an older green run. After any fix, recompute `HEAD` and restart the
procedure for the new SHA. Do not commit a status file solely to announce the
result.

## Shared hosted Quality

`.github/workflows/quality.yml` runs untrusted pull requests and pushes on
GitHub-hosted Ubuntu 24.04. It gives structure, check, test, Clippy, rustdoc, and
exact benchmark compilation independent native runners/targets. Portable targets,
dependency policy, nursery linting, and links use separate jobs. Only nursery
findings are non-blocking.

Every Cargo-building job disables incremental compilation, rejects checkout-local
targets, records disk use, and unconditionally removes its named resources. The
workflow uses the same `verify-component` and portable plans as local tooling.
It does not enable CUDA, download an external model, run a performance threshold,
or provide hardware evidence.

Treat a hosted `No space left on device` or consequential linker failure as CI
infrastructure until a source-level failure is independently shown. Inspect the
failing leg, resource preflight, target size, and cleanup result rather than
rewriting product support from an infrastructure symptom.

## CUDA compile and hardware correctness

The default graph is CPU-only. CUDA requires an NVIDIA driver for the intended
device, Toolkit 12.8 or newer, and build capability 120:

```sh
nvidia-smi
nvcc --version
export CUDA_COMPUTE_CAP=120
```

The metadata-owned compile and strict-lint graph is:

```sh
cargo metadata --locked --format-version 1 --no-deps > /dev/null
cargo xtask architecture
cargo xtask hygiene
cargo xtask cuda-compile
cargo xtask cuda-clippy
```

The two CUDA planners derive every exact `cuda` feature owner and registered
hardware suite from locked Cargo metadata. They include the ordinary all-target
graph, no-run test compilation, each harness-free hardware target, and the serial
fault-injection target without maintaining another package or target list here.

Do not use workspace `--all-features`. Compilation proves feature/API
compatibility only. Hardware execution additionally requires:

```sh
export CUDA_VISIBLE_DEVICES=0
export CUDA_COMPUTE_CAP=120
export MILKDRIFT_CUDA_TEST=1
cargo xtask hardware cuda
```

The metadata-owned `cuda` profile resolves the harness-free Candle, E0, and E1
suites plus the serial E0 fault-injection target. Unknown, empty, mismatched, or
unregistered profiles fail closed. The suites cover explicit CPU in a CUDA build,
invalid ordinal, F32/BF16/mixed adapter paths, generation/accounting/release,
deterministic failed cleanup, selected/actual E1 device, unload, and shutdown.

`.github/workflows/cuda-hardware.yml` runs only trusted `main` pushes or owner
dispatches of `main` on the dedicated `milkdrift-cuda-5070ti` runner. It has no
pull-request trigger, uses read-only permissions, synchronizes the dependency-only
Cargo cache before offline metadata/compile/test work, isolates check and release
hardware targets under `RUNNER_TEMP`, and always removes both. Do not weaken its
labels to generic `self-hosted` routing.

A Toolkit preflight does not broaden product support. Record exact GPU name,
ordinal, driver, toolkit/compiler, compute capability, build cap, commit/tree,
profile, and run URL.

## Controlled external product evidence

The sole ordinary network exception is `runtime-benchmarks`' fixed E1 external
runner for `TinyLlama/TinyLlama-1.1B-Chat-v1.0` at immutable revision
`fe8a4ea1ffedaf415f4da2f062534de366a451e6`. It requires explicit authorization,
an existing explicit cache, a clean committed tree, and exactly `--device cpu` or
`--device cuda:0`; it never falls back.

Build CPU and CUDA executables in separate root-target children and execute the
binaries directly so compiler work cannot overlap model residency:

```sh
mkdir -p target/phase12-cpu target/phase12-cuda target/phase12-evidence

CARGO_TARGET_DIR="$PWD/target/phase12-cpu" \
cargo build --release --locked -p runtime-benchmarks --bin external-baseline

CUDA_COMPUTE_CAP=120 CARGO_TARGET_DIR="$PWD/target/phase12-cuda" \
cargo build --release --locked -p runtime-benchmarks --features cuda \
  --bin external-baseline

target/phase12-cpu/release/external-baseline \
  --allow-network --cache-dir target/phase10-external-cache --device cpu \
  > target/phase12-evidence/tinyllama-cpu-schema6.json

target/phase12-cuda/release/external-baseline \
  --allow-network --cache-dir target/phase10-external-cache --device cuda:0 \
  > target/phase12-evidence/tinyllama-cuda-schema6.json
```

Review reports programmatically without printing generated text or token IDs.
Require exact clean Git identity, fixed model/workload, selected versus actual
device/scalar agreement, terminal/release identity, cancellation after progress,
clean unload/shutdown, qualified process and whole-device observations, and the
absence of fabricated direct-E0 planning/ownership fields.

The fixed profile is homogeneous BF16 and is not mixed-checkpoint evidence. No
reviewed external mixed checkpoint is established. Network, credential, cache, or
service failure is acquisition failure; only an acquired artifact that reaches the
product contract can provide compatibility evidence.

## Manual Slint acceptance

Use a new isolated application-data root:

```sh
test ! -e target/phase12-slint-data
mkdir -p target/phase12-slint-data
XDG_DATA_HOME="$PWD/target/phase12-slint-data" \
CUDA_COMPUTE_CAP=120 \
cargo run --release --locked -p desktop-slint --features cuda
```

A human verifies CPU/CUDA presentation and explicit selection, immutable
resolution/load, actual execution display, streamed output, cancellation after
progress, unload to idle, and bounded close. Process launch alone is not graphical
acceptance.

## Failure interpretation

| Observation | Classification |
|---|---|
| Missing toolchain, driver, device, cache, credentials, or network | Environment/acquisition precondition; not product incompatibility |
| CUDA build passes but hardware suite did not execute | Compile-only evidence |
| Hardware opt-in absent or suite resolves zero cases | Validation failure, never a successful skip |
| Hosted disk exhaustion | CI resource failure unless a source failure is independently reproduced |
| Generated output differs while lifecycle invariants hold | Workload/model-quality observation; not automatically a runtime ownership failure |
| Cleanup pending/exhausted or terminal process retention | Real lifecycle failure/retention; do not report release from zero exact bytes or disconnect |
| External URL unavailable while offline links pass | External availability result; local documentation closure may still pass |

## Recording a new accepted run

After a run:

1. record commit, tree, clean status, exact commands/profile, environment, and run URL;
2. identify whether it is local, hosted, hardware, external-product, manual, or measurement evidence;
3. record the narrow outcome and important gap without copying command output;
4. update [implementation status](implementation-status.md) only if it changes the
   current support/evidence matrix;
5. add one concise [history](../agent/execution/history.md) milestone when the run
   closes meaningful work;
6. place timings/memory tables only in [performance evidence](performance.md); and
7. re-run documentation links and whitespace after editing evidence pages.

Never rewrite an older run as evidence for a later tree.
