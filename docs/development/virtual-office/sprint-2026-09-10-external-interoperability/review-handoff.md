# Independent external interoperability review

Disposition: **accepted**, 2026-09-11. This separate review session did not implement the
candidate or produce the real report. No acceptance defect remains in the assigned boundary.
Acceptance covers the candidate and retained report below; it does not close the coordinator's
documentation/retention work or authorize production controller activation.

## Examined identities

- Candidate commit: `8c6cdb9137bc8f688fd6cc800d3adf416d4f4625`.
- Candidate tree: `7ae25644b92906b4b88ad02e1d73f646174f5648`.
- Report: `C:/code/milkdrift/target/external-interop-20260910/real-bonsai-03/report.json`.
- Report BLAKE3: `b3_182a6602a7276cee5e6daac4ae842fcc3612d0dd1891315392e08e89bd11a1bf`.
- Platform: Windows x86_64/MSVC, Rust 1.95.0. The clean candidate build used a fresh
  `build-bonsai-report/` directory. Recomputed daemon SHA-256 is
  `105a2e6c2ad90687fae8bfb10ae642d1d6d8c9632565bc8638a3f4ba3aa4f97f`; harness SHA-256 is
  `ce64393e23259fc1e669ef992debc84fa0ed4c9575b0f7cf16ddd4ffe8a015d9`.

The candidate remains clean. At review, main is `e53d0c4`; it matches every candidate file outside
the virtual office and status. The execution handoff's description of uncommitted implementation
is superseded by this observed Git state. The remaining differences are documentation, not
untested executable changes.

Unless specified otherwise, evidence references below are relative to ignored
`C:/code/milkdrift/target/external-interop-20260910/`. Raw evidence remains private.

## Evidence and implementation assessment

The review traced the [execution sources](01-execute.md#sources-and-ownership), current consumers,
tests, manifests, [external guide](../../../guides/external-evidence.md), consumer schema, and
[ADR 0027](../../../decisions/0027-controller-final-entry-reservations.md). The examined changes
span `0fb775a` to the candidate: uncommitted-repair validation, separate helper execute scopes,
explicit wait/output bounds, normal model completion, and report finalization/redaction.
They remain in the unpublished harness and its consumers. Each has applicable regression
coverage; all workflow waits and model-task callers adopt the new bounds, and one final writer
owns report publication. No production adapter, public library API, or durable format changed.

`bonsai-report-build-provenance.json`, its build script/log, and
`bonsai-run-03-provenance.json` connect the clean source, actual binaries, selected resources,
and successful real command. The private resource hashes and current executable bytes match.
The source/rendered profiles and retained agent stderr establish byte-pinned `codex-cli 0.153.4`,
Fresh stdin-driven repository work, and the declared local Bonsai agent model. The separate model
profile selects `prism-ml/bonsai-27b` through LM Studio's supported chat-completions mapping,
with streaming and system-role support. External authentication is `no_auth`; generated daemon
tokens remain private. Profile/policy digests and the daemon's production-normalized configuration
digest match the report. No provider was contacted during this review.

The reviewer used the candidate's compiled libraries to inspect an isolated database copy,
without starting a runtime or changing the retained source database. All 389 journal events pass
the owning checksum-validating reader. All 29 stored artifact references match their retained
bytes and lengths, including all 14 references in the report. The exact report passes its Rust
writer and draft-2020-12 consumer schema; Rust re-encoding is byte-identical. The production
readers also accept all ten frozen manifests, three revisions, the process/model profiles, and
the model response. These observations are recorded in `review/verification.json` and
`review/contract-validation.log`; inspection sources and private extracted records remain beside
them under `review/`.

The process evidence establishes both declared fresh coding invocations, unchanged initial/final
commit and tree, and the exact nonempty 180-byte normalized Git diff. The only tracked repair is
the calculator implementation. Retained agent logs contain the declared prompts and repository
verification activity. Both separate verifier results pass the unittest and carry the same diff;
the weak verifier intentionally withholds `verification_pass`. The reviewer helper receives its
manifest. This is the documented orchestration fault injection, not failed agent code.

The process journal reaches the approval wait at sequence 99; the next accepted fact is pause at
100, followed by prospective reconciliation, signal, resume, and the new tasks. There are six
process-task invocations in total: the report lists its five required roles, while the journal
also retains the final remediation reviewer. That sixth task is declared by the remediation
revision. Every task enters once; no prior agent invocation is repeated. The eleven retained
application receipts match all reported commands, and the stored proposal connects the revisions.

The model journal reaches its unreleased wait at sequence 53, then accepts the release signal at
54. Exactly one model attempt enters afterward. Its frozen policy-version-2 manifest selects the
two intended evidence producers and omits the intentionally unselected artifact; the other
omission is the direct task input, which remains present in the invocation. Fresh session,
4,096 output units, streaming, exact generation/profile/protocol/model/origin, final response,
provider identity, and input/output usage agree across the invocation, snapshot, artifacts, and
terminal events. Nine durable fragments reconstruct the complete required final text, with
normal `stop` completion and no uncertainty. The report's two equal before/after restart
sequences agree with these settled journal boundaries and the successful harness assertions.

Redaction inspection checked the generated credential values, forbidden fields, report content,
and referenced private artifacts. The report contains no raw prompts, complete outputs, or
repository file contents. This is independently inspected local evidence, not cryptographic
attestation of the executor or provider.

## Checks run and reused

Reused the complete successful `gate-bonsai-report-final/` gate: formatting, prerequisite binary
builds, all-target/all-feature checking, 709 tests plus 24 doctests, Clippy, warning-denying
rustdoc, dependency audits, discovery, and all 24 repository contracts. Five manual longevity
cases remain ignored. The retained patch hash, changed-file SHA-256 values, base commit, and
candidate comparison establish source applicability. Logs include all twelve harness binary
tests and five external integration tests, including refusal of truncated valid-looking output.
The gate script establishes the warning-denying rustdoc environment. Earlier failed gates and
non-qualifying real attempts were not substituted for this evidence.

Ran independent source/binary/resource hashes, production contract readers, schema validation,
artifact integrity, journal/receipt/provenance/context/response cross-checks, and report redaction
inspection. The review's only repository-source addition is this handoff. Documentation accuracy,
local links, structure, and Markdown were reviewed. Ran
`cargo test -p milkdrift-evidence --test repository_contracts --all-features documentation::`
(eight passed) and `git diff --check`; logs are `review/documentation.log` and
`review/diff-check.log`. The new handoff also passes the explicit untracked-file whitespace check.
No unchanged full gate or real scenario was repeated.

## Return to the coordinator

Accept the report as qualification of this local Windows Codex/Bonsai process/model boundary.
The existing guide explains the tested controls and refusal behavior; final accepted wording in
[status](../../../product/status.md), [verification evidence](../../verification-evidence.md),
and [roadmap](../../../product/roadmap.md) remains the coordinator's responsibility, as does
retaining the complete private evidence and closing the office sprint.

Keep the qualification limited to the exercised resources and controlled abrupt restarts at
settled boundaries. It establishes no model-quality, new-platform, graceful-signal, filesystem
power-loss, peer, or production-controller claim. No additional real run is required for the
unchanged accepted candidate. Cargo ownership returns to the coordinator with this disposition.
