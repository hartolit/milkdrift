# Phase 04 handoff

Codex completed the applications-and-guides area on 2026-09-08, starting from clean commit
`0210dbd`. Changes are Markdown, Rust documentation/private comments, and two supported doctests.
Executable behavior, public names/visibility, dependencies, serialized data, fixtures, test
assertions, and CI are unchanged. The full assigned area is ready for independent phase 05 review;
no package or maintained-prose review is deferred. Work was not delegated. Phase 05 must address
the inherited repository size-policy failure described below before integrated acceptance.

## Coverage and reader review

Reviewed introductions, API docs, and consequential private/test comments across all five packages,
following configuration and credentials through CLI/client calls, daemon admission, command
receipts, execution, and inspection. Intentionally retained clear argument/field descriptions,
codec and projection helpers, command-family routing, and focused failure-test explanations.
Clap docstrings that generate application help were preserved. All 22 current workspace packages
now have a README.

- **Control protocol:** added a package guide connecting commands, accepted results, reads,
  pagination, streams, and layouts. Added a round-trip envelope doctest. Clarified current-minor
  negotiation, cursor inspection versus authenticated validation, and current versus historical
  attempt views.
- **Control client:** added setup and operation guidance with a no-network construction doctest.
  Explained safe-read retries, exact mutation recovery, per-request versus overall deadlines,
  consumer-owned stream acknowledgement/reconnect decisions, and caller verification of downloaded
  artifact bytes.
- **Daemon:** added an operator/contributor entry and request flow. Connected configuration,
  authentication, owner queue, command/read families, receipts, startup, maintenance, peers,
  retained work, and shutdown. Removed stale immediate-config-revocation and reload-after-peer-
  revocation wording.
- **CLI:** added a complete command-family guide, scripting and interruption behavior, explicit
  pages, file cleanup, and source ownership. Explained why rerunning `sequence remediate` after
  a lost reply can reconstruct a different proposal from fresh state, requiring inspection before
  another submission.
- **Evidence:** added a chooser for every binary and the benchmark, linking maintained commands
  and prerequisites. Reviewed shared child/application, storage, context, adapter, peer, daemon,
  fixture, report, mutation, and external scenario owners. Distinguished deterministic harness
  checks, model-only operation, real combined qualification, abrupt settled-work restart, and
  the separate Unix graceful-signal lane.

Reviewed root entry documents, the docs index, product/architecture owners, development guidance,
all maintained guides/operations/references, the ADR index and decisions, and prose/comments in
`examples/` and `.github/`. The reading path now leads operators to setup and contributors to
owners. Daemon operations explains observable recovery and retention choices instead of duplicating
physical storage inventories. The vision links one terminology reference; ADR version statements
distinguish adoption history from current compatibility. Status, roadmap ordering, engineering
policy, workflow commands, schema data, executable Markdown examples, and workflow files were
intentionally retained. Active sprint records and whiteboard contributions were preserved.

The final consistency review included the [phase 01](01-handoff.md), [phase 02](02-handoff.md), and
[phase 03](03-handoff.md) results and their package guides. Shared prose preserves their context,
process, peer, and controller qualifications. Inspected generated rustdoc text and local links for
all five entries, both examples, and selected cursor/configuration/application APIs. Reviewed
Markdown tables, fences, and diagrams in source. No browser visual review or independent reader
acceptance is claimed.

## Checks and evidence

Checks apply to the documentation-only working tree over `0210dbd`. Owner tests preceded final
prose refinements; their executable inputs stayed unchanged. Detailed logs and the scope/rendered
text audits are under ignored `target/documentation-clarity/phase04/`.

| Check | Result |
| --- | --- |
| Build daemon, CLI, and local-process test helper | Passed. |
| Grouped five-package `cargo test --all-features --no-fail-fast`, `CARGO_INCREMENTAL=0`, `-j 2` | 111 unit/integration tests passed; one existing manual daemon test stayed ignored. One repository cohesion-policy test failed on unchanged earlier-phase files; see below. |
| Grouped library `cargo test --doc --all-features` | Both doctests passed. The CLI is a binary with no library doctest target. |
| Grouped five-package `cargo doc --all-features --no-deps`, `RUSTDOCFLAGS='-D warnings'` | Passed; client/evidence rustdoc refreshed after final comment refinements. |
| `cargo test -p milkdrift-evidence --test repository_contracts --all-features documentation::` | All 8 documentation contracts passed, including maintained links, structure, version cells, and production example readers. |
| `cargo test -p milkdrift-cli --all-features documentation::` | Both production-parser documentation tests passed. |
| `cargo fmt --all -- --check`, `git diff --check`, and scope audit | Passed; all 43 changed Rust files preserve non-comment, nonblank code. |

The first grouped test build failed inside rustc 1.95.0 with import-resolution internal errors and
missing compiled-library artifacts before any tests ran. The retry above disabled incremental
compilation and used two jobs. It completed the suites, exposing the separate repository-policy
failure rather than another compiler failure. The initial and retry logs are retained.

`production_sources_over_the_review_threshold_have_exact_bounded_exceptions` fails for eight
unchanged files: local-process `config`, model-provider `adapter`, blueprint `validation`, capability
`descriptor` and `invocation`, control `service`, persistence `journal/commit`, and prompt-sequence
`compiler`. Their contents and line counts match `0210dbd`; none is edited by phase 04. Six exceed
existing exception ceilings and two lack exceptions after crossing the review threshold. The
phase 05 integrated review owns resolving this editorial integration failure under the standing
cohesion policy; test assertions and exception ceilings were not changed to make this pass.

The full executable gate was not required or run. These are local Windows checks with controlled
process/model/peer fixtures. They add no real-agent/provider, Unix process/signal, power-loss,
arbitrary deployment, or exactly-once external-effect qualification.

## Unresolved implementation findings

Preserved the source-derived [context-policy enforcement](../whiteboard/issues/context-policy-enforcement.md),
[process reporting cleanup](../whiteboard/issues/process-reporting-cleanup.md), and
[prompt-sequence version label](../whiteboard/issues/prompt-sequence-version-label.md) findings.
The maintained guides now avoid claims that those gaps invalidate. Their executable repairs and
regression evidence remain separate work; this phase changes no issue disposition or product gate.
