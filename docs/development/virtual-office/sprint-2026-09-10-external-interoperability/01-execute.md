# Execute and qualify real external interoperability

You own the complete execution assignment in the [sprint](README.md): prepare the real resources,
resolve demonstrated integration failures, run the maintained process/model scenarios, and hand
off a reproducible qualifying report. Work through internal iterations without requiring a fresh
assignment for each package or setup step.

Read [AGENTS.md](../../../../AGENTS.md) in order, then the
[implementation](../../practices/implementation.md) and
[documentation](../../practices/documentation.md) practices and the
[workflow](../../workflow.md). Follow the sprint's scope, exclusions, and acceptance criteria.

## Sources and ownership

Trace the [external evidence guide](../../../guides/external-evidence.md) through the
[binary entry point](../../../../tools/evidence/src/bin/milkdrift-external-evidence/main.rs),
[profile preparation](../../../../tools/evidence/src/bin/milkdrift-external-evidence/profiles.rs),
[workflows](../../../../tools/evidence/src/bin/milkdrift-external-evidence/workflows.rs),
[scenarios](../../../../tools/evidence/src/bin/milkdrift-external-evidence/scenarios.rs),
[report validator](../../../../tools/evidence/src/bin/milkdrift-external-evidence/report.rs),
[integration tests](../../../../tools/evidence/tests/external_evidence.rs), and
[consumer schema](../../../reference/external-evidence-report-v1.schema.json).
Inspect the [templates](../../../../examples/external-evidence/README.md), affected manifests,
adapter consumers, and relevant tests before proposing changes.

Use the [process guide](../../../guides/local-process.md) and
[model guide](../../../guides/local-model-endpoint.md) for resource setup. The harness remains an
unpublished evidence tool; ordinary daemon commands, runtime transitions, and adapters retain
their production responsibilities. Coordinate shared edits and Cargo jobs with the coordinator.

## Execute the assignment

1. Inspect Git context and resolve the sprint's operator inputs. Revalidate the deployed agent's
   non-interactive interface, byte identity, prompt delivery, authentication under the cleared
   child environment, repository access, output declaration, and platform facts. Use explicit
   credential source references. Copy templates to private storage; do not assume the example
   agent version or previously used model endpoint is available or qualified now.
2. Validate the real model profile against its supported mapping and actual features. Preserve
   the guide's required system-context support, complete response/usage metadata, bounded
   output, and provenance. Choose explicit bounds that allow the evidence task to finish; a
   truncated or empty response is a failed qualification, not a reason to reduce the assertion.
3. Build the daemon before isolated harness tests and use the existing external-evidence tests
   to check the harness and its refusal paths. Fixture mode is useful local preparation and
   remains non-qualifying. Do not repeatedly run unchanged successful preparation checks or
   launch real attempts before their required inputs are ready.
4. Reproduce any integration failure and identify its owning boundary. Fix necessary defects
   completely across current producers, consumers, refusal paths, tests, and docs. Preserve
   strict profile/report validation, secret redaction, exact replay, generation identity,
   bounded cleanup, and truthful uncertainty. Do not add helper fallback, rewrite the report,
   relabel fixtures, enable controllers, or expand the product freeze to make the run pass.
5. Settle and verify changes, then coordinate a clean candidate that contains all required fixes.
   Build both harness and daemon from that checkout with an output location that cannot reuse
   another candidate's binaries. Keep scratch output and sprint handoff edits outside the
   qualifying source checkout. Record enough build provenance to connect the reported tree to
   the actual daemon; a caller-supplied `--daemon` path alone does not establish that connection.
6. Run the real command from the owning guide using the operator profiles and a fresh empty
   evidence directory. Complete both scenarios in one run: process verification/review,
   settled-boundary restart and prospective remediation, followed by model selected/omitted
   context, restart/release, response, usage, and artifact evidence. Use the existing scenario
   assertions; neither a prior model-only report nor a hermetic result replaces this run.
7. Check command success, report semantics, consumer-schema conformance, and redaction. Inspect
   the actual scenario evidence, including distinct process attempts, model completion without
   uncertainty, retained restart facts, and linked artifact digests. Keep the complete output
   directory private and retain the reviewed report at the agreed evidence destination.

If an attempt fails, preserve its non-qualifying result and diagnose before another external
invocation. A fresh output directory is required for another run. Do not erase uncertain work or
automatically retry an entered non-idempotent effect. Follow existing authorized reconciliation
and the guide's limitations. External unavailability needs an exact blocked handoff, not a new
provider or unbounded retry loop.

## Checks and deliverable

Select checks under the [verification policy](../../workflow.md#choose-verification-for-the-change).
Run the existing `milkdrift-evidence` external-evidence integration target and the binary's report
tests as relevant to the harness changes; use the owning adapter/runtime suites for a production
correction. Any executable, test, fixture, manifest, schema, or committed configuration change
requires the full local gate. Documentation-only updates require the documentation contracts and
any applicable example-reader checks. The real report remains an additional acceptance requirement.

Review the final diff and run `git diff --check`. Prepare updates to the existing external guide,
evidence guide, and status where the work changes their facts; coordinate final acceptance wording
and roadmap edits with the coordinator. Keep lasting explanations out of this temporary directory.

Hand back one current execution handoff with the reviewed scope, exact candidate commit/tree and
binary origin, safe report path and digest, checks actually run and their results, proposed doc
updates, and remaining acceptance gaps. Raw logs and private resource documents stay outside Git.

Stop when the complete candidate and qualifying report are ready for independent review. If a
required operator resource is still missing, finish useful local work and hand off its exact
missing reference, impact, and resume step; mark qualification blocked. Do not claim sprint
completion or start controller activation.
