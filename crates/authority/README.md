# milkdrift-authority

This crate evaluates explicit grants for workflow commands and independent capability execution.
An immutable grant binds an actor, allowed operations, resource selectors, numeric ceilings, a
validity interval and a revocation generation. Humans, services and model-proposed work face the
same evaluator. A grant's title or preset never creates extra permission.

`AuthorityGrantBuilder` constructs the current schema-5 grant. `AuthorityGrant::from_json` accepts
only that bounded contract and rejects duplicate fields, unsupported versions and unknown selectors.
`GrantSetEvaluator` checks an exact request against the current grant set and revocation information.
Schema-3 `AuthorityDecisionSnapshot` retains the request and its decision with a verified digest.
Old grant and decision schemas are refused; changing a document's schema number is not a migration.

Workflow scope applies to admission, edits, cancellation, discovery and reads. Capability scope also
checks the exact operation, selected generation's category, side effects, locality, peer, trust zones
and execution trust class. Adapter requirements contribute filesystem, network and secret facts.
Artifact metadata and content require their own identity and sensitivity scope. Daemon diagnostics,
peer administration and independent workspace access retain separate scopes.
`RenameDaemon` additionally requires the daemon configuration resource fact and scope. It is
independent of workflow editing and does not require a workflow identity. The added operation does
not change existing schema-5 grant bytes or their digests; granting it requires a new grant revision.

At external entry, runtime re-evaluates the accepted actor/grant lineage against current authority.
Revocation or expiry can stop entry even after local preparation. An accepted command receipt retains
a past result; reads and replay responses still require current permission to disclose it.

Numeric grant ceilings authorize one request. The task runtime separately owns cumulative workflow
consumption and reserves enforceable resource maxima before entry. A grant with an absent allowance
does not imply unlimited external work. Capability-host and managed-resource owners retain their
own operational admission, physical identity and stop-evidence protections.

The [contract tests](tests/contracts.rs) cover canonical documents, hostile bounds, revocation,
expiry, type separation and each retained refusal family. [Selector tests](tests/selectors.rs) cover
explicit allowlists and wildcards, and workflow scope tests preserve filtered discovery/read behavior.
