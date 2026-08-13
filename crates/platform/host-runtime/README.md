# host-runtime

Host-process platform support for native runtime execution.

It quarantines Flume, `std::thread`, `Instant`, and the short-lived synchronization
used by frame-pull output batching. Runtime crates receive stable wrapper types
rather than importing Flume or coupling orchestration to concrete host primitives.
It does not own application, inference, or workflow state.

Text and token output expose separate typed cursors, ranges, records, and borrowed
batches. Both wrappers use one private statically dispatched bounded core for
mutex ownership, fixed capacities, monotonic cursor checks, atomic push, pull, and
allocation reuse. Producers use non-blocking `try_lock`, complete every fallible
check before mutation, and never resize admitted storage. A consumer pulls and
logically clears one borrowed batch on its own cadence while both allocations are
retained for reuse.
