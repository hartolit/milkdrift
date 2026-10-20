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

Ordinary project tasks attach their accepted native folder at `/project`, which is also the
working directory. The adapter pins the directory before entry. The daemon supplies its maintained
Rust helper, which opens the same native identity inside Podman's rootless namespace and retains
its descriptor until cleanup. The adapter gives Podman that descriptor path, then verifies the
exact realized source, destination, access and user mapping before starting
the workload. Replacing the registered path refuses entry. The attachment is nonrecursive, private
and read-only unless the accepted task permits writes. No project mount uses `:U`, relabel options
or owned-volume cleanup. The read-only image root and bounded temporary filesystem remain in force.
Without a project binding, the original owned `/workspace` contract remains unchanged.

Embeddings must supply that helper explicitly through `with_project_pin_helper`; a missing helper
refuses attached execution. Its startup output, lifetime and process-group termination are bounded.
The adapter also checks the engine's reported storage and runtime roots, including custom locations,
so an enclosing directory cannot expose them through a project attachment.
It refuses a project containing its configured helper executable, so project writes cannot replace
the host program used by a later invocation. Daemon registration protects its other concrete
configuration, credential and profile inputs before either native or container project tools run.
The OCI runtime must support mounting that held descriptor across its namespace setup. A configured
worker and successful owned-volume verification do not prove this additional prerequisite. If the
runtime refuses the source, the task reports its bounded diagnostic and verifies container removal;
it does not substitute an ordinary host pathname or change runtime privileges.

The attached worker runs as a non-root container UID/GID with only that identity mapped to the
daemon's rootless user; other identities remain in the private automatic allocation. The supported
folder must belong to that user's UID and primary GID and allow the selected access. The adapter
refuses unsupported mappings or inaccessible labels and never changes user ownership, permissions
or security labels to make a task run. Commands use container paths such as `/project/src/main.rs`;
native file tools use the accepted host folder. Those paths are not interchangeable.

Model process tools use the same verified worker and installation generation. Each entered tool
gets a bounded durable use linked to its already-entered parent task. Failed container cleanup
retains that use and the parent's native write claim; maintenance and conflicting work cannot
assume the writer stopped. Verified removal ends the tool's resource lifetime. A lost response
still leaves the parent responsible for uncertain result reporting and workflow outcome, including
its native path claim. Generic installation administration
never appears in the model's process-tool catalog.

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
