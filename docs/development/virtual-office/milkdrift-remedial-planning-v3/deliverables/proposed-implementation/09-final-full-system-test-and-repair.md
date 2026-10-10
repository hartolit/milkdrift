# P09 — Final full-system test, repair and acceptance

Owner: integrated acceptance coordinator with independent review. This is the named final full
gate owner under the approved multi-phase schedule. Requires P08, all selected required branches,
authentic user checkpoints and a current reviewed source state. Read [shared context](context.md)
and the current canonical workflow, evidence lanes and frontend practice; exact current policy wins
over stale command examples below.

Audit adoption before testing: every selected outcome has a complete public daemon/workbench path;
all actual producers/consumers/fixtures/examples/docs use the chosen representation; obsolete
private semantic construction, competing authorities and client shortcuts are removed. Explicit
versioned graph and Program encoders remain supported in one blueprint owner; two source forms are
not two execution authorities. Exact old convenience compilation survives missing-outer-receipt retry.
Search all workspace code/config/examples, not just changed files. Trace current supported stored
definitions, layouts, receipts, accepted active/uncertain work and current CLI through upgrade and
rollback/refusal. No schema conversion silently re-identifies accepted work.

Build real binaries, then run the complete Rust gate:

```sh
cargo build --locked -p milkdrift-daemon --bin milkdrift-daemon \
  -p milkdrift-cli --bin milkdrift \
  -p milkdrift-local-process --bin milkdrift-process-test-helper
cargo strict-checks --output target/remedial-implementation/P09/strict
cargo test --workspace --all-features --no-fail-fast
cargo test --workspace --all-features -- --list
```

Run the exact adopted frontend install/type/lint/format/unit/build/real-daemon browser commands;
the planning candidate is:

```sh
npm --prefix apps/workbench ci
npm --prefix apps/workbench run check
npm --prefix apps/workbench run lint
npm --prefix apps/workbench run format:check
npm --prefix apps/workbench run test:unit -- --run
npm --prefix apps/workbench run build
npm --prefix apps/workbench run test:browser -- --project=chromium --project=firefox --project=webkit
```

Use actual compatible pinned tools/scripts adopted in P02, recording any justified changes,
including the declared Chromium/Firefox/WebKit lanes and accessible non-graph editing, focus,
zoom/density and restrained-motion/contrast checks. Visual snapshots establish regressions, not
human usability. Review changed default/all-feature Rust public API inventories and protocol/fixture
versions under public-API policy. Required missing tools or zero-test filters fail acceptance.

Run maintained actual CLI/daemon authoring, independent-host and relevant controller/publication/
learning evidence, plus the actual Svelte multi-owner journeys. Positive and denial/revocation/
stale conflict/lost reply/restart/uncertain effect cases must operate together. Browser-origin and
proxy/TLS claims require actual selected deployment evidence; physical-resource/platform lanes
remain distinctly qualified and unavailable required lanes keep their scope open.

Include the revised compound oracles: choice with inner satisfied join and unresolved physical
writer permits nonconflicting continuation only under remaining authority/allowance while maintenance
refuses; selected-arm A1 accepted/A2 pending repair; data escape/optional/nested output validation;
private relationship removal after revocation; exact legal large integers through persistence/replay;
full ordinary recovery storage with a permitted stop. Distinguish contract fixtures from real-daemon
and real-browser proof instead of merging their claims.

Exercise a real old-supported-store fixture with Suspended parent, Entered child, old account reservation
and exact saved handoff receipt. The new reader/transition owner preserves IDs/digests/claims/generation
and NoExternalEntry evidence, refuses unsafe return until exact stop proof, settles the original account
and replays the old request exactly. Same-binary reopen alone is not upgrade evidence. Admission closure
must not fabricate a drained resource or terminal child.

Cover all U19 states: immutable closed history, inactive reusable graph, unentered future work,
started/uncertain effects and protected work. Verify graph creation/import/copy/edit under original
rules, positive conversion, precise conversion refusal with successful native pending repair, and
cross-family changed-future adoption without requiring whole-definition equivalence. Preserve legacy
config/dependency fingerprints and occurrence governing revisions. A changed active task configuration
may apply to a later invocation where current reconciliation permits; it must not rewrite the entered
occurrence. Native convenience retry after revision save but before outer receipt must recover the
same v3 revision, not recompile as Program. Test merge-parent ordering against the explicitly selected
proposal base. Cancellation/new work never passes as equivalent prospective repair.

Add commitment cancellation after B has accepted work but before its private child enters, with lost
cancellation; A retains uncertainty/reservation and B enforces its own facts. Test local already-admitted
effect starting after the local fence, successor policy/carry-forward with unchanged counters and no
authority rewriting, no-progress stop, and automatic S28 reconsideration after a fresh-client restart.
Reject human/retrospective knowledge evidence as protected method qualification while preserving its
honest availability as guidance. These are integrated owner checks, not interface labels.
Also select unapproved/negative knowledge under actual read authority with truthful status, and adopt
an ordinary authorized method without falsely requiring protected learning qualification.
Test prepared exact workflow slots under a restricted grant, scope refusal before effect, no implicit
grant from commitment association, and reuse of permitted slots across rounds without allowance reset.
No-progress must distinguish a legacy priority-changing port rename from a harmless Program-ID rename.
Include the observed ordinary nested Any cancellation regression: ancestor evidence must be accepted
consistently by scheduling and timer/wait event validation, with unrelated-sibling/no-source denial.
Graph admission must not become stricter merely to hide this runtime defect or block native repair.

Fix every discovered in-scope defect through its owning boundary with independent regression and
all consumers. Commit meaningful working fixes and preserve diagnostic evidence. Use focused reruns
while repairing, then verify the final integrated executable state with the full gate and combined
journey; don't label a pre-fix run final. CI checks/assertions stay intact, no ignored-test expansion
to obtain green results. Do not claim safety/portability/interoperability beyond executed evidence.

Finally update canonical status, architecture, reference/operator docs, roadmap and evidence with
what now works and limits. Preserve approved future work/real unresolved questions on the whiteboard;
close temporary office material only after lasting facts/dissent survive in their owners. Confirm
clean owned diff, source/binary identities, actual test counts, user checkpoints and rollback limits.
The final report separates implemented acceptance, historical evidence and still-unqualified scope.
Stop; a completed implementation sprint authorizes no automatic successor, deployment or push.
