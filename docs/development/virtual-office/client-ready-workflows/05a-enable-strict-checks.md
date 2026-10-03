# 05a — Make important mistakes fail automatically

## Assignment

After 05, install a substantially stronger, maintained lint and repository-check policy. Check the
whole Rust workspace, not only the files touched by the CLI sprint. Existing violations are allowed
to become visible failures; do not lower standards to keep the first report green.

This assignment implements the checks. [05b](05b-fix-strict-check-findings.md) fixes the existing
code they expose. [06](06-integrated-acceptance-and-closeout.md) still owns final system acceptance.
A red lint handoff is permitted here only under the explicit conditions below. It is not permission
to leave broken checker code, unfinished integrations, or unexplained compiler failures.

Read [the insertion](linting-insertion.md), [the shared sprint rules](README.md),
[AGENTS.md](../../../../AGENTS.md), [architecture](../../../architecture.md),
[implementation practice](../../practices/implementation.md),
[documentation practice](../../practices/documentation.md),
[the work rules](../../workflow.md), and [the API policy](../../../reference/public-api-policy.md).
Follow the repository's reading order once per session; thereafter read the changed owners and
relevant handoff, not the whole sprint and every raw log again.

Use [the checklist](linting-checklist.md) as required coverage. Its named lints are a starting set,
not a maximum or permission to enable incompatible groups indiscriminately.

## 1. Establish the real baseline and coordinate the insertion

Start with `git status --short`. Record HEAD, existing edits, the accepted 05 handoff, Rust/Clippy
versions, relevant feature combinations, and the currently installed CI tool versions. Preserve
other contributors' work and use their actual implementations rather than reverting to the package's
reference commit. Do not run this concurrently with an agent changing the same manifests or owners.

Update only the necessary sprint coordination: insert 05a/05b after 05, make 06 depend on 05b, and
clarify that these steps may run full static checks but not the complete runtime journey. Reconcile
any instruction that would make each local checkpoint rerun the entire system. Do not disable,
rename away, or skip the existing CI gate. This assignment authorizes stronger static CI checks;
it does not authorize a larger hosted test matrix or paid infrastructure.

Inspect these existing owners before designing replacements:

- Root `Cargo.toml`, member manifests, `Cargo.lock`, `rust-toolchain.toml`, `.cargo/config.toml`,
  any Clippy configuration added meanwhile, and `deny.toml`.
- `.github/workflows/quality.yml` and the platform, mutation, stress, and benchmark workflows.
- `tools/evidence/tests/repository_contracts.rs` and its `documentation` / `source_size` children.
- Existing default-feature/test-support checks, public API review tooling, reusable conformance
  suites, and source-size exception handling.
- Production roots, resource/task owners, validated boundary types, and the actual public client
  boundary established in 01–05.

The inspected reference already forbids unsafe code, denies unused must-use values and several
Clippy panic/debug shortcuts, and runs Clippy with warnings denied in quality CI. All 24 reference
members inherit workspace lints. Preserve and strengthen these properties; do not report them as
new work. The existing manifest/source checks use some text searches: inspect their limitations
rather than assuming their presence proves that dependency or visibility rules cannot be bypassed.

Write a compact coverage table: rule, current enforcement, change required, affected scope, and
proof that the check detects a violation. Keep it in `handoffs/05a.md` while working; put durable
policy in existing canonical guidance and executable configuration, not a second architecture book.
Do not start by collecting a giant unstructured dump of every source file.

## 2. Choose a strict policy that has a reason

For every material rule, choose one treatment:

| Treatment | Meaning |
| --- | --- |
| Required check | A violation fails the normal strict command and CI. Existing violations become cleanup work. |
| Scoped required check | It fails in the owners where the rule is valid; justified exceptions stay narrow and reviewable. |
| Review finding | Detection is approximate or needs design judgment. Record evidence and a disposition, not a false guarantee. |
| Not applicable / unavailable | Explain precisely why. A missing stable lint must not be added as an unknown name or used to justify unrelated toolchain changes. |

Use the compiler and standard tools first. Keep project-specific code limited to rules those tools
cannot express. Select high-value additional pedantic, restriction, and nursery lints individually.
Audit the remaining compatible lints for relevant opportunities, recording significant exclusions.
Do not enable the entire restriction group. Do not turn on all nursery or pedantic warnings as a
permanent hard gate simply because their names sound stricter.

A high finding count is not grounds for lowering a well-chosen rule. A lint that encourages a worse
design is not grounds for changing the product to satisfy it. Record the actual trade-off, narrow
the scope where necessary, and leave a reviewed reason. Do not invent an exemption per finding.

Prefer `deny` for additional rules intended to fail normal checks and existing CI warning denial
for broad warning groups. Keep unconditional `forbid` for the existing unsafe-code policy, not every
lint: some otherwise useful lints need a justified local exception. Use Cargo lint-group priorities
correctly so a group cannot override a deliberate individual rule by table ordering. Keep the
product toolchain pinned; do not move it to nightly to acquire more lints.

## 3. Implement shared configuration and verify it is inherited

Own lint levels in root workspace tables. Own Clippy behavior settings in one root configuration
only when needed. Do not repeat the full lint policy in library roots, command aliases, and CI.
Every member must opt into workspace lints, including future members, examples, tools, test binaries,
and platform-specific packages. Discover members from Cargo rather than a second fixed member list.

Add a repository check for inheritance and suppression policy. Cover normal, development, build,
optional, target-specific, and renamed dependency declarations where the rule applies. Do not treat
`[dependencies]` as the only place that Cargo can introduce an edge. Detect attempts to evade checks
by leaving a backend package outside the workspace or adding an unauthorized nested workspace;
retain genuinely approved independent examples as explicit boundaries rather than banning them blindly.

Preserve safe existing defaults. Do not change release panic strategy, overflow behavior, wire
representations, supported platforms, or dependency features merely to make diagnostics quieter.
Make unknown/removed lint names, unrecognized configuration, unexpected conditional flags, and
unfulfilled expectations visible failures. Check tool support against the selected compiler's
help/documentation and small compile probes; today's online catalogue may be newer than the project.

Normal and all-feature builds are both required coverage. A workspace all-feature run can enable
helper features and hide broken defaults. Check the daemon and CLI as actual product roots without
the evidence package's feature union, and inspect default library surfaces separately. Enumerate
only supported additional feature/target combinations; do not create an exponential matrix or use
an unsupported empty-feature build to manufacture failures. Compiling all targets for the current
OS does not compile every OS-specific branch.

## 4. Tighten Rust and Clippy coverage

Implement the required and applicable scoped rows in the checklist. Cover at least these concerns:

- Accidentally discarded errors, futures, lock guards, process handles, and resource obligations.
- Panic-prone indexing/conversion and incorrect arithmetic at input, storage, size, and accounting
  boundaries; partial I/O and ambiguous file-creation behavior.
- Holding inappropriate guards across suspension, accidental task detachment, and unbounded
  channel constructors outside explicitly justified owners.
- Unreachable exports, unnecessary visibility, fallible infallible conversions, and APIs that let
  callers construct invalid values or silently discard required results.
- Redundant allocations/copies and unnecessary wrappers, without changing resource lifetimes or
  adding unsafe code, blanket atomics, unnecessary boxing, or speculative abstractions.
- Debug/stdout leakage, stale suppression attributes, public API documentation, and honest conditional
  compilation. Secret-bearing types must not receive an automatic derived `Debug` just to pass a lint.

Make intentional effects explicit. For example, a cancellation request is not proof that a worker
stopped; dropping a task handle may detach work rather than cancel it. A lint fix must not turn a
visible ignored result into `drop(result)` or `.ok()` and call the problem solved. A `#[must_use]`
annotation is useful but cannot establish that cleanup happened or that external work is finished.

Use typed Clippy `disallowed_methods`, `disallowed_types`, or `disallowed_macros` for suitable
forbidden API families, with checked canonical paths and reasons. Restrict them to actual dangerous
choices or owner boundaries. Raw process/database/network access is legitimate in its mechanism
owner; prohibit unauthorized alternative use rather than introducing a mandatory generic wrapper
around every standard-library call. Do not ban all locks, all `Arc`, all `Clone`, or all dynamic dispatch.

For custom invalid-across-await types, identify the actual type and demonstrate the hazard before
adding it. Lifetime protection and capacity permits may intentionally survive an await; they are not
all mutex guards. Preserve the accepted parent/child editing and resource-lifetime distinction.

## 5. Make the existing architecture checks harder to fool

Improve the existing repository-contract owner, not a parallel lint product. Parse TOML and use
Cargo metadata/package identities for dependency policy. Use a Rust parser or the existing token
machinery for source structure as appropriate. Put parser/tool dependencies only in development
tooling. A small, established parser is preferable to a home-grown pseudo-parser. Do not implement
a compiler plugin, nightly Clippy fork, or general rule DSL for this assignment.

Required improvements:

1. **Dependency direction and product reachability.** Check the actual permitted edges and forbidden
   reachability, resolving dependency aliases and workspace inheritance. Distinguish normal/build
   edges from legitimate dev-only edges such as runtime tests using redb. Check that product roots
   do not pull evidence/test-administration features into their ordinary build. Do not confuse a
   workspace-wide resolver union with a product-only feature graph.
2. **Thin clients.** Protect the CLI/client/protocol boundary reached after 05: no database or private
   runtime/control-service construction in clients; no concrete transport/storage/framework types
   in mechanism-neutral contracts. Preserve legitimate pure definition/wire dependencies. Do not
   forbid a future separate Svelte/Tauri client package, or add frontend tooling now.
3. **Private construction and exports.** Replace brittle exact-source assertions for important
   private constructors, test-only exports, and explicit re-exports with syntax-aware checks and/or
   real compile-pass/compile-fail consumer probes. Support legitimate macros and conditional exports.
   A parser sees syntax, not arbitrary type resolution or macro expansion; state the limits and use
   compiler evidence where the claim requires it.
4. **One owner for established mechanisms.** Keep known canonical identities, validation mechanics,
   schema/default ownership, and production adapter composition explicit. Use exact established
   rules, not a list of every function name or guessed duplicate-code detector. Reachability alone
   does not prove a registered adapter executes; keep composition/conformance tests for that fact.
5. **Source organization.** Preserve token-based source sizing and meaningful reviewed exceptions.
   Existing 1,000-line review guidance and 1,500-line backstop are not an invitation to compress code,
   remove comments, or create `part1` modules. Retain useful tests for the size checker. Report broad
   coupling, duplicate logic, and overly complex functions for review where exact enforcement is not sound.

The checks must reject real violations without rejecting equivalent valid formatting, comments,
string literals, grouped imports, qualified visibility, and legitimate target conditions. Add small
adversarial fixtures for dependency aliases, dependency tables, target/build edges, transitive helper
feature leakage, multiline imports, suppression hiding, and unrecognized packages. Where Cargo's
behavior is in question, run a minimal local fixture through Cargo rather than asserting only your
own normalized representation. Keep fixtures self-contained and offline-capable.

An exact interface test is not a license to freeze an obsolete public API. Review actual consumers
and the adopted architecture before updating its expectation. Remove the obsolete text test only
when its intended rule is covered by the replacement.

## 6. Check dependency and CI hygiene without building another platform

Retain `cargo deny`, `cargo machete`, lockfile discipline, and the existing audit configuration.
Review advisory coverage, allowed registries/git sources, wildcard requirements, licenses, duplicate
versions, and production use of development-only tooling. Do not add a second audit tool solely to
repeat `cargo deny`'s same advisory check. Treat unavoidable duplicate dependency versions by actual
cause and consumer, not a universal ban that forces unsafe or incompatible upgrades.

Verify new tool versions against official releases and the existing toolchain/CI environment.
Pin versions and actions through the project's existing convention; never invent a version or pin
a version-range guess. Avoid a broad dependency upgrade alongside this enforcement change. Any
necessary update must keep the lockfile, audit policy, and affected tests coherent.

Add bounded workflow syntax/expression checking and workflow-security checking using established
tools where not already provided. Actionlint and zizmor are the candidates to assess and integrate;
check their documented current CLI and supported configuration. Keep their distinct purposes, remove
redundant ad hoc checks, and do not enable speculative services. Inspect least-privilege tokens,
untrusted pull-request expressions, action pinning, credential persistence, artifact/cache trust,
and unsafe execution of untrusted PR code on persistent self-hosted machines. A linter warning is
not automatic authorization to change deployment permissions, secrets, runners, or billing settings.

Add an appropriately bounded secret-scanning check if equivalent coverage is absent. Gitleaks is a
candidate, not permission to upload code or secrets to a new service. Use redacted reports. Separate
public dummy fixtures from real exposures with exact justified exceptions, not exclusions of whole
source or test directories. Do not erase history or claim rotation; a real leaked credential needs
operator notification and separately authorized revocation. Keep a one-off historical audit separate
from routine changed-content checks so each commit does not scan unlimited history.

Use existing CI structure. Missing required executables must produce a clear failure, not a green
skip. Do not use `continue-on-error`, discard exit codes, or commit a lint cap in the required gate.
Do not add Svelte/npm tooling, hosted code-upload services, a new mandatory container, or a separate
security dashboard to this Rust-only backend assignment.

## 7. Prove the enforcement, then collect findings efficiently

Test each family of new checks with at least one failing specimen and a valid counterpart. Test
that exceptions are bounded and stale exceptions are detected. Verify an unknown lint/configuration
key cannot silently yield success. Also verify the runner propagates nonzero exit status and fails
when a required checker is unavailable. Do not write one enormous copied test for every upstream lint;
upstream Clippy is not ours to retest. Concentrate probes on the selected configuration and custom rules.

Build and test checker changes while they are still runnable; introducing new hard errors can block
compilation of dependent checker crates. Separate checker validation from the final policy activation
commit. After activation, use an explicitly labeled diagnostic-only command or temporary worktree
with lint severity capped to collect further findings when necessary. Such a run is never a passing
gate and must not be saved as a normal alias, CI path, source allowance, or permanent baseline exemption.
Compiler/type errors and unknown configuration remain distinct from expected lint failures.

Once the rule set has settled, run a workspace-wide strict static pass and collect machine-readable
diagnostics under ignored `target/client-ready-workflows/strict-checks/`. Group findings by actual
code owner and root cause. Deduplicate repeated diagnostics from feature/target variants, but retain
coverage metadata. A dependency that fails compilation can hide later diagnostics; report that
coverage as incomplete and let 05b continue discovery after fixing it. Do not claim an exhaustive
workspace scan merely because the first invocation finished.

The report must distinguish tool failure, compilation failure, confirmed policy violation, review
finding, known valid exception, and a check not executed. Include the exact source revision/diff,
versions, commands, targets/features, exit codes, summary counts, and log paths. Keep raw output out
of Git and chat. Do not manufacture a specific expected error count or a reduction target.

No full product scenario, whole-workspace runtime test suite, live-model call, Podman test, hardware
reconfiguration, stress campaign, or benchmark sweep here. Focused tests of the new checker are
required. Static workspace passes are allowed because the assignment is to establish static coverage.
Measure their cost and avoid rerunning unchanged heavy work after each edit.

## 8. Commit usable checkpoints and hand off honestly

Suggested commit boundaries, adjusted for real dependencies:

1. Sprint insertion plus the observed coverage/ownership decisions and necessary policy clarification.
2. Parsed repository checks and their self-tests, preserving existing behavior until the stricter
   policy is activated.
3. Shared lint configuration, inheritance/suppression checks, and explicit activation of the stronger
   gate. A known-red activation commit is authorized; name it honestly and retain the earlier clean point.
4. Static CI/tool integration and narrowly required documentation, with checker fixes as separate
   coherent commits where useful.

Stage explicit files/hunks and inspect the staged diff. Include related tests. Do not commit raw logs,
secrets, private model inputs, blanket suppressions, or someone else's changes. No automatic squash,
rebase, amend of earlier checkpoints, hard reset, force push, or ordinary push without authorization.

Write `handoffs/05a.md` with: commits; chosen required/scoped checks and major rejected candidates;
proof that enforcement works; the intentional red gate and observed findings; incomplete scan coverage;
exact 05b commands; decisions requiring judgment; and runtime cases reserved for 06. Keep it short
and link to ignored reports, not pasted logs. Existing source fixes needed to make the checker itself
correct belong here. Other violations belong in the immediate 05b cleanup, not an indefinite backlog.

Stop at **checks implemented and validated; product cleanup pending**. Do not call the sprint, product,
or new quality policy fully accepted until 05b and then 06 have completed their separate duties.
