# Canvas experience implementation handoff

## Current assignment

The user assigned S02 on October 24 and resumed it on October 26. Implementation and focused Rust,
frontend, actual-binary and browser verification are complete. S02 stops ready for the S03 user review.
S03–S05 and main 05–07 remain unassigned. The planning checkpoint is `851d216d`; S01 implementation
is `1225fa0e`, with its handoff at `974c438b`. S02 implementation and its checked consumers are
committed at `6c34677b`. This record does not
claim the side-sprint full gate, main-sprint completion or actual user acceptance.

## Delivered boundary

A project now contains discoverable empty workflows with daemon-saved unfinished cards, sources,
wires and usual-model choices. Positions save separately. Placing a blank task opens no inspector;
selecting it opens the right inspector while the canvas remains in place. The persistent daemon →
project → workflow tree, direct connections and accessible list replace ordinary protocol forms.

Shared control constructs executable definitions only for the deliberately reviewed Run/Apply scope.
Conditional checkpoints and exact command receipts have distinct recovery paths. Run atomically
retains remaining ideas and maps accepted cards; inspecting a card does not choose what runs.
Current authority, project and adapter contracts remain enforced by Rust. Stable output promises
permit later connections to queued work, independent order remains distinct from data prerequisites,
and accepted historical predicates and frozen starts are preserved.

Control protocol 3.6, generated TypeScript, native clients and CLI upgrade together. Physical/internal
storage formats are 22/22; supported physical 19/20/21 stores are adopted transactionally without
rewriting frozen records or exact receipts. No user store was reset or running user service replaced.
The latest authoring snapshot and positions are bounded. Explicit cleanup frees obsolete canvas
references and off-page positions while preserving accepted work, pending connections and standalone
ideas. Methods remain S04 work; there are no placeholder method controls.

## Verification and reproduction

[Canonical S02 evidence](../../verification-evidence.md#side-s02-saved-authoring-and-continuous-canvas)
records executed targets, counts and limits. All affected Rust owner/client suites, storage/adoption,
public recovery, authority, scoped static checks and API review pass. Frontend check/lint, 107 unit
tests and production build pass. The actual-binary daemon/CLI check passes at
`target/canvas-experience/s02-application-final-2/`, whose report confirms fixture shutdown.

The first 44-case browser run exposed failures; its artifacts and timing report are preserved under
`target/canvas-experience/s02-browser-1/first-run/`. Repairs cover both stale form interactions in tests
and actual projection, connection, correction and queued-output defects. The second complete run
passed 39 of 44 cases. Its artifacts and timing reports remain under
`target/canvas-experience/s02-browser-2/full-run/`. The 17-case affected rerun passed all five prior
failures and exposed two test assumptions, repaired before the final four-case phone run passed.
All 44 cases have passing coverage across those invocations. The final bundle is `index-BhM2prSY.js`;
artifacts remain under `s02-browser-3/focused-run/` and `s02-browser-4/final-run/` within the same
ignored canvas-experience directory. Only the owned temporary daemons were stopped. Fixture roots contain credentials;
never publish `browser.json` or whole roots. Raw logs use ignored `target/s02-*`.

For focused reproduction:

```sh
cargo test -p milkdrift-control --test authoring_validation --test authoring_authority --all-features
cargo test -p milkdrift-redb-store --test authoring --test daemon_schema_adoption --all-features
cargo test -p milkdrift-daemon --test control_plane --all-features saved_authoring
```

The [web guide](../../../../apps/web/README.md) describes the production build, browser checks,
connection and ordinary authoring journey. The [evidence harness guide](../../../../tools/evidence/README.md)
describes fresh `serve --canvas-cases` fixtures and actual-binary checks. Use a new root for a full
browser suite; its cases deliberately change fixture state. Keep matching daemon/client builds.

## Next boundary

Stop ready for [S03](S03-early-user-review.md), which requires assignment and the actual user's
participation. It must test the requested simplicity and flow with useful input-dependent work;
automated passes and inspected renders do not close prior user objections. S04 owns method authoring
and activation, S05 the full side-sprint gate, and main 07 the later integrated gate. Physical Podman,
physical-phone/HTTPS and real-model quality claims remain outside these controlled checks.
