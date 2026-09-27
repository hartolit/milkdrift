# Validation and CI repairs

The September 28 operator request authorizes fixes for the independent validation at `a07bcc6`
and mutation run 36323713449. The implementation and documentation practices govern this work.
One agent owns the changes, on `codex/validation-retention-ci`, in recoverable commits.

## Work and acceptance

1. Publication retirement: reclaim idle registry generations through production operations;
   preserve pending calls, immutable definitions and exact replay across complete reopen.
2. Retained storage: separate active installations, evaluations and publications from historical
   evidence, preserve replay/conflicts/references, and test turnover beyond small bounds. Define
   verifier scratch retention and safe cleanup with its mechanism owner.
3. CI: close the surviving peer, authority and controller mutations from the linked run, verify
   the exact mutations, and update the deprecated artifact action. Baselines passed in all nine
   failed jobs; these are surviving mutants, not ordinary baseline failures.
4. Review the integrated result and run the full local gate and affected API inventories. Do not
   repeat the retired sprint 06 product/hardware workflow. Remove this temporary directory when
   complete; lasting behavior and evidence belong in their canonical owners.

## Current handoff

Repository started clean at `a07bcc6`. Read both practices, workflow, relevant product/architecture
guidance, validation report and referenced source evidence. Publication retirement now reclaims idle
adapters through service maintenance and restores retired generations only for pending calls.
All 22 publication tests pass, including six generations across full store reopen with a two-slot
registry, preserved pending calls and exact historical replay. The daemon invokes this maintenance.
CI boundary tests now cover hot/archived serving records, independent authenticated-grant facts,
lease/deadline entry, nested accounting, published controller assessment and poisoned status health.
Removed a duplicate pre-write publication association check; the transactional writer validates it.
Artifact uploads use the pinned Node 24 action. Focused tests pass. Direct mutation qualification is
in progress; the first reader run caught 16/18, and the two output-count boundary gaps were corrected
and the reader tests rerun. Requalify them before closure. One nested-account predicate is explicitly
classified with a supported-transition explanation and zero-dimension/reopen coverage.
Next: finish scoped mutation qualification, active store indexes and verifier scratch, then full
gate/API review. CI logs, exact survivor list and focused results are under `target/validation-repair/`.
