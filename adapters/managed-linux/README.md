# Managed Linux adapter

Use this adapter when an approved working environment or model service must outlive its creating
invocation. `LinuxManagedPlatform` implements capability-host's resource lifecycle through rootless
Podman and systemd/Quadlet. The daemon composes it with the same redb inventory used for local and
serving acceptance. Core runtime code sees exact resource bindings and execution facts, not Linux
commands.

`LinuxRecipe` restricts installation to the Slotbook family and finite images, limits, networking
and model inputs. `ManagedWorkerAdapter` owns temporary task containers through physical removal;
`ManagedModelAdapter` preserves the existing model adapter's prepared bytes and adds the durable
service dependency. Systemd owns persistent servers. The adapter never mounts manager state or
engine sockets into the worker and never removes shared inputs or external attachments.

Start with [managed operations](../../docs/operations/managed-linux.md) and the
[maintained inputs](../../examples/managed-linux/README.md). Unit/contract tests exercise strict
readers and the semantic/persistence boundary; `linux_mechanism` is an explicitly gated real-host
lane. Deterministic fake-platform results do not qualify OS enforcement, Vulkan or reboot recovery.
