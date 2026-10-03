# Add stricter checks before the final system test

## Run order

**Finish 00–05 → run 05a → run 05b → run 06.**

| Assignment | Finish with |
| --- | --- |
| [05a — Enable stricter checks](05a-enable-strict-checks.md) | Working enforcement, tested checkers, and an honest list of existing violations. The new strict gate may be red. |
| [05b — Fix what the checks find](05b-fix-strict-check-findings.md) | Fixed implementations and a passing strict gate, with focused regression tests. |
| [06 — Test the whole product](06-integrated-acceptance-and-closeout.md) | The complete product run, full test suite, and fixes for integration problems. |

Do not interrupt an agent partway through an implementation or run these assignments concurrently
with agents editing the same code. The files in this addition do not replace 00–06, their README,
or their handoffs. At the start of 05a, merge the two new assignments into the actual sprint table
and change 06's prerequisite to the accepted 05b handoff. Preserve any changes already made there.

Give the next available agent this instruction after 05:

```text
Execute docs/development/virtual-office/client-ready-workflows/05a-enable-strict-checks.md.
The linting addition changes the order to 00–05, 05a, 05b, then 06.
Do not run 06 before 05b has cleared the findings.
```

## What this adds

Make common mistakes fail automatically, and make important project rules executable rather than
leaving them only in prose. Use the Rust compiler, Clippy, dependency checks, and a small improvement
to the existing repository checks. Do not build a general lint framework.

The [checklist](linting-checklist.md) supplies concrete rules, scopes, pitfalls, and source references.
It is an implementation specification, not a ready-tested `Cargo.toml` to paste blindly. The agent
must verify every selected lint and configuration key against the toolchain actually in use.

Strict checks can expose unsafe patterns, but passing them does not prove freedom from deadlocks,
races, leaked resources, incorrect authorization, or broken recovery. Keep tests for those behaviors.
Some weak source-text tests may be replaced with better structural checks; a runtime test is not
redundant merely because related code is now linted.

## Scope and source

This addition was prepared from the reviewed `a04b55c0e0a3b56a1074da8861ab1d581e16a05c` source,
the revised client-ready sprint, and GitHub `main` at
`d8873cfe32700522347ea4d01858e4a5ddfa9749` on 2026-10-03. That main commit adds the revised
sprint. The inspected workspace pins Rust 1.95.0, and all 24 members inherit workspace lints.
There is no root Clippy behavior configuration in that snapshot. Existing checks already cover
unsafe code, common panic shortcuts, dependency policy, documentation, exports, and source size.
These are starting facts, not findings about changes the working agents have not pushed.

Use the actual checkout after 05; never reset to these references. Neither this package nor its
preparation enabled lints or ran Rust tests. 05a establishes the executable evidence.

## Keep the agreed working style

Use focused checks and small, coherent commits while implementing. Full static analysis is allowed
here because it is this assignment's purpose; the full runtime/product test stays in 06. Do not
push, squash, rewrite history, or disable existing CI without separate authorization.

Svelte remains the first GUI. This addition builds no frontend. Clients remain thin interfaces to
the daemon; tooling must protect that boundary without banning a future authorized Svelte client.
