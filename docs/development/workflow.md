# Development workflow

This document owns assignment procedure, scope and findings policy, completion requirements,
and build, test, lint, fixture, and focused verification commands.
Use the exact toolchain in [rust-toolchain.toml](../../rust-toolchain.toml).
[Development practices](practices/README.md) own implementation and documentation guidance.
[Virtual office](virtual-office/README.md) explains how to divide long-running work into temporary,
coherent assignments and remove sprint coordination files after completion. Its
[whiteboard](virtual-office/whiteboard/README.md) carries broader topics into sprint preparation.

## Carry out an assignment

Read the [practices](practices/README.md) relevant to the assigned work. The procedure below applies
within that scope; code-specific steps apply to implementation changes, and documentation work
follows the [explanation procedure](practices/documentation.md#5-work-on-an-explanation).
Implementation and documentation can inform each other as findings change the design. A final
reader review can be an internal pass in the same assignment; it does not require another agent
or a fixed sequence of practices.

### Before editing

1. Read the relevant implementation, tests, configuration, composition roots, and owning documentation.
2. Identify the responsibility being changed, its intended owner, and the existing explanation of how callers use it.
3. Search the whole workspace for equivalent implementations, callers, defaults, and bypasses.
4. Define the responsibility, expected result, exclusions, checks, and stop condition. Identify likely files to coordinate edits; use the [virtual office](virtual-office/README.md) for sprint assignments.

### During the work

Apply the selected practices, including the [implementation steps](practices/implementation.md#5-apply-an-implementation-change)
when changing code. Revisit the design and explanation when evidence changes; preserve the
assignment's responsibility and exclusions.

### Before finishing

1. Search again for old types, helpers, literals, factories, readers, call paths, and terminology.
2. Confirm that the new design is used throughout its declared scope.
3. Confirm that no production-local implementation competes with an adopted adapter.
4. Run all relevant quality, contract, failure, and architecture checks.
5. Once the affected behavior and structure have settled, review the final structure and
   explanation from the perspective of a new contributor. Follow the affected operation through
   the docs before checking it against source; correct missing connections and stale guidance as
   well as inaccurate descriptions of individual symbols. Complete necessary documentation in
   the same change; capture local reasons during implementation and compose the broader account
   from the finished design.
6. Report what became canonical, what was removed, and what evidence proves completion.

## Findings beyond the assignment

Scope follows an assigned responsibility and its acceptance criteria, not a file count. Complete
necessary corrections, callers, tests, and documentation within that responsibility even when
the plan did not anticipate them. Update the file list as they are discovered. Coordinate shared
files with other workers; respect explicit user exclusions and separately assigned ownership.
A genuine conflict needs a narrow scope decision, not an unfinished result disguised as a new issue.

Use the [whiteboard](virtual-office/whiteboard/README.md) for problems or ideas requiring a broader
decision, another responsibility, or work excluded by the assignment. Explain the evidence or
opportunity and why separate attention is useful. Fix ordinary in-scope findings directly.
Whiteboard contributions are optional; never invent an entry or weaken a deliverable to create one.

An entry is not an established defect, approved design, or automatic dependency. Continue the
assignment unless a demonstrated problem prevents its correctness or acceptance, and report that
specific impact. Recording a topic does not lift the product scope freeze. The whiteboard procedure
owns investigation and carryover; the [office procedure](virtual-office/README.md) owns sprint cleanup.

## Definition of done

A task is not complete because a diff exists or tests pass. It is complete when the intended
design is applied throughout its scope, the previous design is removed, and the result is
demonstrably better. Leave code that works and an explanation that lets the next contributor
understand and use it.

A change is complete only when every applicable row is true.

| Check | Required result |
| --- | --- |
| Intent | The problem, intended rule, owner, and scope are clear. |
| Correctness | Required behavior and failure behavior are implemented. |
| Simplicity | No simpler complete design is being avoided to reduce the diff. |
| Ownership | Each concept and policy has one owner. |
| Coherence | One canonical representation and operation path remain. |
| Abstraction | Shared rules use the smallest suitable abstraction. |
| Adoption | Every applicable producer, consumer, and composition path uses it. |
| Removal | Superseded code, aliases, fallbacks, configuration, and tests are deleted. |
| Interface | Public APIs are minimal, typed, and do not leak unrelated mechanisms. |
| Lifecycle | Resources, cancellation, shutdown, and bounds have explicit outcomes. |
| Evidence | Tests and checks prove the rule and important failure cases. |
| Documentation | A new reader can explain the purpose, follow a supported use, and understand important limits from the maintained prose, package README, and API docs. Claims match source and tests; links and examples work. See the [documentation practice](practices/documentation.md). |
| Final search | No conflicting implementation remains in the declared scope. |

A change is incomplete when the new and old designs both remain valid paths for the same responsibility.

## Choose verification for the change

Choose by what changed, including uncommitted work being integrated, rather than by filename or
task label. For mixed changes, combine the applicable checks; executable changes require the full
gate. Review the final diff,
run `git diff --check`, and report the checks actually run. A discussion or planned experiment
is not executed evidence.

| Change | Required verification before completion |
| --- | --- |
| Prose, planning, policy, or whiteboard notes only | Read for accuracy and consistency, check local links and document structure with the documentation contracts below, and inspect affected Markdown formatting. No full Rust gate is required. |
| Rust comments, rustdoc, or Rust documentation examples only | The prose checks, `cargo fmt --all -- --check`, affected package doctests, and warning-denying rustdoc. Run relevant owner tests when needed to substantiate a changed behavior explanation or example. |
| CLI/configuration examples inside documentation | The prose checks plus existing command parsing or production-reader checks for the changed examples. Run the relevant scenario when the claim depends on its execution, and state any unavailable prerequisites. |
| Executable code, tests, fixtures, manifests/lockfiles, schema/data files, runtime configuration, build scripts, or CI behavior | The full local gate below, plus any relevant focused or evidence suites. |

Documentation checks use the existing repository contract target:

```sh
cargo test -p milkdrift-evidence --test repository_contracts --all-features documentation::
```

For Rust documentation, replace `PACKAGE` with each affected package name:

```sh
cargo test -p PACKAGE --doc --all-features
RUSTDOCFLAGS='-D warnings' cargo doc -p PACKAGE --all-features --no-deps
```

Use the PowerShell environment handling below for rustdoc. Maintained CLI examples are checked by
`cargo test -p milkdrift-cli --all-features documentation::`; repository documentation contracts
also check maintained example documents through their production readers. A Markdown file used
as executable input still needs the executable-change checks. New compatibility, security, or
interoperability claims need the evidence required by their owner, regardless of file type.

For a documentation sprint, workers run the checks for their changed area and the coordinator
verifies the integrated result. Do not repeat unchanged successful checks at every phase handoff;
rerun when new changes, failures, or unresolved concerns affect their result. CI keeps its configured
gate; this policy determines local verification and does not change CI workflows.

## Full local gate

When required above, run from the repository root. Build the product daemon before isolated
application evidence tests and the shared process fixture before isolated daemon integration tests:

```sh
cargo build -p milkdrift-daemon --bin milkdrift-daemon \
  -p milkdrift-local-process --bin milkdrift-process-test-helper
```

The workspace gate also builds these targets. Hermetic external-evidence tests require Python 3
and Git on the harness's `PATH`; generated verifiers use the resolved absolute Git executable
because process adapters deliberately clear the child environment.
On Windows, put a real Python installation before the `WindowsApps` execution aliases on `PATH`.
The fixture resolves and hashes the interpreter file; an alias can fail executable resolution
before a scenario starts. Adjust the test shell's `PATH`, not the fixture's identity checks.

```sh
cargo fmt --all -- --check
cargo check --workspace --all-targets --all-features
cargo test --workspace --all-features --no-fail-fast
cargo clippy --workspace --all-targets --all-features -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --all-features --no-deps
cargo deny check
cargo machete
cargo tree --workspace --duplicates
cargo test --workspace --all-features -- --list
cargo test -p milkdrift-evidence --test repository_contracts --all-features
```

In PowerShell set `$env:RUSTDOCFLAGS = '-D warnings'` before `cargo doc` and restore its previous
value afterward. Use `cargo fmt --all` to apply formatting. Dependency changes require the full
deny/machete/duplicate-tree audit; pinned CI tool versions are in
[quality.yml](../../.github/workflows/quality.yml).

## Focused suites

Select the owning boundary while iterating. A focused pass does not replace the full gate for
changes that require it under the verification policy.

```sh
cargo test -p milkdrift-blueprint --test kernel --all-features
cargo test -p milkdrift-authority --all-features
cargo test -p milkdrift-control-protocol -p milkdrift-control-client --all-features
cargo test -p milkdrift-cli -p milkdrift-prompt-sequence --all-features
cargo test -p milkdrift-daemon --test control_plane --all-features
cargo test -p milkdrift-runtime --test durable_runtime --all-features
cargo test -p milkdrift-runtime --test structured_runtime --all-features
cargo test -p milkdrift-runtime --test causal_context --all-features
cargo test -p milkdrift-model --test contracts --all-features
cargo test -p milkdrift-model-provider --test mock_endpoints --all-features
cargo test -p milkdrift-control --all-features
cargo test -p milkdrift-persistence --all-features
cargo test -p milkdrift-redb-store --test contracts --all-features
cargo test -p milkdrift-redb-store --test application_state --all-features
cargo test -p milkdrift-peer-protocol --test protocol --all-features
cargo test -p milkdrift-peer-http --test peer_service --all-features
cargo test -p milkdrift-daemon --test two_daemon_peer --all-features
```

Tests use temporary stores, ephemeral loopback listeners, fixed identities/clocks, controlled
adapters, and private fixture credentials. Model mocks contact no provider. Local-process and
daemon integration suites share the byte-pinned Rust process helper. See
[status](../product/status.md) for executed platform evidence. Native source checks
or cross-compilation alone cannot establish runtime portability or filesystem durability.

Shared production-adapter conformance and host lifecycle checks:

```sh
cargo test -p milkdrift-capability-host --all-features
cargo test -p milkdrift-local-process --test process_execution --all-features
cargo test -p milkdrift-model-provider --test mock_endpoints --all-features model_endpoint_adapter_passes_shared_conformance -- --exact
cargo test -p milkdrift-control --test control_service --all-features workflow_control_adapter_passes_shared_conformance -- --exact
cargo test -p milkdrift-peer-http --lib --all-features remote::tests::remote_capability_adapter_passes_shared_conformance -- --exact
```

After Unix process-tree tests, `pgrep -af milkdrift-process-test-helper` can reveal surviving
helpers; exclude a match belonging to the inspection command itself. Fix lifecycle/coordination
defects instead of marking hanging or nondeterministic tests ignored.

## Fixtures and documentation contracts

Golden JSON lives under each owner's `tests/fixtures`. Change implementation and hand-reviewed
fixture together, run exact canonical re-encoding and refusal tests, and record reader-compatibility
changes in an ADR. Never regenerate fixtures merely to satisfy an assertion.

Maintained operator examples live under [examples](../../examples/operator/README.md), not fixtures.
Repository contracts check every maintained example through its production reader, canonical
documents and local links, source-derived versions, obsolete history paths, source size/cohesion,
dependency direction, and public re-export boundaries. CLI documentation command parsing runs in
the CLI test target; [actual-binary evidence](verification-evidence.md#actual-binary-scenarios)
checks executable composition, not just syntax.

Public API review follows [the API policy](../reference/public-api-policy.md): generate default
and all-feature inventories under `target/public-api`, trace actual consumers, and review
test/evidence features separately. Nightly rustdoc JSON tooling does not change the product's
pinned stable toolchain.

## Storage-boundary stress

Ordinary projection bounded-state checks remain non-ignored:

```sh
cargo test -p milkdrift-runtime --all-features projection::tests::bounded:: -- --nocapture
```

The five expensive persistence/operational cases are release-only manual/weekly evidence:

```sh
cargo test --release -p milkdrift-redb-store --test application_state --all-features release_receipt_longevity_crosses_many_hot_bounds_and_replays_after_restart -- --ignored --exact --nocapture
cargo test --release -p milkdrift-daemon --test two_daemon_peer --all-features peer_execution_retention_longevity_survives_turnover_and_restart -- --ignored --exact --nocapture
cargo test --release -p milkdrift-control --test control_service --all-features revision_and_lifecycle::release_controller_longevity_stops_once_across_checkpoints_and_restart -- --ignored --exact --nocapture
cargo test --release -p milkdrift-control --test control_service --all-features admission::release_controller_admission_longevity_turns_over_reservations_artifacts_and_restart -- --ignored --exact --nocapture
cargo test --release -p milkdrift-runtime --test structured_runtime --all-features lifecycle::historical_execution_frontier_stays_bounded_across_index_limit -- --ignored --exact --nocapture
```

[stress.yml](../../.github/workflows/stress.yml) also owns the exact selected 10,000-cycle projection
cases. Keep filters synchronized with test discovery; run reports and timings belong in CI
artifacts or ignored `target/` paths. [Verification evidence](verification-evidence.md) owns
mutation, benchmark, operational, and external qualification lanes.
