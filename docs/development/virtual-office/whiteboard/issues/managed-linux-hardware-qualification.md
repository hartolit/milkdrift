# Remaining managed Linux hardware qualification

The assignment 02 follow-up at baseline `78de0bf` replaces the obsolete Podman restriction and
qualifies real workers, owned CPU inference, service recovery without Milkdrift, and ownership-safe
cleanup on desktop Arch Linux/Podman 6.1.2. CPU, memory and PID controllers are delegated and their
actual kernel limits pass. These are no longer blockers on that host. The
[handoff](../../adaptive-hosts/handoffs/02.md) owns commands, exact inputs and results.

## Evidence and remaining boundary

Drifty accepts real model requests through its existing native llama.cpp service. Read-only SSH
inspection found no Podman executable on that host; its listener is bound to the NetBird address,
not loopback. An authenticated SSH forward to that exact listener supports the ordinary Milkdrift
model smoke. This does not exercise Milkdrift-owned Vulkan containers or qualify shared-memory
pressure on the UM790. The desktop CPU test does not establish those properties either.

A machine reboot and power-loss recovery were not executed. The desktop is the active user session;
interrupting it would terminate this review. Starting a second 35B Vulkan service beside Drifty's
existing native service also needs an explicit resource/interruption plan. The user provided the
native service for inference tests, not permission to stop it or reconfigure its host.

The remaining acceptance work needs an operator-prepared rootless host, an exact Vulkan server
image and model, device permissions, memory headroom measured as one shared pool, and a scheduled
reboot. Run the maintained lifecycle lane, preserve actual offload and successful inference
evidence, then inspect retained state after reboot and controlled pressure. Do not clear this issue
from endpoint health or a CPU result.

## Questions of purpose

- Must the initial qualified deployment own Drifty's model service, or is an explicitly attached
  native service enough for the first supported use? Keeping attachment avoids taking over an
  already useful operator service, but does not satisfy an owned-Vulkan claim.
- Which reboot and pressure failures must Milkdrift recover from, and which require operator repair?
  What retained evidence distinguishes an interrupted call from a safely restartable service?
- Can an isolated maintenance window and the existing host establish these claims, or is a separate
  test host needed to avoid disrupting active inference? Tests should justify a support claim,
  rather than merely demonstrate that a second server can start.

2026-09-22 — Birch-20260922-a (agent pseudonym), assignment 02 review follow-up: recorded the
remaining host-level work after executing desktop Podman 6.1.2 qualification. This is the same
review session, not an independent review or a claim of consensus.
