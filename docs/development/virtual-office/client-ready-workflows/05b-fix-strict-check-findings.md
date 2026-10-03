# 05b — Fix the code without weakening the checks

## Assignment

After [05a](05a-enable-strict-checks.md), clear the stricter checks across the actual workspace.
Read `handoffs/05a.md`, [the checklist](linting-checklist.md), [the sprint rules](README.md),
[implementation practice](../../practices/implementation.md), and relevant architecture/API guidance.
Do not rerun 00 or reload every earlier log. Inspect the actual code and current diagnostics.

The goal is better ownership, error handling, and maintainability—not a green result obtained by
suppressing the evidence. Complete the underlying corrections and all affected callers. Do not
leave lint remediation to the final product-test agent.

## 1. Work from a reproducible starting point

Record HEAD, existing edits, policy version/commit, and the diagnostic commands. Preserve the clean
checkpoint before lint activation. Keep unrelated agents' changes out of this work; coordinate edits
rather than resetting their branches. Recheck findings that may have changed since 05a.

Distinguish actual compiler/type errors from lint failures. Fix early blocking owners first so later
packages can be analyzed. A stored first-pass report is a work list, not proof of complete coverage.
Re-run newly unblocked targets and add newly visible findings until the declared matrix is covered.
Do not stop at the packages that happened to compile in 05a.

## 2. Fix by rule and owner, not one diagnostic at a time

Use coherent batches. Start with resource/error correctness, then input/bounds and API ownership,
then structural and allocation/readability cleanup. Take harmless mechanical fixes separately from
semantic changes so a regression can be isolated. Use automated fixes only on a clean, bounded set
of files, inspect their diff, and reject changes that alter ownership or obscure intent.

When a diagnostic reveals repeated validation, cancellation, cleanup, or configuration code, fix
that shared responsibility at its existing owner and migrate matching callers. Do not paste the
same workaround at every diagnostic. Remove the obsolete alternative. A sound repair may cross
crates; a lint count is not a reason to add a crate, adapter, generic framework, or parallel API.

Protect these behaviors during cleanup:

| Area | Required care |
| --- | --- |
| Ignored failures | Decide which failures change the operation's result or require retained evidence. Logging and continuing is not automatically correct. Best-effort cleanup still needs truthful uncertainty when stop/removal cannot be proved. |
| Locks and async work | End critical sections before external code where the design permits it. Revalidate facts after a lock is released when needed. Do not replace a sync lock with an async lock merely to silence a warning, or introduce a lost-update race by moving work. |
| Tasks and processes | Keep explicit ownership, cancellation, joining/reaping, and bounded shutdown. Dropping a handle, adding `kill_on_drop`, or acknowledging cancellation does not establish durable external quiescence. Preserve independently supervised services. |
| Arithmetic and casts | Use checked conversion/arithmetic where invalid values must fail. Choose wrapping or saturation only when the domain actually requires it. Do not clamp budgets, charges, lengths, IDs, or counters to conceal invalid inputs. |
| Indexing | Prefer iteration, exact-size types, and checked access at trust boundaries. Do not replace indexing with `get().unwrap()` or duplicate validation everywhere. Proven internal indexing may use a narrowly justified expectation. |
| Public/private types | Keep public contracts usable and validating construction intact. Do not expose fields, add `Default` for required identity/authority, or derive permissive deserialization to simplify a lint fix. |
| Clones and allocations | Preserve lifetimes, task ownership, snapshot semantics, and cancellation state. Do not remove a clone by introducing broad mutable sharing; do not box everything to meet a size warning. |
| Documentation | Explain the actual caller obligation or failure. Prefer making an unused item private/removing it over documenting an unnecessary public API. Never add repetitive boilerplate or derived secret-revealing diagnostics just to satisfy a lint. |
| Tests | Keep failure, authority, recovery, resource lifetime, and wire-format assertions. Do not change expected values to match the faulty implementation or remove a test because the code now passes Clippy. |

Make any demonstrated race or leak fix immediately and add a focused regression. Do not hide it in
an unrelated mechanical cleanup commit. If a real boundary must change, migrate producers, consumers,
fixtures, readers, docs, and errors together. Preserve supported durable replay; an API cleanup is
not authority to reinterpret existing records. Seek no new product features in this assignment.

## 3. Review exceptions deliberately

First prefer clearer code or a better type. If a selected rule genuinely conflicts with required
behavior, narrow its scope or use the smallest supported `#[expect(..., reason = "...")]` with an
explanation of the invariant and evidence. Account for conditional compilation; an expectation must
not become unfulfilled in another supported build. Use a justified `allow` only when `expect` would
misrepresent legitimately configuration-dependent behavior.

Do not allow whole lint groups, files/directories, production crates, or all test code to erase the
backlog. Do not give the checker an ever-growing list of current violations. Do not move behavior
behind a macro, `.ok()`, `drop`, ignored variable, or wrapper to hide it. A narrowly scoped existing
exception may be sound; review its actual reason rather than automatically deleting it.

Any change to the agreed policy must be separate and explained in the handoff, with a failing/valid
fixture showing why the new policy is more accurate. Count reduction is not justification. A raw
number-of-arguments or line limit must not produce anonymous `Context` bags or meaningless modules.

## 4. Preserve the safety properties static analysis cannot prove

Trace changed lifetime/error paths through ordinary entry, refusal, early return, panic containment,
cancellation, and restart where relevant. Keep resource-use protection until there is the required
stop evidence. In particular, the waiting parent's lifetime hold and the child's editing access
must remain distinct; a lint cleanup must not release both to make a test stop hanging.

For changed concurrent logic, write a deterministic interleaving/fault test that reaches the real
owner. Prefer existing clocks, barriers, hooks, and stores. Sleep-based timing luck is not evidence.
Use drop counters/weak-reference probes and repeated bounded turnover where useful for a leak fix;
retained audit/history is not automatically a memory leak. Verify resources against their promised
lifetime rather than requiring everything to disappear when one request ends.

Loom, Miri, sanitizers, and whole-system memory profiling are not ordinary lints. Use the checklist's
selection rules. A small additional checker run is justified by a specific changed owner and a
known-supported setup; do not rebuild the product around a model checker or make stable builds
require nightly. Record focused runtime cases that 06 must integrate.

Structural-test deletion is allowed only when the replacement checks the same rule more robustly
and has negative coverage. Keep a short old-check → replacement mapping in the handoff. Do not
reduce runtime test coverage as a token-saving measure.

## 5. Validate efficiently and commit as you go

After a coherent change, run the relevant package's check/Clippy command and focused tests, including
affected callers where its API changed. Ensure a filtered test command actually discovers and runs
the intended tests. Run changed checker self-tests and dependency audits when their inputs change.
Keep outputs under ignored `target/client-ready-workflows/strict-checks/`.

Commit each complete owner-level fix with its tests before the next distinct change. Separate
mechanical edits, semantic fixes, and exceptional policy corrections. Do not save all cleanup for
one giant commit or split an API change from the consumers needed to compile it. Stage explicitly,
review the diff, and retain previous checkpoints. No automatic squashing, history rewriting, or
pushes. An intermediate commit may remain red for unrelated recorded findings; state the local
checks passed and the remaining strict failures rather than claiming full acceptance.

When the findings are cleared, run the complete selected **static** matrix once on the settled code:
compiler/Clippy, repository checks, warning-denying documentation checks as selected, dependency
checks, and workflow/secret checks. Include product-only defaults, workspace all-target/all-feature
coverage, and the finite supported target/feature coverage established in 05a. Diagnose unavailable
platform/tool prerequisites explicitly; do not substitute a skip for a required result or claim
cross-platform runtime verification from cross-compilation. Reuse artifacts, not stale conclusions.

Run targeted regressions for semantic repairs; do not run the full workspace runtime suite, full
release-notes/Slotbook journey, live models, hardware lane, or broad stress tests here. Those remain
06's job. If final static checks reveal more work, return to focused fixes, then rerun affected
static coverage on the resulting code. Do not re-run every long command after each trivial edit.

## 6. Hand off a clean static gate to 06

Update canonical configuration/policy documentation, not a separate permanent findings catalogue.
Write `handoffs/05b.md` with the exact tested commit/diff, commits by fix, remaining legitimate
exceptions and their reasons, all static results, runtime regressions added, replaced structural
tests, and the final-system cases 06 must run. Link raw reports instead of pasting them.

Finish when the required strict checks pass, custom checkers have working negative tests, there
are no known unresolved in-scope defects, and reviewed findings have explicit dispositions. Missing
required tools or targets are blockers, not successful results. Do not weaken this definition by
moving cleanup into 06. The final agent will still run the complete system and fix integration
problems on the final source; this handoff does not claim the product has already passed that test.
