# Platform crates

Process-host facilities used by runtimes belong here when they are neither domain
logic nor an integration with a model, storage system, network service, or vendor
SDK.

Current platform crate:

- `host-runtime`: bounded channels, named threads, monotonic time, and bounded
  token/text output accumulation for hosted execution.

`host-runtime` keeps its existing name because it describes the host execution
substrate it wraps. Renaming it to a broad term such as `native` would imply a
larger platform abstraction that does not exist yet.

Platform and adapter crates occupy the same lower infrastructure layer in the
current dependency policy, but are separate physical categories because they have
different reasons to change. Only the current `host-runtime` platform role is
registered. Adding another platform crate requires an explicit architecture
change rather than gaining a role from directory placement.
