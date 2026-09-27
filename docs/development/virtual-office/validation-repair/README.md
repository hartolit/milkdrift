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
Next: retained store capacity and verifier scratch, then CI survivors, then integrated gate/API review.
CI job logs are available through the GitHub connector; the exact survivor list and publication test
log are under `target/validation-repair/`.
