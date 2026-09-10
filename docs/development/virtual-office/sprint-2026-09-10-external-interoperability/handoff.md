# Current coordinator and execution handoff

Owner: `Cedar-20260910-a` (agent pseudonym), 2026-09-10. Coordination and execution are assigned;
independent review is unassigned. Qualification is blocked on operator resources, and acceptance
is pending. No real coding-agent or model invocation has been launched by this assignment.

## Candidate and completed corrections

The clean candidate is `6c73ab37da7cba655151d258e491d3b938dbfefb`, tree
`dcc065c4d5763b58507929ba62bb889395e30fa0`, on local branch
`codex/external-interop-20260910`. Its worktree is
`C:/code/milkdrift/target/external-interop-20260910/candidate`.
The same owned changes remain in the main workspace; the pre-existing sprint documents and office
entry were preserved. No changes have been pushed or merged into main.

The candidate contains two necessary harness corrections:

- The report validator rejected the unchanged commit required by the scenario's no-commit prompt.
  It now requires unchanged commit/tree pairs and a nonempty diff with a valid digest. The scenario
  uses this validation before the model run, including in fixture mode. Existing v1 fields carry
  the evidence; the consumer schema now requires them for qualification. Regression tests reproduced
  rejection of a compliant repair and acceptance of changed history before the fix, then passed.
- Generated grants previously covered only the agent's executable directory. A separately installed
  Python interpreter used by the verifier, reviewer, and evidence producer consequently lacked
  execute authority. The profile-preparation owner now passes that canonical executable identity
  into grant construction. Both scenario grants include the exact execute scopes, without duplicate
  scopes or extra write access when the executable directories coincide.

The [external evidence guide](../../../guides/external-evidence.md) contains the resulting operator
explanation. Status and roadmap still correctly leave real interoperability unqualified and controller
activation separate. There is no new public library API, provider, workflow primitive, or durable
format. These internal harness/report-validation corrections need no new report version.

## Verification and evidence

All paths below are under ignored `C:/code/milkdrift/target/external-interop-20260910/` and stay local.

- `build-provenance.json`, `build.log`, and `external-tests.log` record the clean preparation baseline
  `0fb775ae4f7e6230f491d7a9b3a7b2198e096ebe`: daemon/harness build, four binary unit tests, and five
  external integration tests passed on Windows x86_64/MSVC with Rust 1.95.0 and Python 3.12.14.
  The `build/` binaries predate the corrections and must not be used for the revised candidate.
- `regression-before.log` and `regression-after.log` record the two report-validation regressions.
- `consumer-schema.log` records JSON Schema draft 2020-12 validation using `jsonschema` 4.25.1:
  schema validity, an accepted process-facts document, and eight missing/invalid-field refusals.
  No fixture report was relabeled as real evidence.
- `helper-authority.log` records the separate/shared-directory authority regression passing for both
  process and model grants.
- `gate-final/results.json`, `gate-final/source-identity.json`, and the named logs establish the full
  local Windows/MSVC gate: formatting, fixture prerequisites, all-target/all-feature checking,
  704 workspace tests and 24 doctests, Clippy, warning-denying rustdoc, deny/machete/duplicate-tree
  checks, test discovery, and all 24 repository contracts pass. The five manual longevity cases
  remain ignored. No new Unix or external-provider qualification is claimed.
- `gate-candidate-equivalence.json` compares all 676 tracked files with the clean candidate.
  Git-normalized executable contents match; raw differences in nine unchanged Rust files are
  checkout line endings. The intended source difference is the office entry, plus the untracked
  sprint coordination files in the main workspace. Final documentation edits require only the
  documentation contracts and diff check.
- `candidate-build-provenance.json` and `candidate-build.log` record the clean corrected-candidate
  build into a fresh `build-final/` directory, including the command, toolchain, timestamps, and
  SHA-256 identities of its daemon and harness. Use these binaries for this candidate, not `build/`.
- `fixture-final/report.json` is retained with its private session for review. Both maintained
  scenarios pass, with `dirty_at_start: false`, `fixture_mode: true`, and every qualification flag
  false. Its BLAKE3 digest is
  `b3_13afb69e21ec869568e71c6f27008d6ccf7a08c8dc973932368a17e869fbcd70`.
  `consumer-schema-final.log` confirms complete consumer-schema conformance. `fixture-inspection.json`
  verifies all 15 artifact references (1,784 bytes), five distinct terminal process invocations,
  unchanged repository commit/tree and the exact diff digest/size, both settled restart boundaries,
  two selected and two omitted model context items, generated-secret redaction, and unchanged binary
  hashes. These are executor checks of deterministic evidence, not independent review or real
  interoperability qualification.
- `final-documentation.log` and `final-checks.json` record all eight documentation contracts and
  `git diff --check` passing after the final handoff and office edits; the candidate remains clean.

The earlier `gate/` pass began before the helper-authority correction and is superseded by
`gate-final/` for the executable candidate.
There is no qualifying report path or digest yet, and no independent acceptance disposition.
The fixture session remains local at the proposed evidence destination; final retention of the
future reviewed real report still needs agreement. No sensitive session data belongs in Git.

## Remaining inputs and resume point

The operator must supply the real coding-agent profile path, supported model profile path and
capability identity, and any credential-source references (private file paths or environment-variable
names, never secret values). Repository evidence directories contained prior model-only profiles
and reports, including `target/closure-07/gemma-profile.json`, but no supplied real agent profile.
The prior Gemma resource has not been selected or contacted for this sprint.

Before a real invocation, revalidate the supplied profile's executable bytes, non-interactive flags,
prompt input, cleared-environment authentication, declared filesystem requirements, and Windows
platform facts. Check the selected model against the combined harness's fixed 64-output-unit task
and bounded wait loop (1,200 polls with 25 ms sleeps plus request time). The earlier Gemma smoke used
4,096 output units. This is an unresolved resource-fit question, not evidence that the current
combined model task succeeds or a demonstrated provider failure. Any required executable correction
must enter the candidate and receive the affected checks before qualification.

Resume the real command from the clean corrected candidate using its own freshly built daemon and
harness, the operator profiles, and a fresh empty evidence directory under the proposed destination.
Preserve any failed or uncertain run and diagnose before another invocation. Validate the complete
consumer report, inspect private scenario/artifact evidence and redaction, and record its digest.
The coordinator can then hand the concrete candidate and private evidence to a separately assigned
reviewer using [02-review.md](02-review.md). Only accepted real evidence permits canonical status/
roadmap qualification and sprint removal. The production controller lifecycle remains uninstalled.
