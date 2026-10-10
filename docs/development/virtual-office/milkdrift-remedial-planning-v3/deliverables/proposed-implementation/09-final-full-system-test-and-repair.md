# P09 — Final full-system test, repair and acceptance

Owner: integrated acceptance coordinator with independent review. This is the named final full
gate owner under the approved multi-phase schedule. Requires P08, all selected required branches,
authentic user checkpoints and a current reviewed source state. Read [shared context](context.md)
and the current canonical workflow, evidence lanes and frontend practice; exact current policy wins
over stale command examples below.

Audit adoption before testing: every selected outcome has a complete public daemon/workbench path;
all actual producers/consumers/fixtures/examples/docs use the chosen representation; obsolete
construction helpers, competing authorities, private client shortcuts and dual writers are removed.
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

Use actual compatible pinned tools/scripts adopted in P01, recording any justified changes,
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
