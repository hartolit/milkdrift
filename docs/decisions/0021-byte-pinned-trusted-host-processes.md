# ADR 0021: Byte-pinned identity for trusted host processes

- Status: accepted
- Date: 2026-08-29

## Context

A path and descriptor revision do not identify the executable bytes actually entered. The file at
an allowed path can be replaced between registration and invocation, while path mediation alone
does not isolate arbitrary behavior of code running as the daemon account. Treating those controls
as a sandbox would make capability selection, authority, health, and provenance disagree with the
actual security boundary.

## Decision

Local-process profile schema v2 requires an operator-declared BLAKE3 content digest and exact size
for a bounded regular executable file. Registration canonicalizes the configured path under an
execute root, streams and verifies the content, observes stable open-file/path and platform facts,
and derives one implementation identity from safe configured/canonical path digests, content,
size, optional package revision, and platform evidence. The generated descriptor also freezes the
complete profile digest, a separate execution-policy digest, optional documentation reference,
trust class, and honest platform process-ownership facts. A resolved snapshot retains those
bounded descriptor facts for durable attempt provenance.

Health and the boundary immediately before spawn re-resolve and rehash the same executable. An
identity failure makes that adapter generation unavailable without mutating its descriptor;
restoring bytes does not revive the invalidated instance. Changed tooling requires an explicitly
registered higher profile/descriptor revision. Successful entry records the exact pre-entry
identity digest in bounded attempt progress. Path-only schema v1 is refused rather than
implicitly migrated from whatever mutable bytes happen to exist at startup.

The adapter advertises exactly `TrustedHostProcess`. It executes with daemon-account privileges and
mediates direct argv, a rebuilt environment, selected materialization, declared import/export, and
bounded observations. It does not claim filesystem, network, namespace, container, or VM
isolation. Capability requirements and authority scopes can constrain execution trust exactly, so
`SandboxedProcess` requirements cannot resolve to this adapter.

## Rejected alternatives

- Path, inode, or modification time as identity, because each can remain or change independently
  of executable content and deployment intent.
- Hashing unbounded content or silently falling back to path-only identity, because both weaken a
  security boundary under hostile input.
- Automatically migrating schema v1 by hashing startup state, because that substitutes ambient
  mutable host state for an operator identity decision.
- Calling materialization and path mediation a sandbox, because the executable retains all host
  access available to the daemon account.
- Adding partial container, namespace, descriptor-exec, or unsafe platform machinery to this
  adapter, because a real sandbox is a distinct complete ownership and safety boundary.

## Consequences

Operators must generate schema-v2 profiles, pin digest and size, and advance both equal revisions
when executable or descriptor facts change. Documentation-only changes alter the full profile and
descriptor provenance but not implementation identity or execution-policy identity. Health and
pre-entry failures use bounded typed reason codes without exposing host paths. Existing process
materialization, output, secret, timeout, cancellation, and uncertainty behavior remains intact.

Portable safe Rust cannot atomically enter an already-verified open handle on every supported
platform. Rechecking the open file, path resolution, authorized root, metadata, bytes, and size
immediately adjacent to `spawn` minimizes but does not eliminate a final replacement race. Unix
group ownership also cannot prevent a malicious descendant escaping to another session/group;
non-Unix builds do not claim complete descendant cleanup.

## Reconsideration triggers

Add a separate `SandboxedProcess` adapter only when it can advertise and test a complete enforced
container, namespace, VM, or equivalent contract. Reconsider stronger executable entry when safe,
maintained, portable-enough APIs can execute a verified handle with a focused platform contract
and tests.
