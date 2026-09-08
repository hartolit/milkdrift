# Phase 04: Explain integrations, applications, and development tools

Work toward the reader outcome assigned for this phase in the [sprint README](README.md).
Read [AGENTS.md](../../../../AGENTS.md) in its required order, the
[documentation standard](../../engineering-rules.md#7-documentation), and the
[virtual-office procedure](../README.md). Use the reviewed phase 01 example and the assigned
reader outcome; coordinate related explanations without waiting for unrelated packages.

## Assignment

The package rows for this phase cover the three client/protocol crates, five adapters, two
applications, and `tools/evidence`. Work only on the assigned package portion. Inspect its
manifest/features, exports or binaries, production construction, normal consumers, and relevant
tests. Add a README and improve its crate entry and public/private/test explanations in scope.

Explain a supported use appropriate to the package:

- Protocols/client: what is encoded or requested, who checks access, how versions and cursors
  constrain a call, and which retries or disconnects are safe to handle automatically.
- Local process/secret: how configured inputs reach a process or secret consumer, which account
  permissions apply, what is validated, and what happens on cancellation or cleanup.
- Model provider: how a model request reaches each supported endpoint mapping, which options can
  be refused, how streaming becomes output, and what a lost response leaves unknown.
- Peer HTTP: what the originating and serving daemons each record, what an acknowledgement
  establishes, and what remains unknown across disconnect or restart.
- Redb store: how to open and use the store through its ports, what transactions protect, what
  recovery verifies, and where backup/retention/compatibility procedures are documented.
- Daemon/CLI: who runs the program, its normal setup and command path, and how to inspect results.
  Link to maintained operator recipes rather than copying configuration or credentials.
- Evidence: which developer question a harness answers, prerequisites, commands, outputs, and
  the limits of a successful report. Distinguish fixtures, local runs, and external qualification.

Preserve platform, trust, provider, and production-availability qualifications. Explain their
practical effect. Do not turn a development feature into a supported operator workflow. Trace
setup through current readers and binaries, and use placeholders instead of credential values.

## Verify and stop

Select checks from [workflow](../../workflow.md#choose-verification-for-the-change), including
relevant owner tests and existing command parsing checks for CLI examples. Inspect
rendered docs. Do not launch external providers, deployments, or paid evidence runs as an
editorial check. If an example needs unavailable prerequisites, state the unverified part.

Hand off exact coverage, supported example and failure explanation, validation results and limits,
unresolved findings, and remaining files. Stop after this portion; do not sweep all integrations
in one assignment or begin the maintained-prose phase automatically.
