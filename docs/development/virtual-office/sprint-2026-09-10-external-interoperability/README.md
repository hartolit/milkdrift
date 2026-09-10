# Sprint: qualify real external interoperability

Prepared on 2026-09-10 against `0fb775ae4f7e6230f491d7a9b3a7b2198e096ebe`.
This sprint takes the first unfinished outcome in the [roadmap](../../../product/roadmap.md):
establish that a real coding agent and a supported real model can complete the maintained
interoperability scenarios through the production daemon. The
[whiteboard](../whiteboard/README.md) had no open topics at preparation.

## Outcome and scope

Produce a strict, redacted, qualifying combined report from an exact clean source candidate,
have its evidence and any integration corrections independently reviewed, and put the accepted
qualification and its limits in the existing documentation owners. A successful fixture run,
a model-only result, or completed preparation does not meet this outcome.

The [external evidence guide](../../../guides/external-evidence.md) owns prerequisites, scenario
meaning, commands, and report handling. [Status](../../../product/status.md) owns current
implementation and evidence. This directory holds assignments and temporary handoffs only.

Work includes configuring operator-supplied resources, exercising the existing harness, and
correcting demonstrated failures within the existing process/model interoperability boundary.
Finish a necessary correction across its implementation, consumers, refusal paths, tests, and
explanations; the likely files below are coordination aids, not artificial scope limits.

Production controller activation is excluded. A qualifying report supplies one prerequisite for
the roadmap's subsequent activation task; that task must assess all of its own current evidence
requirements under [ADR 0027](../../../decisions/0027-controller-final-entry-reservations.md).
The product freeze also excludes UI, new provider families, new workflow primitives, broad
architectural cleanup, remote-peer qualification, and storage migration. This sprint does not
qualify model quality, filesystem power loss, or additional platform guarantees.

## Assignments and order

Execution started on 2026-09-10. `Cedar-20260910-a` is the coordinator's agent pseudonym in the
task assigned to execute `00-coordinate.md`. Execution has resumed in the task assigned
`01-execute.md`, which prepared the supplied local Bonsai resources, corrected and verified the
harness, and produced an inspected qualifying combined report from a clean candidate. The final
reviewer remains unassigned and must be a separate session that did not implement the candidate
or produce its report. Acceptance is pending.

| Assignment | Current owner | Responsibility and expected result | Dependency |
| --- | --- | --- | --- |
| [00: Coordinate and close](00-coordinate.md) | Cedar-20260910-a; execution handoff ready | Own shared edits, candidate identity, accepted coverage, evidence disposition, and office cleanup. | Closure waits for accepted review and evidence retention. |
| [01: Execute and qualify](01-execute.md) | Current 01 task; execution complete, ready for review | Complete setup, necessary integration fixes, verification, and the qualifying real run; hand off reproducible evidence. | Candidate `8c6cdb9` passed the full local gate; `real-bonsai-03/report.json` qualifies and passed executor inspection. |
| [02: Independently review](02-review.md) | Unassigned; acceptance pending | Review the candidate and report, then give a supported acceptance or specific correction request. | Separate assignment, execution handoff, and access to the cited evidence. |

The numbers describe coordination and dependencies, not three required agent launches. Preparation
does not start execution, contact a provider, or create tasks. Use delegation only when explicitly
assigned. Keep workspace Cargo jobs with one owner, including during review.

## Inputs to resolve at execution

The operator supplied two Bonsai instances at local LM Studio. The executor prepared private
model and byte-pinned Codex profiles; the endpoint needs no authentication. Exact references,
preflight results, sandbox requirements, and the current run live in the [handoff](handoff.md).
Resource preparation must establish:

- The real non-interactive coding-agent profile path, exact executable identity and version
  arguments, authentication source references, and bounded execution settings. The agent must
  consume the declared prompt, edit the disposable repository without committing, and publish
  the required `diff` output. Revalidate the actual release's interface and host needs.
- The supported model profile path and capability identity, reachable endpoint/model, truthful
  feature declarations, and credential source references if used. Retain the guide's required
  context, response, usage, and provenance checks.
- The agreed source candidate and private evidence destination. Keep credential values out of
  prompts and handoffs; request paths or environment-variable names instead.

Missing resources leave qualification blocked. Complete useful local preparation and necessary
in-scope corrections, then record the exact missing input and where execution resumes. Do not
substitute fixtures or weaken checks to turn that state into completion.

The clean candidate and complete private evidence remain under ignored
`target/external-interop-20260910/`; the coordinator owns final evidence retention and acceptance.
The whiteboard remains empty. The [current coordinator/execution handoff](handoff.md) records
the exact candidate, verification, report digest, and independent review requirements.

## Work areas and edit ownership

| Area | Likely files and canonical owners |
| --- | --- |
| Harness and report | [Evidence package](../../../../tools/evidence/README.md), its external-evidence binary modules, [integration tests](../../../../tools/evidence/tests/external_evidence.rs), and [consumer schema](../../../reference/external-evidence-report-v1.schema.json). |
| Resource setup | [External templates](../../../../examples/external-evidence/README.md), [process guide](../../../guides/local-process.md), and [model guide](../../../guides/local-model-endpoint.md); rendered profiles stay private. |
| Demonstrated integration corrections | Existing process/model adapters and their consumers, as established by source tracing. Coordinate any change to runtime, host, daemon, or shared manifests before editing. |
| Lasting evidence and instructions | [External guide](../../../guides/external-evidence.md), [verification evidence](../../verification-evidence.md), [status](../../../product/status.md), and [roadmap](../../../product/roadmap.md). The coordinator owns final integration of shared docs. |
| Temporary coordination | This README and one current handoff per started assignment. Raw logs, reports, and generated inventories belong under ignored `target/` or private external storage. |

Each assignment directly selects the applicable
[implementation](../../practices/implementation.md) and
[documentation](../../practices/documentation.md) practices in its prompt.

## Acceptance and stop conditions

1. The harness and daemon were built from the exact clean candidate cited in the report.
   Private setup and later sprint notes do not obscure the tested commit/tree or binary origin.
2. The real command succeeds and the strict report has `fixture_mode: false`,
   `dirty_at_start: false`, and qualifying process, model, and top-level results. Its scenario
   facts, artifacts, restart observations, provenance, and redaction satisfy the owning guide
   and validator; the consumer schema also accepts it.
3. Every required check for the actual changes passes under the
   [verification policy](../../workflow.md#choose-verification-for-the-change). Executable
   corrections require the full gate and relevant focused evidence. Unchanged successful checks
   can be reused with exact source identity and an explanation of their applicability.
4. Independent review accepts the candidate and report, with no unresolved acceptance defect.
   A report is evidence to inspect, not independent attestation merely because its flags are true.
5. Canonical docs describe only the accepted evidence and its limits. Controller activation
   remains separate. Close and remove this directory only through the
   [office procedure](../README.md#close-and-remove-a-sprint), after retaining the evidence at
   its agreed destination and resolving temporary inbound links.

If an external input or demonstrated out-of-scope problem prevents acceptance, keep one concise
current handoff with its impact, owner to act, and resume point. Use the whiteboard only when the
[findings policy](../../workflow.md#findings-beyond-the-assignment) applies. Neither a blocked
handoff nor an accepted deferral establishes interoperability qualification.
