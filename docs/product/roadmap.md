# Roadmap

Ordered unfinished work; [status](status.md) owns implementation and evidence facts.

The active [client-ready workflows sprint](../development/virtual-office/client-ready-workflows/README.md)
completes the ordinary CLI route through shared public daemon operations. Setup, workflow
authoring, supplied-input execution, exact request recovery, result inspection and the supported
prospective repair, reuse/copy, separate publication preparation, evaluation commands and the
independent client boundary are complete. The next assignment is
[05a — Enable stricter checks](../development/virtual-office/client-ready-workflows/05a-enable-strict-checks.md),
under the prepared [linting insertion](../development/virtual-office/client-ready-workflows/linting-insertion.md).

Work proceeds in this order:

1. Enable the assigned strict static checks, then fix their findings in 05b.
2. Run the integrated operator journey, full gate and authorized local-model checks in 06; fix failures
   before closing the sprint.

The [verification schedule](../development/virtual-office/client-ready-workflows/README.md#test-as-you-build-test-the-full-system-in-06)
assigns focused checks to 01–05 and full acceptance to 06. Early handoffs do not establish full-system
acceptance. Missing required model access keeps final acceptance open. Each prompt assigns its own
bounded work; adopting the sprint does not automatically execute later prompts.

Svelte is the first future GUI, and every frontend uses the daemon's public operations. This sprint
adds no GUI code or dependencies and does not authorize an automatic GUI successor.

The previous adaptive methods and independent hosts implementation is closed. Its integrated
physical acceptance includes explicitly assisted application source; unaided model coding was not
a closure requirement. Preserve that behavior and its regression coverage while completing the
client route.

The managed resource, protected adaptation, publication and evaluated-learning implementations
have finite desktop and UM790 qualification. Current coverage and limits belong to
[status](status.md#current-validationevidence-snapshot). Broader managed Vulkan, active-reboot and
memory-pressure claims remain separately scoped questions in the
[hardware qualification issue](../development/virtual-office/whiteboard/issues/managed-linux-hardware-qualification.md).

Use the maintained [Slotbook example](../guides/adaptive-method-example.md) for the operator route.
A learning outcome may be negative or inconclusive; demonstrating the mechanism does not require
fabricating a better method. Preserved model failures and uncertainty remain evidence even when an
authorized assistant supplies a corrected implementation to continue the workflow.

Keep native trusted execution, attached endpoints, ordinary files, external inference, scoped
knowledge, local-first operation and shared administration. Linux is the first managed mechanism;
existing portable behavior and future Windows/macOS clients remain part of the product direction.
No live production deployment, data destruction, or host privilege change follows from this plan.

GUI implementation, an inference engine, new cloud-provider families, a discovery/VPN mesh,
shared multi-host storage, consensus/failover, a general package manager, custom supervisor,
second scheduler, global static/dynamic modes, and unrelated graph primitives remain excluded.
Necessary contract and ownership changes are authorized; unrelated architectural cleanup needs
a demonstrated defect or a separately authorized operator need. The whiteboard's generation-policy
and external-configuration-provenance discussions remain separate work.
