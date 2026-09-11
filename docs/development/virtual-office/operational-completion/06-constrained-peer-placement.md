# Assignment 06 — Express and enforce locality and peer requirements

## Outcome

Complete placement for a concrete two-host workflow: one task must run on an approved repository/tool
host, another on a different approved host, with no fallback to the wrong machine. Add the missing
task-side constraints to the existing capability requirement and resolution path. Do not build a
cluster scheduler, discovery service, VPN, global load balancer, peer role hierarchy, or consensus.

Read the sprint README, `AGENTS.md`, canonical docs, workflow and implementation/documentation
practices. Preserve transactional peer admission, durable acceptance, observation paging, tombstones,
authority, and core artifact integration; the review found those improved, not in need of replacement.

## Evidence and source trail

The reviewed descriptors already record `Locality` and authenticated `PeerId`. Requirements cannot
express those restrictions, so revision admission conservatively refuses grants narrowed on them.
That refusal is correct until tasks can state enough to prove authorized selection.

Inspect `crates/capability/src/descriptor.rs` requirements/descriptors, resolved snapshots, authority
resource constraints and revision admission in `crates/runtime/src/engine/authority*`,
`crates/capability-host/`, daemon profile/catalog composition, `crates/peer-protocol/`,
`adapters/peer-http/`, command/config readers, and `docs/operations/authority.md` and `peers.md`.
Reuse existing identity, locality and trust-zone types; do not confuse a remote model endpoint with
an authenticated Milkdrift peer execution host.

## Implement

- Add the smallest bounded task requirement for permitted locality and exact peer identity/set.
  Define absent/empty/wildcard semantics explicitly and compatibly; impossible combinations, such
  as local-only plus a required peer, fail validation. Preserve exact capability/profile/operation/
  trust-zone matching. Update all strict readers, builders, digest owners and fixtures together.
  Old persisted revisions must not silently acquire wider authority or a changed identity.

- Carry the requirement through import/authoring, revision admission, registration matching,
  candidate filtering, deterministic selection, frozen resolved snapshot, final authority check,
  peer acceptance and external entry. Admission may now accept a narrowed grant only when the task
  requirement proves the intended restriction and dispatch enforces the actual candidate. Do not
  simply delete the old refusal while retaining wildcard selection.

- Both availability and authorization matter. No eligible host is a typed unavailable/unsatisfied
  placement result, not permission to use a local tool or unrelated peer. Preserve selected
  descriptor/catalog generation after acceptance. Retry/re-resolution changes placement only under
  explicit existing policy and current authority; never silently migrate entered or uncertain work.

- Make inspection explain requested constraints, chosen authenticated host/generation, and the
  authorized reason no candidate matched. Do not leak forbidden hosts/catalog entries in rejection
  details. A tag such as `trusted` cannot authenticate or authorize. Exact peer/locality plus current
  capability/profile/trust-zone constraints are sufficient for this assignment. Preserve existing
  descriptive tags; add tag selectors only if the demonstrated workflow cannot be expressed with
  these supported facts and the coordinator explicitly accepts that extra scope.

- Preserve repository/tool locality and credential ownership. Remote results move as authorized
  core artifact references; no automatic shared checkout, secret copying, remote filesystem mount,
  or discovery of every tool on the machine. A local HTTP model at a remote URL need not be wrapped
  in a Milkdrift peer.

## Prove a complete two-host use

Use existing two-daemon infrastructure and controlled tool capabilities with identical operation
names but distinct peer identities and observable entry counters. Pin repository A work to host A
and repository B work to host B. Prove both run on the correct host and return exact provenance.
Use temporary loopback peers, not production databases or deployment tools.

Negative cases: unauthorized peer, locally advertised look-alike, expired catalog, health loss,
conflicting exact capability/peer requirements, empty deny set, removed selected generation,
revocation before entry, and reconnect after accepted work. No negative case may increment a wrong
host's entry counter. Race a catalog update against resolution/entry; exact acceptance/replay and
uncertainty guarantees must survive. Restore/restart and inspect the same selected identity.

Test valid narrowed revision admission, contradictory/malformed schema, deterministic candidate
ordering, and the unchanged behavior of unconstrained existing tasks within their current grants.

## Verification and stop

Run capability/authority/blueprint/runtime resolution and final-entry suites, peer protocol/service
and two-daemon tests, updated CLI/config examples and schema fixtures, then the full gate. Update
current placement and authority docs; remove the blanket narrowed-grant refusal claim only for the
dimensions actually implemented. No new provider/network substrate or broad resource scheduler.

Finish when the ordinary workflow path can express and enforce the two-host requirement end to end,
no forbidden/wrong-locality fallback is possible, and observations retain exact placement provenance.
