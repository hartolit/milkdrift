# Strict checks: what to enforce and what not to pretend

This is the coverage specification for [05a](05a-enable-strict-checks.md) and
[05b](05b-fix-strict-check-findings.md). Keep enduring decisions in the existing work/practice/API
owners and actual configuration. Delete this temporary specification during normal sprint closeout
only after those decisions have a maintained home.

The proposed lint names below refer to Rust and Clippy, including the 1.95 catalogue consulted for
this plan. Verify the exact names, stability, default groups, behavior, and configuration against
the executing checkout's pinned compiler. A rule already active through a group need not be listed
twice. An unavailable rule needs an explicit disposition, not an invented spelling or silent skip.

## 1. Compiler and coverage baseline

**Required.** Preserve `unsafe_code = "forbid"`, `unused_must_use`, current documentation requirements,
and ordinary compiler warnings in the strict gate. Keep existing `rust_2018_idioms` coverage where
applicable; its name is not a reason to remove useful lints from an edition-2024 project.

Add or confirm these specific coverage decisions:

| Check | Scope and purpose |
| --- | --- |
| `unknown_lints`, `renamed_and_removed_lints`, `unfulfilled_lint_expectations` | Configuration/suppression mistakes must fail the strict run. Verify both active and conditional expectations in their supported builds. |
| `unexpected_cfgs` | Declare actual custom flags and feature values. Do not allow the lint globally to accommodate a misspelled feature or an undeclared test flag. |
| `unreachable_pub` | Require narrower visibility for an item that cannot be part of the declared public API. Do not make a parent module public just to quiet this diagnostic. |
| `unused_lifetimes`, `unused_macro_rules` | Remove stale type parameters and macro branches after checking actual feature/target coverage. |
| `trivial_casts`, `trivial_numeric_casts` | Make unnecessary casts visible; do not remove a cast needed by a different supported target without checking it. |
| `dead_code`, unused imports/variables, deprecated uses, ineffective drops | Preserve normal compiler coverage. Do not disable it across crates to make dormant alternatives compile. |
| `let_underscore_lock` | Retain the compiler's standard-lock protection. Also keep Clippy's corresponding coverage for lock implementations it supports. |

**Assess deliberately:** `unnameable_types`, `unused_qualifications`, `explicit_outlives_requirements`,
`unused_crate_dependencies`, and any newly available binary-specific dead-code check. Some produce
noise for legitimate public types, macro-generated code, or packages with several targets. Give
high-value cases a checked scope; do not enable them solely to fill a list. Do not use nightly-only
`must_not_suspend` as a promised stable solution.

Default compiler checks cannot identify every unused public API. Preserve the public consumer
review and default-feature inventory instead of claiming `dead_code` replaces them.

## 2. Clippy's normal coverage stays enabled

**Required.** Keep `clippy::all` under warning-denying verification, without broad source overrides.
This supplies normal correctness, suspicious-code, complexity, performance, and style checks.
Retain the existing explicit restrictions on `dbg_macro`, `expect_used`, `panic`, `todo`,
`unimplemented`, `unwrap_used`, and `wildcard_imports`.

Check that existing configuration or attributes do not suppress relevant normal checks, including:

```text
await_holding_lock                  await_holding_refcell_ref
await_holding_invalid_type          let_underscore_future
let_underscore_lock                zombie_processes
arc_with_non_send_sync              async_yields_async
unused_io_amount                   suspicious_open_options
ineffective_open_options           suspicious_command_arg_space
clone_on_copy                      unnecessary_to_owned
needless_collect                   large_enum_variant
```

This is a coverage reminder, not a second copied lint list to maintain in three places. Do not
claim these were all newly enabled. `clippy::all` is not every Clippy lint; it omits opt-in groups.

## 3. Additional high-value rules

Enable the following compatible rules unless a specific documented conflict calls for a narrower
scope. Existing violations are cleanup work, not a reason for excluding the rule.
Use the current spelling on the pinned compiler; older lint aliases are not an acceptable final configuration.

| Family and named lints | Intended result and important limit |
| --- | --- |
| `mem_forget`; `let_underscore_must_use` | Make suppression of destructors and ignored important outcomes visible. Explicit `drop`, `.ok()`, leaked references, and detached handles still need review; these checks do not prove leak freedom. |
| `fallible_impl_from` | Do not disguise a fallible conversion as `From`. Correct the API and callers rather than replace a panic with a default value. |
| `unchecked_time_subtraction` | Handle potentially underflowing instant/duration subtraction explicitly according to the clock/lease contract; do not turn underflow into an unlimited wait. |
| `cast_lossless`; `cast_possible_truncation`; `cast_possible_wrap`; `cast_sign_loss`; `cast_precision_loss` | Review numeric conversion at size, accounting, time, wire, and platform boundaries. Use checked conversion or exact invariant evidence rather than silent clamps. |
| `match_wildcard_for_single_variants` | Make closed state handling explicit when the last named case is known. This is not a replacement for reviewing broad wildcard arms. |
| `missing_errors_doc`; `missing_panics_doc` | Explain meaningful caller-visible failures of public APIs. Do not document accidental panics as supported behavior or repeat every internal error variant. |
| `must_use_candidate`; `return_self_not_must_use` | Inspect callers that discard returned values. Mark semantically important values, especially ownership-bearing results; do not annotate every getter without judgment. |
| `allow_attributes_without_reason` | Require an actual attribute reason, not merely a nearby comment, for suppressions the tool covers. Extend repository checks to required `expect`/conditional cases if coverage is incomplete. |

Clippy's restrictions and pedantic rules are not a single severity level. A permitted exception must
explain the exact valid pattern and its scope. Test-only panic fixtures are different from production
error handling; do not blanket-disable all quality checks under `cfg(test)`.

## 4. Scoped rules for risky boundaries

**Required assessment and implementation where applicable.** Record which owner receives each
rule, why that scope is sound, and any essential exception.

| Rule/configuration | Use it here; avoid this misuse |
| --- | --- |
| `indexing_slicing` | Reject unchecked indexing/slicing at externally influenced input, parser, artifact, and storage boundaries. Prefer iteration/exact-size types internally. Do not rewrite every proven access into an unwrap or add redundant checks after validated construction. |
| `arithmetic_side_effects` | Apply at length/offset/budget/sequence/time arithmetic where overflow changes correctness. Deliberate wrapping or saturation needs domain meaning. Do not make mathematical code unreadable or silently saturate accounting to satisfy the lint. |
| `wildcard_enum_match_arm` | Review project-owned closed execution/authority/resource states. Keep necessary catch-all behavior for external non-exhaustive contracts and explicit unknown-version refusal. |
| `exit`, `print_stdout`, `print_stderr` | Library owners must not terminate the host or write unstructured output. Permit the CLI's actual presentation/exit owner and explicit test/evidence binaries at reviewed locations. Do not ban proper tracing. |
| `disallowed_methods`, `disallowed_types`, `disallowed_macros` | Use type-resolved checks for prohibited primitive/API choices and known direct bypasses. Canonical paths and scope must be proven by negative examples, including aliases. |
| `await-holding-invalid-types` | Populate only with actual types whose guards must not span suspension. Never put every resource hold, generation permit, or async-safe guard on the list. |
| `allow_attributes` | Assess as an aid to replacing stale-prone `allow` with `expect`. Do not force `expect` where the lint legitimately appears only on another target, or use it to hide a policy violation. |

Review exact API families before configuring restrictions:

- `std::mem::forget`, `Box::leak`, `Vec::leak`, and manual lifetime escapes. They need an explicit
  ownership justification; safe Rust permitting an operation does not make it appropriate here.
- Unbounded channels such as `std::sync::mpsc::channel`, Tokio unbounded channels, and equivalent
  constructors of dependencies actually used. Assess queue capacity, producer bounds, and exhaustion
  semantics; a bounded wrapper must enforce the actual allocation, not just report a later limit.
- Detached task/thread creation and raw process creation outside owned executor/supervisor/harness
  locations. Retaining a variable named `handle` is not evidence that it will be joined or reaped.
- Hidden environment/global/current-directory access, raw storage/network entry, or independent
  time/random generation in owners documented to receive those facts explicitly. Native adapters,
  configuration compilation, and authorized administration legitimately use system APIs.
- File operations whose safety depends on checking a path and then acting later. Static hints are
  useful, but symlink/race-resistant behavior still needs mechanism-specific code and tests.

Do not create a universal wrapper for these APIs. Use the existing owner, a small private helper
when justified, or a narrow reviewed exception. The purpose is to stop unowned use, not to add layers.

## 5. Review code decay without creating lint-shaped architecture

Review these candidates against the actual code; promote those with a useful signal and manageable
exceptions. The review must cover them, but they are not all automatically global hard errors:

```text
redundant_clone                     implicit_clone
needless_pass_by_value              needless_pass_by_ref_mut
trivially_copy_pass_by_ref          ref_option
unnecessary_wraps                  unnecessary_box_returns
unused_self                        unused_async
large_stack_arrays                 large_futures
large_types_passed_by_value         option_option
struct_excessive_bools              fn_params_excessive_bools
match_same_arms                     map_unwrap_or
significant_drop_in_scrutinee       significant_drop_tightening
```

Use a bounded diagnostic survey of additional compatible pedantic/nursery rules, not a permanent
blanket gate. Configuration thresholds are review triggers, not design truths. Measure or explain
a tighter threshold; do not invent tiny limits for functions, arguments, generics, nesting, arrays,
or enums to guarantee a huge error count.

A legitimate trait implementation may need an apparently unused parameter; a task future may be
large for a measured reason; a `Clone` may preserve immutable request identity. Removing an error
wrapper may lose useful classification. Inspect these meanings before accepting a suggestion.
The significant-drop lints have specific type/annotation coverage; evaluate them on actual lock or
resource owners. Do not shorten a deliberately retained lifetime hold merely because it appears
unused after its last reference. These lints are not general lock-order analysis.

Do not automatically enable entire restriction/nursery groups, mandatory `Copy`/`Debug` on all types,
a universal `as_conversions` ban, bans on every arithmetic operation, forced atomics, blanket boxing,
or naming/document-length/style rules unrelated to a maintainable Rust codebase. Do not enable
opposing stylistic requirements together.

## 6. Project rules that deserve structural checks

The current implementation practice explicitly asks for enforceable boundaries. Give each durable
rule one executable owner; do not mirror the architecture's entire prose in a second configuration.

| Project rule | Suitable enforcement | What must remain outside the claim |
| --- | --- | --- |
| Every backend member inherits shared lint/version policy | Parsed manifests and Cargo member discovery; test missing inheritance and hidden nested manifests | Merely being a member does not establish correct behavior |
| Dependencies point toward their allowed owners | Declared/resolved package identities with aliases, kinds, targets and features; negative Cargo fixtures | A dependency edge alone does not prove correct runtime delegation |
| CLI/Svelte/future clients stay thin | Product graph restrictions, private APIs, external-client tests already owned by the sprint | A graph check cannot recognize every duplicated business rule |
| Test-only facilities stay out of normal product builds | Product-only feature checks and small consumer compile probes, separate from the all-feature evidence graph | A workspace union is not a faithful product feature report |
| Invariant-bearing values use validated construction | Private fields and constructors; specific compile-fail boundary probes; production-reader tests | A derives/AST check cannot prove a constructor validates correctly |
| One owner for schema identity, shared validation, and resource transitions | Maintain the established exact rules; use private APIs and semantic owner tests | Global duplicate-string or magic-number detection is not an ownership proof |
| Adapters are actually adopted | Required product reachability plus existing composition and shared conformance tests | An unused normal dependency can make reachability pass |
| No hidden resource creation outside owned mechanisms | Type-resolved API restrictions and narrow ownership exceptions | Static analysis cannot prove all handles stop or resources are freed |
| Explicit exports and real module boundaries | Syntax-aware use/visibility checks; existing token-count/cohesion policy | A line count does not prove cohesion and must not encourage minification |
| Exceptions remain justified | Attribute/configuration parsing, exact scope/reason, stale-exception tests | An arbitrary sentence is not a sound rationale |
| Maintained docs and examples remain usable | Existing link, production-reader, CLI example and rustdoc checks | `missing_docs` cannot determine whether an explanation teaches |

Build valid and invalid examples for these checks. Valid cases include reformatted TOML, grouped
imports, comments containing forbidden-looking code, strings with `pub use`, target-specific
allowances, and legitimate macro declarations. Invalid cases include renamed forbidden packages,
forbidden build edges, hidden optional edges, transitive enabling of helper features, a newly public
constructor bypassing a declared private construction boundary, and a broad conditional lint suppression.

Use typed/compiler checks for facts requiring name resolution. A syntax checker cannot magically
resolve arbitrary aliases, trait calls, proc macros, or every conditional compilation state. Define
its supported representation, reject malformed inputs, and document the remaining review boundary.
Do not quietly skip an unfamiliar file or return success after a parse error.

## 7. Dependency, workflow, and secret checks

Keep the existing `cargo deny` policy authoritative for advisories, sources, licenses, and bans, and
`cargo machete` for likely unused dependencies. Audit actual features/defaults and lockfile changes.
Do not make every duplicate transitive version fatal; review incompatible major versions and
intentional coexistence. Any audit suppression needs an exact package/advisory, reason, and a
review/removal condition. No broad permanent ignore lists.

Integrate missing static coverage for GitHub Actions syntax/expressions (actionlint) and workflow
security (zizmor) after verifying compatibility and exact commands. These have different jobs;
avoid rebuilding their rules with our own YAML substring checks. Check their handling of the
repository's real self-hosted labels, pinned action SHAs, shell snippets, and permissions.

Add secret scanning with an established local tool such as Gitleaks if no equivalent exists.
Do not print discovered credentials. Restrict fixture exceptions precisely and verify that a
synthetic planted secret elsewhere still fails. No blanket exclusion of `tests/`, examples,
configuration, or Markdown. History cleanup/credential rotation remains an operator action, not
a lint-autofix. Keep routine scanning bounded to the chosen CI event/change; use a separate bounded
initial audit for existing tracked content/history. Avoid duplicate overlapping secret scanners.

External maintained tools may use another implementation language; repository-owned check logic
stays Rust. Do not add Node/ESLint, Svelte, Tauri, or Iced dependencies before frontend work is
actually authorized. Later frontend linting belongs to its own shell, not the daemon's domain crates.

## 8. What still needs runtime evidence

| Risk | Static help | Required behavior evidence when affected |
| --- | --- | --- |
| Mutex deadlock or wrong lock ordering | Detect some guards across suspension; explicit owners | Deterministic competing-operation/interleaving tests |
| Wrong cancellation or parent/child editing transfer | Must-use checks, restricted spawn entry, typed claims | Entry/stop/fencing/restart tests against the existing owner |
| Reference cycles or growing retained state | Discourage manual leaks and needless copies | Drop/weak-reference checks, bounded turnover, storage/retention tests |
| Unfinished task/process after early return | Handle-use diagnostics and task ownership restrictions | Failure/cancellation cleanup and real process-tree tests where relevant |
| TOCTOU, broken idempotency, or incorrect authority | Narrow API and input checks | Concurrent/fault/refusal/replay tests; appropriate real OS enforcement |
| Logic/security errors in valid code | Some suspicious-code diagnostics | Independent assertions against the promised contract |

Loom can be valuable for a small concurrency owner using its modeled synchronization; it does not
analyze arbitrary unchanged Tokio/OS code. Miri runs supported Rust under an interpreter and may
find undefined behavior or leaks on executed paths; it is not a proof of product correctness and
has platform/foreign-call limits. Sanitizers and memory profiling also require execution. Assess
these tools separately and use them only for a concrete useful seam; do not start a mandatory
nightly, container, model, or hardware campaign as part of enabling lints.

05b runs focused tests for its fixes. 06 runs the complete system and existing required failure
coverage. Do not delete those tests in exchange for more compiler flags. Record any proposed
replacement of a structural test by the stronger check that now owns the same assertion.

## 9. Verification commands and reporting

These are known command shapes for the inspected workspace, not proof they pass the later checkout.
Use the actual toolchain and supported feature matrix chosen in 05a. A Rust-owned helper/alias may
coordinate existing commands, but do not add another build system or copy the lint configuration
into its arguments.

```sh
# Static checks: compile/analyze; do not run the full runtime test suite.
cargo fmt --all -- --check
cargo check --locked -p milkdrift-daemon -p milkdrift-cli --bins
cargo clippy --locked -p milkdrift-daemon -p milkdrift-cli --bins -- -D warnings
cargo check --locked --workspace --all-targets --all-features
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings

# Focused repository-rule tests, not the whole product suite.
cargo test --locked -p milkdrift-evidence --test repository_contracts --all-features

# Existing dependency hygiene.
cargo deny check
cargo machete
cargo tree --workspace --duplicates
```

Use the current work guide's warning-denying rustdoc commands and public-API checks where selected.
Default/all-feature library coverage and actual supported platform branches need their corresponding
checks; the two product commands above are not the whole matrix. Verify target/test discovery and
checker exit statuses. Pin any added tool and declare its invocation in CI and the existing work guide.

Collect structured diagnostics and summarize by rule and code owner. Avoid source-line baselines
that silently authorize every existing violation. A red 05a report is temporary execution evidence;
05b must clear required failures. The normal gate must always fail for a real violation, missing tool,
configuration error, or incomplete mandatory scan, rather than quietly convert it into a warning.

## 10. Sources and limits of this specification

Repository sources reviewed for this addition:

- [Agent rules](../../../../AGENTS.md), [architecture](../../../architecture.md),
  [implementation practice](../../practices/implementation.md),
  [documentation practice](../../practices/documentation.md),
  [work/verification rules](../../workflow.md), and
  [public API policy](../../../reference/public-api-policy.md).
- Root manifest/toolchain/audit configuration, quality/platform workflows, repository-contract
  tests, and their source-size/documentation helpers in the `a04b55c` reference; the `d8873cf`
  main commit introduces the revised sprint. Recheck these sources after the working agents finish 05.

Primary tool documentation, consulted 2026-10-03:

- [Clippy groups and their intended use](https://doc.rust-lang.org/stable/clippy/).
- [Clippy 1.95 lint catalogue](https://rust-lang.github.io/rust-clippy/rust-1.95.0/index.html)
  and [configuration](https://doc.rust-lang.org/clippy/configuration.html).
- [Cargo workspace lint inheritance](https://doc.rust-lang.org/cargo/reference/workspaces.html#the-lints-table)
  and [manifest lint priorities](https://doc.rust-lang.org/cargo/reference/manifest.html#the-lints-section).
- [Rust diagnostic attributes](https://doc.rust-lang.org/reference/attributes/diagnostics.html)
  and [compiler lint listing](https://doc.rust-lang.org/rustc/lints/listing/allowed-by-default.html).
- [Tokio task-handle behavior](https://docs.rs/tokio/latest/tokio/task/struct.JoinHandle.html)
  and [Rust reference-cycle limits](https://doc.rust-lang.org/book/ch15-06-reference-cycles.html).
- [cargo-deny checks](https://embarkstudios.github.io/cargo-deny/checks/index.html),
  [actionlint](https://github.com/rhysd/actionlint), [zizmor](https://docs.zizmor.sh/),
  and [Gitleaks](https://github.com/gitleaks/gitleaks).
- [Loom](https://github.com/tokio-rs/loom) and [Miri](https://github.com/rust-lang/miri).

This document selects work and explains the policy boundaries. It does not certify that the selected
configuration compiles on the later source, that these tools found a particular defect in Milkdrift,
or that adding lints eliminates the runtime risks listed above. The execution prompts require that
verification and an honest handoff.
