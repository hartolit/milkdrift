# 0047 — Reuse definitions through existing owners

Status: accepted.

## Decision

Control protocol 2.15 exposes saved interface fields and independent copy construction through the
same authorized daemon API used by the CLI. A copy is a genesis revision with another explicit
workflow identity. Its immutable reason records the exact source; the ordinary command receipt
binds the complete request. Nodes, edges, interfaces and metadata remain definition facts. Runs,
inputs, artifacts, grants, reservations and editing claims are not copied.

Governing agreements bind the original workflow identity and cannot simply be attached to an
independent copy. This convenience refuses them; it neither removes protection nor silently seals
a replacement agreement. Reuse an exact governed definition or explicitly author another governed
method through the existing owners.

## Consequences

Saving and copying do not publish a service or repin accepted execution. The coordinated wire
change requires matching daemon and client builds; durable blueprint and receipt formats remain
unchanged. Clients display the server's interface projection and use returned revision identities.
No template store or mutable preferred-version registry is introduced.

Publication preparation is a transient control operation over `PublicationDraft`. The existing
publication owner resolves the stored agreement and operator-configured service identity, then
runs ordinary publication validation without registering a capability. Descriptor contracts,
service grant selection and every allowance remain reviewed inputs. The daemon checks revision
read and publication authority before exposing the prepared document. Publishing rechecks current
facts through the existing separate command; a saved preparation is not a grant or an acceptance.
The CLI's text-input convenience uploads through the same bounded input endpoint and freezes the
returned reference. It does not alter published input contracts or artifact permission scope.
