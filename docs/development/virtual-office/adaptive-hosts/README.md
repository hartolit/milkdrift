# Adaptive methods and independent hosts

Prepared 2026-09-17 for `hartolit/milkdrift`, against `855f8ecbb1007baa2a91006384fa322af40ebef9`.

## Outcome

Make Milkdrift useful as a system for developing, adapting, and reusing methods of work. An
independent host supplies authorized execution and managed resources. An adaptable workflow can
repair its method without silently weakening the obligations it accepted. A published workflow
makes that method callable. Experience can produce a better version and isolated product variations.

The sprint finishes with an actual desktop-to-UM790 scenario: establish a protected working setup,
use a local model, develop and verify an application, repair a failed candidate, publish it through
a versioned workflow capability, evaluate a proposed reusable improvement, and preserve useful outputs
when the disposable setup is removed. A negative or inconclusive learning result remains valid evidence
under the fixed criterion. Disconnects, restarts, unauthorized shortcuts, stale evidence,
and failed updates are part of the scenario, not later hardening work.

These are seven sequential assignments, including the direction-adoption assignment. They are
complete responsibility boundaries, not seven small patches or time-box estimates. Continue an
unfinished assignment across sessions instead of treating its number as permission to stop early.

## Start here

This sprint is installed in `docs/development/virtual-office/adaptive-hosts/` and registered in
the [virtual office](../README.md). Start from the assignment table and predecessor handoff.

Execute **00**, then **01–06 in order**. Each agent reads this README, its prompt, and
[the discussion synthesis](discussed-direction.md), followed by the canonical documents and source
required by its assignment. Once 00 has adopted the direction, canonical documents own the accepted
rules; the synthesis remains a temporary record of intent and rationale, not a second specification.

Earlier downloadable patches are not prerequisites; agents must inspect what is actually present
rather than assuming that a previous patch was applied. The shared maintained application is
[Slotbook](../../../guides/adaptive-method-example.md); 02–06 must use its single specification.

The user's request authorizes this implementation sprint and its direction changes, now recorded in
the [roadmap](../../../product/roadmap.md). Do not ask again for permission already given. This does not authorize unrelated
features, production deployments, destruction of user data, or unannounced host privilege changes.

## Assignments and current state

The coordinator maintains this table. Claiming an assignment sets its real session owner; do not
invent completed reviews or additional participants. Implementation assignments remain prepared
until their actual owners execute them; direction adoption is not implementation evidence.

| Order | Assignment | Owner | State | Required outcome |
| --- | --- | --- | --- | --- |
| 00 | [Adopt direction and ownership](00-first-execution-prompt.md) | Alder-20260918-00; review: Rowan-20260918-review | Reviewed and accepted — [handoff](handoffs/00.md) | Canonical intent, architecture, scope, compatibility decisions, and testable implementation boundaries agree. |
| 01 | [Independent host execution](01-independent-host-execution.md) | Codex, implementation; subsequent Codex review-and-commit session | Reviewed and accepted — [handoff](handoffs/01.md) | Direct and workflow-originated work use complete authorized execution, context, artifacts, and recovery without requiring a local workflow role. |
| 02 | [Managed Linux working setup](02-managed-linux-environments.md) | Codex, corrective execution 2026-09-23 | Implemented and desktop-qualified; [post-02 design corrections](02-post-execution-plan.md) executed; hardware follow-up remains — [handoff](handoffs/02.md) | A useful rootless Podman/systemd setup has a complete lifecycle, enforced worker separation, durable ownership, and safe resource-use coordination. |
| 03 | [Adaptation with protected obligations](03-adaptive-methods-and-obligations.md) | Codex, implementation; subsequent review and corrections 2026-09-24 | Reviewed and accepted for finite desktop qualification — [handoff](handoffs/03.md) | Useful autonomous revisions are permitted within scope; acceptance requirements and protected effects cannot be bypassed. |
| 04 | [Published workflow capabilities](04-published-workflow-capabilities.md) | Codex, implementation; subsequent review and corrections 2026-09-24 | Reviewed and accepted for finite desktop qualification — [handoff](handoffs/04.md) | A versioned adaptable method can be called locally or remotely, with durable invocation/run linkage and distinct invocation/edit/publication permissions. |
| 05 | [Learn and reuse better methods](05-learning-and-product-variations.md) | Codex, implementation; review and corrections 2026-09-25 | Reviewed and accepted for finite desktop fixture qualification; 06 remains — [handoff](handoffs/05.md) | Evidence produces an evaluated candidate blueprint, a promotion/rejection/inconclusive decision, and isolated product variants through ordinary workflows. |
| 06 | [Integrated acceptance and closure](06-integrated-acceptance-and-closure.md) | Unassigned | Prepared | The combined system works through product binaries, including real Linux/UM790 evidence, failure recovery, and removal; remaining defects are corrected. |

Do not parallelize these numbered assignments: they deliberately share contract and composition
owners. Within an assignment, delegate only when explicitly assigned and with coordinated files.
One agent owns Cargo jobs. Read each predecessor's short handoff before making dependent changes.
The coordinator accepts coverage, not merely a worker's claim that its tests passed.

Complete the [post-02 execution plan](02-post-execution-plan.md) within 02 before advancing to 03.
It corrects example-specific behavior, model identity, resource policy and operator configuration;
the existing physical evidence does not establish completion of these design corrections.

## Scope and exclusions

Implement execution-only and workflow-enabled hosting in one product; typed direct/workflow origin;
complete input/artifact authority; one managed Linux recipe and service adapter; scoped adaptive
methods; versioned workflow publication; evidence-based method improvement and product variations;
and a complete CLI/client/API path for these operations. Existing local process/model/peer/control
consumers must remain coherent as shared contracts change.

The first managed platform is Arch Linux on the UM790 Pro, with desktop/laptop clients. Use rootless
Podman and systemd/Quadlet as the initial mechanism. Local inference remains an external service.
Keep trusted native tools and attached remote endpoints usable without containerizing everything.
Preserve portable core behavior and existing platform coverage; new platform-specific operations
must return truthful unsupported results elsewhere.

Do not add a GUI, inference engine, new cloud-provider family, custom VPN/discovery mesh, shared
multi-host database, failover/consensus system, general package manager, custom process supervisor,
second workflow scheduler, global static/dynamic engine modes, or an ECS/vector database merely to
store lessons. Published workflows, adaptation policies, and the necessary schema changes are
explicitly in scope; an unrelated new graph primitive or framework is not.

## Complete the responsibility, not the smallest diff

Follow [implementation practice](../../practices/implementation.md) and
[documentation practice](../../practices/documentation.md). The existing
[workflow](../../workflow.md) remains the verification and completion authority.

A discovered design defect belongs in the current assignment when it prevents the assigned
outcome, creates a bypass, or duplicates the owner being changed. Correct that owner and migrate
all applicable producers, consumers, constructors, storage forms, tests, fixtures, examples, and
documentation now. Do not put a facade around a broken abstraction, preserve two equivalent paths,
weaken validation to reduce the diff, or leave the real integration to 06. Larger replacements are
acceptable when they leave a simpler complete design.

This is not blanket authorization for unrelated cleanup. Trace each expansion to a demonstrated
acceptance requirement. Leave unrelated work on the whiteboard with evidence and a concrete reason.
Do not reopen settled product intent because an older scope-freeze sentence survives in a file.

Each numbered implementation must finish usable behavior through its applicable public boundary.
Types without consumers, a disconnected adapter crate, fixture-only functionality, a stub route,
`todo!()`, permissive fallback, or an example shell script standing in for the production owner do
not satisfy it. A later assignment adds a distinct responsibility; it does not complete a knowingly
half-implemented predecessor.

## Shared acceptance rules

Preserve immutable definitions and prospective changes, append-only workflow truth, exact request
replay/conflict, bounded work, authority below transport handlers, honest uncertainty, external
inference, and one owner for each durable fact. Shared mechanisms do not require duplicate journals.
A local workflow attempt keeps its runtime transaction; a remote host keeps its own acceptance
record. Resource state survives compaction of operation detail.

Every protective rule needs a positive test of permitted work and a negative test of the forbidden
alternative. A system that rejects every adaptation is not an adaptive workflow system. A system
that starts services but cannot recover or remove them is not a managed environment. A published
workflow that blocks its own child work by consuming all worker slots is not complete.

Repository refactoring does not authorize editing live stores or installations. Supported history
must remain meaningful. When an incompatible pre-release store is deliberately unsupported, refuse
it before mutation and provide the existing preservation/inspection path. Never reset data silently,
reinterpret old records, or keep an unreviewed compatibility fallback. Review current fixtures before
choosing versions; 00's compatibility map is the starting decision, not permission to guess.

## Verification and evidence

00 follows the documentation-only verification policy unless its actual changes are executable.
01–06 require the full executable-change gate plus their focused tests. Read the current workflow
for prerequisites and exact commands; at the planning baseline the gate is:

```sh
cargo build -p milkdrift-daemon --bin milkdrift-daemon \
  -p milkdrift-local-process --bin milkdrift-process-test-helper
cargo fmt --all -- --check
cargo check --workspace --all-targets --all-features
cargo test --workspace --all-features --no-fail-fast
cargo clippy --workspace --all-targets --all-features -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --all-features --no-deps
cargo deny check
cargo machete
cargo tree --workspace --duplicates
cargo test --workspace --all-features -- --list
cargo test -p milkdrift-evidence --test repository_contracts --all-features
git diff --check
```

Use the repository-pinned Rust toolchain, not the version from a prior chat. Update owner-specific
checks when an intentional ownership change makes their old paths obsolete; do not delete a
constraint merely because it blocks the implementation. Review public API and dependency changes
under the current repository policy. Hand-review semantic fixtures and demonstrate that targeted
negative tests reject a deliberately weakened rule where practical.

Actual-binary evidence must invoke the product daemon/CLI, not construct a privileged in-process
host instead. Keep raw logs, counters, command outputs, and inventories under ignored
`target/adaptive-hosts/` or CI artifacts. Distinguish deterministic tests, real Linux mechanism
checks, physical two-machine operation, real model behavior, and power-loss testing. None implies
the others. Missing hardware can prevent physical qualification without justifying an incomplete
implementation or a fabricated pass.

## Handoff and closeout

Use [the handoff procedure](handoffs/README.md). Each assignment has one current handoff containing
accepted coverage, important design decisions, affected contracts, checks actually run, and exact
remaining blockers. Do not append diaries. Record partial execution as partial; do not advance a
dependent assignment over an unresolved safety or ownership prerequisite.

06 fixes in-scope integration defects, transfers lasting explanations and evidence into their
canonical owners, and follows the office closeout procedure. Remove this temporary sprint only
when its accepted coverage and remaining decisions have durable homes. Do not delete the plan to
make missing UM790 qualification disappear, and do not create another sprint automatically.

## Planning verification

The original package was prepared against the stated baseline. Assignment 00's handoff records the
actual adoption checkout and documentation checks. [The synthesis](discussed-direction.md) retains
discussion rationale; canonical owners and ADRs 0038–0041 now own accepted direction. No daemon,
model, container, or UM790 execution is established by adopting these documents.
