# ADR 0050: Complete authorized capability snapshots

Status: accepted.

## Decision

Control protocol 2.20 emits `capability_snapshot` with the complete authorized catalogue. A client
replaces its previous set on every snapshot, including an empty one. Capability identity and
descriptor generation distinguish entries; `current` and `draining` describe entries still present.
This fixes the earlier addition-only stream, which could leave removed capabilities visible forever.

The daemon retains bounded snapshots under the complete cursor authority binding. It rechecks
authentication and authorization on every poll and closes when an open subscription's binding
changes. Health and capability cursors also bind a random HTTP feed incarnation, so a number reused
after restart cannot refer to an old observation. Failed process-local continuation checks emit
`resync_required` before any catalogue or health disclosure. Durable run/timeline cursors keep
their existing restart meaning. The incarnation belongs to the HTTP state that owns retention;
separate HTTP servers over one host have separate continuation domains as well.

## Consequences and compatibility

Fresh subscriptions receive the latest complete set. Reconnects replay snapshots within the retained
item/byte window; expired windows or snapshots outside the document bounds require resynchronization.
Polling observes current state and does not promise to retain every intermediate lifecycle change.
The [control contract](../reference/control-api.md#cursors-and-sse) owns exact bounds and client use.

The closed observation vocabulary replaces `capability` with `capability_snapshot`; older protocol
minors and the old variant are refused. Milkdrift is unreleased and upgrades daemon and clients
together. No compatibility reader or client-owned capability registry is introduced. Cursor schema 2
is unchanged because its existing opaque scope digest can bind the daemon incarnation. Durable
workflow history, command receipts and storage formats are unchanged.

Production registry lifecycle tests exercise drain, removal, replacement, empty catalogues and
fresh/reconnected convergence through HTTP and control-client. Separate regressions cover authority
isolation, rotation, bounded retention and restart. These establish the headless contract, not browser
authentication, Svelte behavior or a lossless capability audit.
