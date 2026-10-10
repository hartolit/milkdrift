# Proposed implementation program — not assigned

This is the unexecuted route from the inspected Rust product to a real standalone Svelte product.
The user must first review the planning decisions and authorize a bounded implementation scope.
Listing a prompt here does not activate it. [Shared execution context](context.md) supplies the
source, decision and verification rules every prompt requires.

| Assignment | Complete responsibility | Prerequisites | Usable result and focused verification |
| --- | --- | --- | --- |
| [P00](00-adopt-reviewed-design.md) | Adopt approved decisions/practice and reconcile source | User decision on this proposal | Canonical intent/architecture/roadmap accurately authorize work; documentation contracts. |
| [P01](01-browser-and-production-foundation.md) | Real browser transport and retained Svelte foundation | P00, selected A1/F1 deployment | Two real authorized owners reachable, identity/session/stream boundaries; HTTP/config/browser contract checks. |
| [P02](02-first-connected-workflow.md) | First real author/input/run/result/recovery journey | P01 | Maintained UI used with actual daemon, different briefs and lost replies; focused real-daemon browser tests. |
| [P03](03-first-human-review.md) | User critique and resulting corrections | P02, actual user participation | Product/architecture feedback addressed, dependencies reopened where needed; affected checks. |
| [P04](04-structured-authoring-and-notation.md) | Complete structured authoring, notation and choice continuation | P03, reviewed P2/N1; R-C01 decision | Same semantics through graph/list/agent/daemon; structured/reconciliation/round-trip cases. |
| [P05](05-reuse-services-and-independent-owners.md) | Reuse, restricted services, direct work and multiple owners | P04 and P1/A1 | Real author and invoke-only journeys; service relation/discovery remedies; peer/replay/privacy checks. |
| [P06](06-adaptation-and-managed-work.md) | Adaptation, tools/resources and composed lifetimes | P05 | Legitimate child progress plus protected-effect and maintenance refusals; managed/authority/recovery checks. |
| [P07](07-knowledge-and-evaluated-reuse.md) | Selected knowledge, evaluation and prospective promotion | P06 | Negative/inconclusive/eligible outcomes, private evidence and independent variants; learning/context tests. |
| [P08](08-compound-human-review.md) | Demanding combined work and actual user critique | P07, actual user participation | S21/S26/S28 together, counterexamples repaired and decisions rechecked. |
| [P09](09-final-full-system-test-and-repair.md) | Final full-system test, repair and acceptance | P08 and all selected branch work | Integrated current Rust/frontend gates and real journeys; accurate canonical evidence. |

These are coherent boundaries, not a limit on justified repairs. Each author fixes its own discovered
defects; P09 is not a backlog for known failures. P01/P02 deliver retained production code early,
then P03 can reopen backend/product decisions. Advanced capabilities remain assigned through
P04–P08, rather than disappearing behind “first version.”

The explicitly proposed multi-phase schedule assigns the full local Rust gate and integrated
frontend/application acceptance to **P09**. Before implementation, P00 must register that schedule
under the [workflow policy](../../../../workflow.md#explicit-multi-phase-sprint-schedule). Earlier
prompts run focused behavior/static/docs checks and report limits; they are not full-system
acceptance. CI protections remain enabled throughout.

## Conditional branches and exact decisions

| Branch | Question and current consequence | Work needed before implementation can be ready |
| --- | --- | --- |
| B1 Cross-owner relay/migration | Does the user need a direct client at execution-only C to relay into B, or workflow-owner migration? Neither is inferred from multiple connections. Existing client→B and A-workflow→B remain usable. | Identify caller/origin, privacy and recovery promise; run exact current route; compare direct connection, explicit transfer and relay. Add complete owner/authority/account/retirement tests before selecting new semantics. No ready relay/migration task is claimed. |
| B2 Release from management | Is preserving files on removal sufficient, or must an active service become independently administered? Current code has no detach action. | Decide active-use fencing, configuration/secret/supervisor transfer, drift, rollback and operator responsibilities; require a real-host plan. P06 exposes actual lifecycle but cannot pretend detach exists. |
| B3 Standard interchange/core replacement | Is tested UML/BPMN import/export or canonical regions worth the migration cost? Principles-based notation alone does not answer this. | Choose concrete external artifacts and required semantic subset; test round trip, unsupported constructs and current-data transition. Reopen P2/N1/P04 rather than add a second compiler. |
| B4 Stronger credential/private-record custody | Is persistent login, encrypted shared-device storage or cross-device recovery required beyond F1's explicit personal-profile IndexedDB or export-before-send choice? | Select custody/key/revocation/clearing and operator responsibilities. P01/P02 already require exact request durability before submission without stored bearer tokens; they cannot promise protection from other users of the same browser profile or automatic recovery on another device. |
| B5 Broader network/platform qualification | Which remote HTTPS/overlay/browser and later desktop environments should be supported beyond the initial verified lane? | Supply controlled endpoints and certificate/network setup, run auth/stream/recovery/download tests; no blanket compatibility claim from loopback Chromium. |

Conditional branches are not an indefinite backlog for currently working outcomes. They name
unapproved additions or a broader promise whose need/proof is unresolved. If user review makes
one a required outcome, finish its bounded design and insert complete assignments before P09;
the selected program cannot be declared complete while that requirement is only this table.

## Outcome and reverse audit

V01 → P01–P03; V02 → P02/P04/P08; V03 → P04/P06/P08; V04/V05 → P04/P05;
V06/V08 → P01/P05; V07 → P06; V09 → P04/P06; V10 → P07/P08; V11 → every boundary,
especially P01/P05/P06; V12 → P02/P05/P06/P09. S01–S30 remain in the
[dossier](../../working/system-dossier.md). R-A01/02 → P01/P02; R-A03 → P06;
R-A04/05/06 → P05; R-C01 and R-FE02 → P04; R-FE01 → P07.
B2 corresponds to R-A07, not an already approved remedy.

Every new backend responsibility therefore has a user action and proof destination. No generic
framework, new provider family, second scheduler, inference engine or unrelated cleanup is planned.
Root-cause remedies are public browser support, complete semantic authoring, exact owner-qualified
views and missing authorized discovery/relationships, rather than new names over hidden private paths.
