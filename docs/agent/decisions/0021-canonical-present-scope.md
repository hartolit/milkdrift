# ADR-0021: Keep only present canonical package responsibilities

- **Status:** Accepted
- **Date:** 2026-08-16

## Context

The canonical workspace retained `corrective-workflow` as an incubating runtime
for a future workflow direction. No application, runtime, or library product
consumed it. Its only workspace-local dependency with no other production
consumer was `task-graph`. Both packages were private to this repository,
`publish = false`, and neither had an independently delivered present contract.

Workspace membership, substantial implementation, tests, and portability are not
present product responsibilities by themselves. Keeping the pair also preserved a
`runtime-capability` role for an unratified successor and made ordinary native and
portable gates certify inactive source.

The workspace lint policy also enabled all of `clippy::pedantic`. That broad group
made line counts, naming, argument passing, and boolean grouping part of acceptance
even when those diagnostics did not protect a Milkdrift invariant.

## Decision

Remove `corrective-workflow` and `task-graph` from the canonical tree. Remove their
manifests, source, tests, dependencies, exceptions, portability commands, current
documentation, and validation registrations. Git history remains the archive.
The workflow/workspace/authority successor stays unratified and must define its
contracts, roles, and first product consumer before source is introduced.

Remove `runtime-capability` from the current role vocabulary. Every workspace
member declares one nonempty present `responsibility` beside its role. Architecture
policy derives product reachability from Cargo metadata and requires every runtime
package to be reachable from an application through normal dependencies. Build
and development dependencies do not establish a product execution path.
This check is graph-derived; it does not add a package or edge registry.

Retain the current product boundaries that quarantine Candle, Hugging Face,
tokenizers, redb, Slint, host facilities, E0 ownership, and E1 coordination. Narrow
`domain-contracts` to one public crate-root path for its contracts by making its
implementation modules private.

Replace blanket `clippy::pedantic` with `clippy::all`, hard panic/unwrap/expect
denials, checked indexing pressure, exact lossy-cast lints, and exact large
ownership/error lints. Narrow `#[expect]` remains valid where an inline owner,
allocation contract, or numeric representation justifies the exception.

## Rejected alternatives

- **Wire corrective execution into E1:** reachability created solely to preserve
  inactive code would not establish a product responsibility.
- **Retain task graph as a foundation:** no current consumer or independent
  delivery boundary remained after removing the corrective runtime.
- **Exclude or park both packages as experiments:** the repository has no active
  owned experiment boundary; Git history is sufficient.
- **Keep the capability role empty:** that would silently pre-authorize an
  unratified runtime category.
- **Replace Cargo truth with exact member or edge lists:** typed Cargo metadata
  already owns membership, dependencies, features, and targets.

## Consequences

- The active workspace contains fourteen members: twelve current product-path
  packages, one benchmark observer, and one repository tool.
- Both portable plans derive four current domain packages instead of five.
- Adding an inactive runtime package or omitting a present responsibility fails
  architecture and verification planning.
- General workflow, workspace, authority, plugin, provider, and peer systems
  remain future direction rather than certified current code.

## Review trigger

Review when a ratified workflow/workspace/authority program has a concrete
application or headless host, or when a library has a real independent delivery
contract that justifies canonical maintenance without application reachability.
