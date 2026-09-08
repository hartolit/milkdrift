# Phase 01: Define work and its inputs

Execute the whole phase described in the [sprint plan](README.md), using the
[documentation standard](../../engineering-rules.md#7-documentation) and the repository's required
reading order. The plan owns scope exclusions, coordination, verification, and handoff rules.
This phase replaces the former context pilot and package-fragment assignments.

## Reader outcome and ownership

A contributor should understand how to describe work before it runs, choose the inputs a task
requests, and interpret the documents exchanged with execution. Own all documentation in
`crates/contracts`, `crates/blueprint`, `crates/workspace`, `crates/model`, and
`crates/prompt-sequence`: README, crate/module orientation, public APIs, and meaningful private/test
comments. Reassess the existing contracts, blueprint, and model edits rather than treating them
as the approved writing style.

Build the explanation around a workflow someone wants to run. Follow a prompt sequence into a
blueprint revision, task requirements and context policy, then the workspace values and model
documents needed at execution. Read runtime selection and adapter consumers to explain the handoff
accurately. The revision describes intended work; the runtime's records establish what happened.
Show this relationship where it helps the reader, without turning every type into a workflow guide.

Within that account, explain how ordinary and structured definitions relate, why a change creates
a revision, how branch-owned values and artifact references differ from their bytes, and why a
policy request differs from a manifest of actual selections. Put constructor choices and meaningful
failures near the API where the caller makes those choices. A compact flow sketch may communicate
the definition-to-execution distinction better than repeated lists of properties.

For contracts, start with the problem shared document checks solve for their callers. Show a real
consumer when it makes their composition clear. Keep domain-specific meaning with the owning
document. Revisit the existing numbered validation walkthroughs and macro/helper comments:
retain useful distinctions, but move incidental implementation detail out of the introduction.

## Complete the area

Trace source, consumers, tests, and the relevant ADRs before making claims. Recheck the
[context-policy finding](../whiteboard/issues/context-policy-enforcement.md); do not describe a
declared session or required-evidence rule as enforced without evidence. An executable fix remains
outside this assignment.

Use the workflow-to-task and policy-to-manifest paths as starting points, then review the remaining
modules and public surface across all five packages. Include a supported construction or consumer
example where it teaches a consequential choice; avoid repeating setup for every constructor.
Retain clear material and remove redundant private narration. Public items still need the required
rustdoc, scaled to what they add beyond the owning explanation.

Check the affected docs under the verification policy, then read the result without method bodies:
can a contributor choose the right entry point, explain the relationships, and use the documented
path? Hand off once the whole area is ready for review, with any real gaps identified. Finishing
the policy example or one helper package is an internal milestone, not the end of this phase.
