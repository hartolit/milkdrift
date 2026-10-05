# ADR 0050: Framed editor edge identities and retained connections

Status: accepted.

## Problem

The model editor hashed colon-separated kind and endpoint names. Colons are legal in those names,
so `draft/final_text → review/x:brief` and `draft/final_text → review:x/brief` had the same hash
input. Duplicate identity validation correctly refused the resulting graph. Replacing all edge
IDs when opening an old workflow would change immutable definitions, turn no-op saves into edits,
and needlessly consume the bounded mutation batch when making a small change.

## Decision

The daemon's single editor graph builder gives new connections an `author.` ID followed by the
lowercase BLAKE3 digest of a compact JSON string array. Its six elements are
`milkdrift.author.edge.v2`, the explicit lowercase `control` or `data` tag, source node, source
port, target node and target port, in that order. String boundaries preserve exact spelling and
field roles. The resulting 71-byte ID fits the existing edge identity limit. This removes the
ambiguous encoding; it is not a claim that hashes can never collide.

The reader accepts an edge only when its ID matches that encoding or the preceding supported
writer's exact `Control:source:port:target:input` / `Data:source:port:target:input` hash. That old
calculation is read-only recognition. It is never used to assign an ID to a new connection.
The reader retains validated identities by exact connection, rebuilds once, and requires equality
of the entire semantic definition. Arbitrary IDs, duplicate connections, changed acceptance or
context behavior, and other richer content still refuse convenience editing.

An unchanged connection keeps its retained identity through an edit. New or retargeted connections
use the current encoder, including repair-generated edges after terminal retargeting. Consequently,
a supported child may contain both known ID forms. No-op opens and saves retain the original
revision and digest. Actual edits create children; copies preserve source graph facts under their
existing semantics. Definitions, run pins, accepted proposals and exact receipts are never migrated.
Blueprint schema 3, storage formats and command shapes stay unchanged. This is narrowly scoped
support for currently accepted editor definitions, not a general historical-format policy.

Invocation input names also accept colons so legal blueprint ports reach capability entry unchanged.
The existing byte bound and all other character checks remain. This additive reader correction
changes no canonical bytes for existing requests; older binaries cannot execute the newly exercised
colon-port request. Input names remain data, not paths or commands.

Changing the delimiter or banning colons would leave ambiguity or withdraw legal names. Ignoring
IDs during recognition could admit content the editor cannot preserve. Keeping a second graph
compiler would duplicate the supported shape. Reconsider this narrow reader rule only with an
explicit change to supported saved definitions and their reopen/replay evidence.

## Evidence

The [frozen old-writer fixture](../../apps/daemon/tests/fixtures/authoring/README.md) predates the
encoder change. Public daemon tests cover the exact two-connection example through model HTTP,
old-document identity, no-op save, child edit/reopen, copy, stopped-store reopen and exact saved
request replay without extra calls. Encoding cases pin independent JSON arrays, endpoint roles,
repetition and name limits. Existing richer-shape, stale-base and authority refusals remain in the
same authoring suite. Current executed acceptance belongs to [status](../product/status.md).
