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

Failed helper observation has a bounded force/reap allowance and returns unconfirmed cleanup to
the caller. Worker execution attempts container removal after both ordinary errors and observation
panics. A failed removal retains the durable resource use for recovery; releasing the creator's
registration only permits the recovery fence to inspect the container, and does not prove it absent.

A worker's prepared duration includes its task timeout and the finite helper deadlines for
creation, verification, removal and absence inspection, including each helper's force/reap wait.
Managed model calls preserve the model adapter's local HTTP deadline. Neither deadline proves
physical quiescence; failed cleanup retains the managed hold and the workflow reservation.

Start with [managed operations](../../docs/operations/managed-linux.md) and the
[maintained inputs](../../examples/managed-linux/README.md). Unit/contract tests exercise strict
readers and the semantic/persistence boundary; `linux_mechanism` is an explicitly gated real-host
lane. Deterministic fake-platform results do not qualify OS enforcement, Vulkan or reboot recovery.

Independent resource evaluation and publication remain available through the CLI. A passed finite
HTTP evaluation is distinct from publication intent, verified service state, task success and
general application correctness. The retired workflow-backed method engine is not required for
these managed-resource guards.

The generic lifecycle adapter cannot declare one overall platform-operation deadline. Finite
workflow and serving admission therefore refuse it before adapter entry. Lifecycle conformance
tests still exercise its reporting and cancellation mechanics with an authorized test context;
a separate production serving test verifies this refusal and zero platform effects. Administrative
resource commands continue through `ManagedResources`, with their own authority and durable receipts.
