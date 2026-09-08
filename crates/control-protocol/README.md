# Control protocol

This package defines the requests and replies exchanged by an operator client and the daemon.
Use it when building a client or adapting a daemon route: `CommandRequest` describes an operation,
read types describe what the caller may inspect, and `Page` and `ObservationEnvelope` let a caller
continue through history without loading a whole run. The
[control client](../control-client/README.md) supplies HTTP transport.

## Submit work and inspect its result

A client chooses a command ID and keeps the complete request until its outcome is known. The
daemon supplies the actor and grant from authentication, checks authority and optimistic guards,
and retains the command result. If the reply is lost, sending the same request under the same
authority recovers that result. Changing the reason, evidence, guards, or command body under the
same ID conflicts. A successful start response means the command was accepted; inspect the run
and its attempts to learn whether the work succeeded.

```text
CommandRequest → daemon checks and durable acceptance → CommandAccepted
                                    |
                                    v
                          run, attempt, timeline reads
```

The command family covers blueprint and sequence validation/import, run control and signals,
retained-work resolution, proposals, controller checkpoints, and layout updates. Controller
commands exist in the contract, but [production activation](../../docs/product/status.md) remains
gated. Peer administration has separate routes described in the
[wire reference](../../docs/reference/control-api.md).

## Read and continue

`RunRead` shows the current execution frontier. `AttemptRead` supplies exact current or historical
attempt evidence, including selected capability, authority, context, usage, and output references.
`TimelineEntry` projects journal facts into a stable public vocabulary. None requires a client to
decode internal runtime events or database records.

Pages return an opaque continuation for the same query; streams return one with each observation.
Keep it unchanged. A new filter, grant, or credential may require a fresh read and subscription.
`ResyncRequired` asks the consumer to refresh its view; a transport reconnect alone cannot restore
history that the feed no longer retains. Server code must use the authenticated cursor methods;
the position-only decoders are for inspection and do not establish authority.

`LayoutDocument` stores positions and annotations for an exact revision independently of workflow
meaning. Its digest and update generation protect presentation writes, not run history.

Start at the [crate API](src/lib.rs) for codecs, pages, and cursor verification;
[command types](src/command.rs), [read types](src/read.rs), and [layout](src/layout.rs) own their
specific fields. The [control API reference](../../docs/reference/control-api.md) owns routes,
wire versions, error codes, and CLI output. [Daemon operations](../../docs/operations/daemon.md)
owns setup and recovery procedures.
