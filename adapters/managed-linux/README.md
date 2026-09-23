# Managed Linux adapter

Use this adapter when an approved working environment or model service must outlive its creating
invocation. `LinuxManagedPlatform` implements capability-host's resource lifecycle through rootless
Podman and systemd/Quadlet. The daemon composes it with the same redb inventory used for local and
serving acceptance. Core runtime code sees exact resource bindings and execution facts, not Linux
commands.

`LinuxRecipe` supplies workload-independent finite images, limits, networking and model inputs.
`ProtectedServiceRecipe` additionally fixes a candidate verifier, target policy and isolated service.
Only managed publication can replace that service’s candidate; the separate worker emits immutable
stdout artifacts when `stdout_artifact` is true. `ManagedWorkerAdapter` owns temporary task containers through physical removal;
`ManagedModelAdapter` preserves the existing model adapter's prepared bytes and adds the durable
service dependency. Systemd owns persistent servers. The adapter never mounts manager state or
engine sockets into the worker and never removes shared inputs or external attachments.

Start with [managed operations](../../docs/operations/managed-linux.md) and the
[maintained inputs](../../examples/managed-linux/README.md). Unit/contract tests exercise strict
readers and the semantic/persistence boundary; `linux_mechanism` is an explicitly gated real-host
lane. Deterministic fake-platform results do not qualify OS enforcement, Vulkan or reboot recovery.

The [adaptive Slotbook example](../../examples/adaptive-slotbook/README.md) composes both recipes
through the CLI. A passed finite HTTP evaluation is distinct from publication intent, verified
service state, workflow success and general application correctness.
