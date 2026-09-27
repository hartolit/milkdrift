# Remaining managed Linux hardware qualification

The assignment 02 follow-up at baseline `78de0bf` replaces the obsolete Podman restriction and
qualifies real workers, owned CPU inference, service recovery without Milkdrift, and ownership-safe
cleanup on desktop Arch Linux/Podman 6.1.2. CPU, memory and PID controllers are delegated and their
actual kernel limits pass. These are no longer blockers on that host. The
[managed evidence guide](../../../verification-evidence.md#managed-linux-installations)
owns commands, exact inputs and results.

## Evidence and remaining boundary

Drifty now has an operator-prepared dedicated account, delegated controllers, rootless Podman
6.1.2 and matching kernel/TUN support. Integrated acceptance exercised physical direct/workflow calls,
native seeded Slotbook protection and an owned CPU service using the exact Ornith 35B weights.
An orderly operator reboot automatically restored that service before Milkdrift started. Completed
calls replayed without new entry; drained replacement preserved the working area, and removal
retained source, output and data. The
[integrated evidence guide](../../../verification-evidence.md#integrated-host-acceptance) binds the
exact inputs and independent observations. These completed setup and idle-recovery checks are no
longer blockers.

The independently attached native server uses Vulkan. That does not exercise Milkdrift-owned
Vulkan containers or qualify controlled shared-memory pressure on the UM790. Simultaneous native
Vulkan and owned CPU loading was observed with free memory; no stress or performance guarantee
follows. All generation was serialized. Neither an orderly reboot with completed calls nor daemon
interruption establishes physical power-loss recovery or recovery of active inference across reboot.

The remaining hardware work needs an exact approved Vulkan container image, device permissions,
and an explicit pressure/interruption plan within the machine's shared memory pool. Preserve actual
offload and successful inference evidence, then test the specific recovery claim. Do not clear this
issue from native endpoint health, CPU inference or the completed idle reboot.

## Questions of purpose

- Which owned Vulkan configuration, beyond the qualified CPU and attached-native paths, is worth
  claiming support for? Attachment and CPU operation do not satisfy an owned-Vulkan claim.
- Which reboot and pressure failures must Milkdrift recover from, and which require operator repair?
  What retained evidence distinguishes an interrupted call from a safely restartable service?
- Can an isolated maintenance window and the existing host establish these claims, or is a separate
  test host needed to avoid disrupting active inference? Tests should justify a support claim,
  rather than merely demonstrate that a second server can start.

2026-09-22 — Birch-20260922-a (agent pseudonym), assignment 02 review follow-up: recorded the
remaining host-level work after executing desktop Podman 6.1.2 qualification. This is the same
review session, not an independent review or a claim of consensus.
