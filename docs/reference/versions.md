# Current contract versions

Owning constants, strict readers and golden fixtures define these versions. Unsupported versions
are refused; the task cutover does not migrate earlier stores or revive graph readers. Task
definitions are covered by the control protocol; workflow snapshots also carry the runtime document
version and redb format below.

| Contract or durable family | Version | Read behavior |
| --- | --- | --- |
| Capability descriptor / resolved snapshot | 1 / 3 | Exact owning reader; incompatible input refused. |
| Invocation request | 3 | Exact owning reader; incompatible input refused. |
| Model document / endpoint profile | 1 / 2 | Exact owning reader; incompatible input refused. |
| Authority grant / authorization decision | 5 / 3 | Exact owning reader; incompatible input refused. |
| Administrative integrity cursor | 5 | Exact owning reader; incompatible input refused. |
| Peer hot record / compact tombstone | 5 / 3 | Exact owning reader; incompatible input refused. |
| Redb internal document format / physical schema | 22 / 19 | Exact owning reader; incompatible input refused. |
| Runtime task workflow document | 3 | Exact owning reader; earlier task documents refused before mutation. |
| Local-process profile / host materialization | 2 / 1 | Exact owning reader; incompatible input refused. |
| Daemon configuration | 14 | Exact owning reader; incompatible input refused. |
| CLI JSON output | 2 | Exact owning reader; incompatible input refused. |
| Managed resource request / inventory | 3 | Exact owning reader; incompatible input refused. |
| External control / authenticated cursor | 3.1 / 2 | Exact owning reader; incompatible input refused. |
| Peer protocol and catalog messages | 1.6 | Exact owning reader; incompatible input refused. |
