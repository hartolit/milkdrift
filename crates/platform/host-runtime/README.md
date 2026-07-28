# host-runtime

Host-process platform support for native runtime execution.

It quarantines Flume, `std::thread`, `Instant`, and the short-lived synchronization
used by frame-pull output batching. Runtime crates receive stable wrapper types
rather than importing Flume or coupling orchestration to concrete host primitives.
It does not own application, inference, or workflow state.

The text output accumulator allocates its byte and record storage once during
setup. The separate generic token accumulator preallocates token IDs and ordered
request/state records, exposes absolute monotonic token ranges, and accepts a
`Copy` state payload defined by the inference engine. Inference producers use
non-blocking `try_lock`, validate all capacities before mutation, and never resize
the buffers. An application consumer pulls and clears one borrowed batch on its
own cadence while retaining the allocations for reuse.
