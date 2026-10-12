# Roadmap

The product account and technical choices are finished for the
[daemon-workflows implementation sprint](../development/virtual-office/daemon-workflows-implementation/README.md).
**Prompt 01 is complete at its focused handoff**, with production daemon, CLI and Svelte integration
and tested local checkpoints `37aa8c4` and `95a4dc5`. The next step is the user's actual review in 02.
Prompts 02–07 remain unassigned; 07 owns the named final full-system
test-and-repair gate. The superseded remedial P00–P09 is historical evidence only.

[Vision](vision.md) owns the intended experience, [ADR 0051](../decisions/0051-daemon-task-workflows.md)
the selected design, and [status](status.md) the current implementation and its qualified evidence.
The new program delivers these outcomes in order:

1. A real standalone Svelte client submits a useful task to its daemon, inspects results and
   reconnects from a fresh client; the ordinary workflow replacement is adopted end to end.
2. An actual early user review tests that journey and repairs behavior and usability findings.
3. Models and humans build parallel research, composition and corrective future work through the
   same commands, with explicit readiness, authority and cumulative limits.
4. A daemon-wide method library expands its current saved templates into editable local future tasks,
   including restart-safe expansion and explicit cross-daemon copies.
5. Clients administer persistent execution-host connections and see remote operations truthfully;
   rolling retention protects required inputs, unresolved work and exact replay.
6. An actual user reviews the combined animation and different-project journeys; findings are repaired.
7. The named final **test-and-repair** assignment runs the full current Rust/frontend gates and
   combined product evidence on the repaired code. Earlier focused passes are not system acceptance.

Preserve the existing independent execution-host and Podman mechanisms and their protections. The
fresh-store replacement may remove obsolete graph, published-workflow and evaluated-learning paths;
archived code remains available for old stores. Planning did not reset stores, change services, or
qualify a new deployment. Unexpected active data discovered during implementation requires a bounded
preservation decision for that data, not a migration framework for hypothetical users.

Inference, new provider families, remote workflow ownership, mesh/discovery infrastructure, central
method services, permanent knowledge/forensic systems and unrelated workflow primitives are excluded.
The [hardware qualification issue](../development/virtual-office/whiteboard/issues/managed-linux-hardware-qualification.md),
generation-policy and external-configuration-provenance topics remain separate. Svelte is the selected
first GUI in this finite sprint; no other frontend or automatic successor is authorized.
