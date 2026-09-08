# milkdrift-authority

This package answers whether an actor may perform an operation with the supplied resources.
It gives commands, capability invocations, and protected reads the same decision rules. Use it
when adding an authorization boundary or changing how grants constrain work.

Consider starting a run. The daemon authenticates the caller and supplies an exact grant claim.
The [runtime](../runtime/README.md) constructs an `AuthorityRequest` containing the actor, grant
revision and digest, operation, workflow/run, resource facts, and evaluation time.
[`GrantSetEvaluator`](src/evaluator.rs) compares those facts with its configured immutable grants
and revocation generations. A valid denied request returns a decision with reason codes; malformed
input returns an error. Runtime saves the decision with the accepted or rejected command result.

An allowed start also establishes an [`ExecutionAuthorityBasis`](src/model/execution.rs). Later
attempts inherit this exact grant reference. Capability selection and final adapter entry make
fresh decisions using the selected generation's resource requirements. Replacing a provider or
revoking authority therefore cannot be bypassed by an earlier successful start decision.
The [host](../capability-host/src/lib.rs) supplies candidate facts; authority evaluates them.

## Choose the scope deliberately

Build an [`AuthorityGrant`](src/model/grant.rs) with operations, resource scope, validity, and
budgets, or decode its strict document with `AuthorityGrant::from_json`. Operation permission and
resource permission must both fit. Artifact metadata and content have separate operations, and
artifact checks use stored sensitivity as well as identity. A capability's descriptive labels
and a caller's role name are not grants.

[`Selection`](src/selection.rs) distinguishes an explicit wildcard from a nonempty exact set.
Capability allow scopes combine their selectors: narrowing a profile does not waive the operation
or side-effect ceiling. Revision admission checks the whole requested capability envelope;
an unspecified dimension is `Any`. In particular, blueprint requirements currently cannot narrow
locality or peer, so a grant narrowed in either dimension cannot admit that envelope. See the
[authority guide](../../docs/operations/authority.md) for supported configuration.

`AuthorityBudget` compares one request with its grant ceilings. It does not accumulate spending;
durable cumulative reservations belong to [persistence accounts](../persistence/src/controller_account.rs).
`FilesystemScope` compares canonical path components. The host still has to resolve and inspect
the real path before presenting that fact. `SecretRef` identifies a value for a separate resolver;
`SensitiveSecret` controls ordinary formatting and borrowed access to resolved bytes.

The evaluator reads no clock, authenticates no token, and inspects no filesystem or network.
Boundary callers must supply trustworthy facts and enforce the returned decision before acting.
The [evaluator trait](src/evaluator.rs) documents this division for implementers.

Run `cargo test -p milkdrift-authority --all-features` from the repository root for selector,
grant, denial, path, and redaction contracts. No feature selection or external service is needed.
