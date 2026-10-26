# Current contract versions

Owning constants, strict readers and golden fixtures define these versions. Unsupported versions
are refused except for the narrowly supported physical store adoption described below; it does
not revive graph readers or convert historical workflows. Task
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
| Redb internal document format / physical schema | 22 / 22 | Current format, plus bounded adoption of physical 19, 20 or 21 with internal format 22; other formats refused. |
| Runtime task workflow document | 3 | Exact owning reader; earlier task documents refused before mutation. |
| Local-process profile / host materialization | 2 / 1 | Exact owning reader; incompatible input refused. |
| Daemon configuration | 14 | Existing version-14 files remain valid; optional `name` seeds absent metadata and defaults to `Milkdrift`. |
| CLI JSON output | 2 | Exact owning reader; incompatible input refused. |
| Managed resource request / inventory | 3 | Exact owning reader; incompatible input refused. |
| External control / authenticated cursor | 3.6 / 2 | Exact owning reader; incompatible input refused. |
| Peer protocol and catalog messages | 1.6 | Exact owning reader; incompatible input refused. |

Opening a physical-schema-19, -20 or -21 store is supported only when its internal document format is exactly
22 and its original tables and required metadata pass validation. One transaction adds the three
project registration, operation and workflow-association tables when absent, adds the two daemon metadata tables
if adopting schema 19, adds separate authoring and position tables, and advances the physical marker to 22. Existing workflow,
receipt and installation-identity rows are not rewritten. Interruption leaves either the original
format or the complete adopted table set; a later open can continue. The daemon then initializes
its absent name from configuration. This is an addition of bounded metadata to the current task
store, not a migration of obsolete graph or earlier task documents. Older executables that require
physical schemas 19, 20 or 21 cannot reopen the adopted store.

Control 3.2 adds required session name fields, naming routes and the explicit daemon-identity
mismatch code; daemon and clients upgrade together. Grant schema 5 is unchanged, but adding
`RenameDaemon` changes the Controller preset's grant content. Existing immutable grant claims are
not rewritten by store adoption. Follow the [authority upgrade guidance](../operations/daemon.md#configuration-and-startup)
before changing configured grant revisions or continuing work with an older frozen claim.

Control 3.3 adds safe task-authoring contracts, permitted project-root choices, named published
outputs, bounded shared-source reads and an optional authoring-generation guard. Daemon and clients
upgrade together. Stored task definitions and descriptor digests are unchanged; existing canonical
requests and receipts remain exactly recoverable.

Control 3.4 adds daemon-owned projects, scoped folder/environment discovery, exact creation recovery,
workflow associations and typed project/default versus explicit task execution settings. New ordinary
workflows require a project; earlier canonical requests are decoded for receipt recovery before that
requirement is checked. Optional runtime fields retain their absent historical encoding. Association
does not rewrite accepted tasks or receipts. `InspectProject` and `ManageProject` extend operation
presets, changing affected grant digests; unchanged authority schema does not imply unchanged grants.
Complete unfinished old work with its matching binary/configuration before upgrading those presets.

Control 3.5 adds `entered` to compact task summaries so clients distinguish frozen starts from
durable external-entry intent without fetching captured inputs. Upgrade daemon and clients together.
Pause and temporary-contention repairs preserve the current task/store format and exact receipts;
queued tasks can retain an unentered frozen start and its original `started_at`.

Control 3.6 adds empty workflow creation, bounded conditional authoring and position snapshots,
reviewed Run/Apply, compact topology, workflow rename and advertised authoring bounds. New dependency
maps state independent success or settlement ordering; historical arrays retain their original
input-dependent predicates when read. Fresh definitions use maps. The optional capability generation
guard is retained in new definitions and checked again before start; old absent fields keep their
original encoding. Physical schema 22 adds authoring storage and replacement accounting without
rewriting accepted definitions or receipts. Exact old command replay precedes current validation.
