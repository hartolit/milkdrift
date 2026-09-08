# Phase 03 handoff

Codex completed the external-capabilities area on 2026-09-08, starting from clean commit
`52cf340`. Changes are Markdown, Rust documentation, private/test comments, and three supported
doctests. Executable behavior, public names/visibility, dependencies, serialized data, fixtures,
and test assertions are unchanged. The full seven-package area is ready for independent phase 05
review; no package or editorial coverage is deferred. Work was not delegated.

## Coverage and reader review

Reviewed all seven packages' introductions, public APIs, configuration readers, lifecycle code,
and consequential private/test comments. Traced blueprint requirements through runtime selection,
frozen context, effect claims, final authority/accounting, host preparation, and adapter entry;
checked daemon construction and shutdown consumers. Related ADRs explain the ownership decisions.
Intentionally retained clear identity/accessor, schema/codec, error, resource-validation,
publication/provenance, conformance, and failure-test explanations rather than annotating every
symbol. Workspace membership still contains 22 packages; this phase owns the seven assigned ones.

- **Capability:** added a guide separating task requirements, generation advertisements, durable
  selections, and live adapters. Clarified pure matching versus authority/capacity, portable
  references/documents, admission bounds, retry observations, and cancellation evidence. Added a
  short requirement-building doctest.
- **Capability host:** added a runtime-to-adapter sequence and implementer starting point. Explained
  immutable registration replay, exact-generation entry, durable reporting, input and artifact
  ports, caller-owned authorization, separate cancellation workers, health/catalog views, drain,
  shutdown deadlines, and storage lifetime. Corrected stale registry/worker orientation and
  clarified input-batch versus aggregate output accounting.
- **Local process:** added configuration and invocation guidance covering byte identity, direct
  arguments, isolated versus authorized host working directories, explicit environment/secrets,
  bounded capture/publication, cancellation, and restart meaning. Improved profile/adapter APIs
  and private identity/monitor/stream explanations. Preserved platform and trusted-process limits.
- **Model provider:** added endpoint setup and a mapping feature table; connected manifest
  verification, injected context, negotiation, bounded HTTP/SSE, completion markers, response
  artifacts, and uncertainty. Clarified whole-request timeout behavior, local-only health, fresh
  model sessions, and the distinction between mocked parsing and real-provider evidence.
- **Local secret:** added reference-to-source setup, per-call rotation, byte/file bounds, platform
  permission checks, redaction, and caller-owned authorization. The constructor doctest performs
  no secret read; the existing environment test's child-process isolation is explained.
- **Peer protocol:** added a two-host sequence, exact lost-reply recovery, request-bound validation,
  observation cursors/archival, cancellation, and metadata-first artifact transfer. Clarified exact
  v1.2 support and the unavailable incremental-catalog transport. Added an envelope/lookup doctest.
- **Peer HTTP:** added origin/serving setup and lifecycle guidance covering relationship scope,
  rotating credentials, catalog generations, closed startup admission, durable acceptance/entry,
  retries, polls/SSE, explicit artifacts, cancellation, shutdown, and retained tombstones.
  Corrected constructor readiness wording and documented the SSE archived-summary limitation.

Read the seven guides together against the implementation ordering. Inspected generated rustdoc
text and links for every crate introduction, all three examples, and the final host contract
refinements. Reviewed Markdown tables, fences, and sequence sketches in source. No browser visual
review or independent reader acceptance is claimed.

## Checks and evidence

Checks apply to this documentation-only working tree over `52cf340`. Owner suites ran after the
examples were added and before final prose refinements; their executable and example inputs stayed
unchanged. Detailed logs and rendered-text extracts are under ignored
`target/documentation-clarity/phase03/`.

| Check | Result |
| --- | --- |
| Grouped `cargo test` for all seven assigned packages with `--all-features --no-fail-fast` | 132 unit/integration tests and 3 doctests passed; the secret test also passed its nested child-process scenario. No ignored tests in these suites. |
| Grouped seven-package `cargo doc --all-features --no-deps`, `RUSTDOCFLAGS='-D warnings'` | Passed; host rustdoc refreshed after final comment refinements. |
| `cargo fmt --all -- --check` and `git diff --check` | Passed. All 40 changed Rust files preserve their non-comment, nonblank lines. |
| `cargo test -p milkdrift-evidence --test repository_contracts --all-features documentation::` | All 8 documentation contracts passed, including links, structure, and maintained example readers. |

The full executable gate was not required or run. These are local Windows results: Unix-only
process descendant cases were not exercised. Model and peer tests use local fixtures; no paid
provider call, real-provider interoperability, arbitrary external peer deployment, or new
filesystem/crash qualification is claimed. The reporting-cleanup finding below is source-derived,
not an executed failure-injection experiment.

## Findings and adjacent ownership

Rechecked and contributed to the existing
[context-policy issue](../whiteboard/issues/context-policy-enforcement.md). The model adapter
accepts only fresh model requests but does not compare blueprint session intent or redo runtime
selection policy. Host data loading verifies selected references under supplied read authority;
it is not a new actor/grant check. Package explanations preserve those limits.

Recorded [process reporting cleanup](../whiteboard/issues/process-reporting-cleanup.md): an initial
post-spawn report failure can bypass child termination/I/O joining, and monitor reporting errors
reach joins before termination. Updated local-process documentation and canonical product status
to avoid an unconditional cleanup claim. Executable repair and regression evidence remain separate
implementation work, not an unfinished editorial portion of this phase.

Supporting corrections in the peer wire reference now describe the server's actual feature flags
and distinguish catalog filtering from full invocation resource/capacity checks. Phase 04 should
preserve these corrections and both open findings during its broader shared-guide review.
