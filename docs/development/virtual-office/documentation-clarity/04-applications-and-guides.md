# Phase 04: Use the applications and maintained guides

Execute the whole phase described in the [sprint plan](README.md), using the
[documentation standard](../../engineering-rules.md#7-documentation) and the repository's required
reading order. The plan owns scope exclusions, coordination, verification, and handoff rules.

## Reader outcome and ownership

An operator should know how to set up and use the supported system, inspect what happened, and
respond to interruption. A contributor should be able to find the product intent, architecture,
development guidance, and detailed contracts without reconstructing the system from source.

Own all documentation in `crates/control-protocol`, `crates/control-client`, `apps/daemon`,
`apps/cli`, and `tools/evidence`, including introductions, API docs, and meaningful private/test
comments. Also own the shared prose review: root entry documents, `docs/README.md`, product and
architecture documents, development, guides, operations, reference, decisions, and maintained
prose/comments in `examples/` and `.github/`. Preserve active sprint state and the evidence and
contributions in whiteboard topics; they are not material for a stylistic rewrite.

Follow an ordinary operator use from configuration and authentication through a CLI/client request,
daemon command handling, and result inspection. Connect the protocol's commands, reads, cursors,
and streams to what clients can do. Explain daemon startup/recovery, admission, maintenance, and
shutdown in terms of what the operator observes and must arrange. Keep detailed recipes in their
maintained guides and link package introductions to them.

Use an example or simple flow where it reduces effort, such as showing where a command is checked
and where its result is retained. Keep concrete CLI commands and configuration names exact.
Explain disconnect/reconnect and retry behavior where it affects operator or client decisions.

## Complete the area

Cover the packages' remaining modules and supported command families as well as the starting
recipe. Explain what each evidence tool establishes, how to use it, and where its evidence ends.
Keep fixture demonstrations, local operation, and external qualification distinct.

Review the maintained documents as connected reading paths, not isolated word replacements.
Give each document a clear purpose and trim repeated explanations or vocabulary lists that do
not help it serve that purpose. Preserve canonical ownership: vision describes intent,
architecture describes responsibilities, status reports current facts and limitations, roadmap
orders unfinished work, and ADRs preserve decisions and their tradeoffs. Clarify historical
explanations without changing what was decided or implying that planned behavior already works.

Read the other phases' owners as evidence and coordinate edits to shared guides. This phase can
start before their drafts are ready, but its final consistency review must include their results.
Keep precise reference information with its owner, move incidental detail out of introductions,
and update inbound links when headings change.

Use the verification policy for documentation contracts, doctests/rustdoc, and existing CLI or
production-reader example checks. Inspect formatting and diagrams. Do not change fixture/schema
data, workflow behavior, or test assertions to make editorial changes pass.

Hand off the complete package and maintained-prose coverage, supporting checks, and real gaps.
Finishing one operator recipe or one document family is a milestone within this assignment.
