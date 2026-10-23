# Roadmap

The product account and selected implementation choices are recorded for the
[daemon-workflows implementation sprint](../development/virtual-office/daemon-workflows-implementation/README.md).
**Prompt 03 is complete locally**: model-authored future batches, bounded tools, parallel research,
correction and guarded human editing pass the assigned focused checks. The user's October 17
instruction authorized proceeding while actual user review in 02 remained pending as an evidence gap.
The subsequent partial review reports successful Studio A connection and requests simpler setup,
daemon-owned names and a visual redesign using the supplied mockup and real logo. The assigned correction
[02a](../development/virtual-office/daemon-workflows-implementation/02a-daemon-connection-identity.md)
now implements simple connection and shared names, with the user's recheck pending.
The subsequently assigned
[02b](../development/virtual-office/daemon-workflows-implementation/02b-visual-design-and-brand.md)
implements the supplied dark visual direction and exact logo across the client. The user subsequently
rejects its forms, navigation and unclear workflow creation. Subsequently assigned
[02c](../development/virtual-office/daemon-workflows-implementation/02c-workspace-and-task-creation.md)
implements contextual task creation, explicit workflow/addition actions and bounded navigation.
The user rejects its remaining page/form model and requests a canvas with left-side details and
direct input connections. Assigned
[02d](../development/virtual-office/daemon-workflows-implementation/02d-canvas-and-direct-authoring.md)
replaces that authoring model and supplies the narrow daemon contracts needed for truthful connectors.
Its implementation and focused verification are complete. The October 19 review identifies the
missing project home and unclear relationship between folders and execution environments. Assigned
[02e](../development/virtual-office/daemon-workflows-implementation/02e-projects-and-execution-environments.md)
adds daemon-owned projects, workflow grouping, shared execution defaults and explicit Podman project
attachments while preserving user folders. Implementation and focused verification are complete;
positive physical Podman attachment qualification remains open on this host. The prepared
[canvas experience side sprint](../development/virtual-office/canvas-experience/README.md) consolidates
the October 23 design discussions under [ADR 0052](../decisions/0052-canvas-authoring-and-method-experience.md).
The next order is **side S01 → S02 → S03 → S04 → S05 → main 05 → main 06 → main 07**.
S03 fulfills outstanding main 02 only after actual review; S04 takes over all unimplemented main 04
method work. Main 04 is superseded as an assignment, not completed. The side includes daemon lifecycle
repairs, saved authoring, the coherent canvas and method experience, and its own S05 full gate.
Prompt 03 stays complete and is not replayed.
Earlier checkpoints and stores remain preserved; no corrected hosted run is claimed.
Side S01 is complete locally with its focused evidence. S02 is next prepared work;
S02–S05 and main 05–07 remain unassigned.
Main 07 owns the final integrated test-and-repair gate
after remote tools, retention and main 06 review; the side gate does not pre-qualify those changes.
The superseded remedial P00–P09 is historical evidence only.

[Vision](vision.md) owns the intended experience; [ADR 0051](../decisions/0051-daemon-task-workflows.md)
and its [0052 refinement](../decisions/0052-canvas-authoring-and-method-experience.md) own the selected
design. [Status](status.md) owns the current implementation and its qualified evidence.
The new program delivers these outcomes in order:

1. A real standalone Svelte client submits a useful task to its daemon, inspects results and
   reconnects from a fresh client; the ordinary workflow replacement is adopted end to end.
2. An actual early user review tests that journey and repairs behavior and usability findings.
   Its implemented 02a correction simplifies connection and shares the daemon's name; 02b applies the
   supplied visual direction and logo. Implemented 02c addresses the rejected authoring/navigation
   experience, but its user recheck rejects the remaining protocol forms. Assigned 02d replaces them
   with canvas authoring and compact left-side selection. Its focused checks are complete. Assigned
   02e makes projects and their execution location explicit; its focused checks are complete. Recheck
   the replacement through side S01–S03, including naming and a useful input-dependent result,
   before adding methods. Side planning does not close actual-user acceptance.
3. Models and humans build parallel research, composition and corrective future work through the
   same commands, with explicit readiness, authority and cumulative limits.
4. Side S04 delivers the transferred daemon-wide method library and visual authoring/use, with current
   content at activation, explicit export release, eligible local future edits, restart-safe expansion
   and cross-daemon copies. Side S05 verifies useful reuse and runs its full current gate before handoff.
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
first GUI in these finite assignments; planning either sprint authorizes no production phase or
automatic successor. Early snapshot method customization, arbitrary method-owned adaptive descendants
and scoped human-decision gates remain separate follow-up proposals, not hidden side-sprint work.
