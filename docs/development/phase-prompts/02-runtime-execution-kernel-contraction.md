# Pass 2 — Contract the runtime execution kernel

Refactor the runtime and capability-host execution path into a smaller, traceable set of owners without weakening durability, authority, structured concurrency, context, or uncertainty semantics.

Follow `00-pristine-readiness-contract.md` in full. This pass is about the deterministic execution kernel, not the daemon transport or redb table layout except where their narrow ports must change atomically.

## Primary outcome

A maintainer must be able to trace this path from a small set of named modules:

```text
accepted run command
  -> planned events
  -> projected state
  -> scheduling eligibility
  -> exact resolution and claim
  -> final authority/entry decision
  -> prepared external execution
  -> incremental observations
  -> terminal, retry, cancellation, or uncertainty
  -> recovery and prospective reconciliation
```

Each transition family has one owner. Shared transition mechanics are expressed once. Large files remain only when exhaustive proximity genuinely improves understanding.

## 1. Establish the executable path before editing

Trace all production callers and tests for:

- `RuntimeService` command acceptance and planning;
- scheduler ticks, leases, effect claims, and worker dispatch;
- `TaskExecutor`, prepared-entry/final-entry behavior, and every implementation;
- exact capability resolution, generation permits, entry authority, and cancellation;
- reporter observations and terminal classification;
- retry and uncertain-outcome recovery;
- projection reducers and snapshot/replay;
- structured branch, fork/join, reducer, repeat, wait/signal, and subworkflow transitions;
- reconciliation planning and enactment;
- causal-context discovery/materialization as it intersects dispatch.

Identify repeated parameter groups, event construction, stale-state guards, lookup/validation, and error conversion. Search the entire workspace before creating an abstraction.

## 2. Replace parameter sprawl with owned transition mechanics

The checkout contains many `too_many_arguments` allowances, including repeated runtime methods that pass the same run, projection, event buffer, workspace mutation, sequence, clock, node, execution, and scope facts.

Where those arguments represent one atomic transition, introduce the smallest private transition owner that:

- borrows only the state and outputs for one commit-planning operation;
- exposes named invariant-preserving methods rather than public fields;
- centralizes sequence/event/workspace/result accumulation and checked bounds;
- cannot outlive the operation or become mutable ambient service state;
- does not hide database I/O, external execution, or authority evaluation;
- reduces call-site knowledge and repeated guard logic.

Do not replace argument lists with a passive “context” bag, giant tuple, macro that hides control flow, or public framework. Keep distinct facts separate when they do not share one lifecycle.

Remove obsolete helpers and every allowance made unnecessary by the adopted owner. Remaining allowances require a local rationale naming why the arguments are independently meaningful.

## 3. Contract command planning and event construction

Review `crates/runtime/src/engine/command_planning.rs`, engine support, command types, and all command-family tests.

Split by real closed command families and common planning mechanics, leaving a small exhaustive dispatcher. Consolidate repeated:

- optimistic state checks;
- command/result identity and replay handling;
- actor/authority basis validation;
- bounded event batch construction;
- sequence and event identity allocation;
- no-effect rejection and terminalization;
- reconciliation or controller guard plumbing.

Do not create one trait per command. Prefer private functions/modules and one closed match. Tests must continue to prove every command variant, denial, stale guard, replay, and atomicity independently.

## 4. Contract effect entry, reporting, completion, retry, and recovery

Review `engine/effects.rs`, `engine/completion.rs`, executor/reporting contracts, capability-host execution, worker ownership, and structured runtime tests.

Leave one canonical external-entry path with these properties:

- exact resolution/generation/request/attempt/lease/authority facts are revalidated once at the owning boundary;
- a denied or stale entry cannot call adapter code;
- the durable entry fact precedes external work;
- the adapter reports observations but does not decide workflow state;
- terminal classification and uncertainty are owned once;
- a worker transport/clock/panic error cannot overwrite a more specific durable adapter observation;
- post-entry absence of evidence remains uncertain;
- retry requires the current idempotency and side-effect contract and never replaces an uncertain non-idempotent attempt;
- cancellation acknowledgement does not fabricate remote termination;
- permits, workers, and reporter lifetimes release exactly once.

Remove duplicated final-entry, terminal, uncertainty, or recovery reason construction in capability-host, peer, deterministic executors, and runtime test drivers. Layer-specific evidence may be retained, but one runtime/persistence semantic owner must choose the durable state.

## 5. Organize projections by invariant, not event-file size

Review large projection owners such as `projection/node.rs`, structured/repeat reducers, projection tests, and replay/query integration.

Keep one exhaustive top-level event dispatcher, but move separable event families into private reducers when they own distinct invariants. Consolidate repeated mechanics for:

- execution/attempt lookup and exact identity checking;
- lifecycle transitions;
- lease and cancellation closure;
- terminal summaries and compaction;
- structured scope ownership;
- retry/uncertainty retention;
- reconciliation plans/actions.

Do not split every enum variant into a forwarding function. Do not duplicate validation between event construction and projection; construction owns accepted-event validity, while projection independently refuses impossible history.

## 6. Contract reconciliation and structured execution

Review `reconciliation.rs` and structured engine modules for repeated classification, lookup, and action construction.

Ensure:

- one matrix classifies completed, started, entered, uncertain, pending, added, removed, changed, and incompatible work;
- completed and entered history stays attached to its original revision;
- only uncommitted future work can be redirected;
- structured runtime-owned work is not mistaken for ordinary pending tasks;
- reconciliation plan size and enactment remain bounded and atomic;
- action creation and projection interpretation cannot drift.

Use a closed data-driven table only where it makes the matrix more explicit and independently testable. Do not create a generic graph-diff framework.

## 7. Preserve causal-context ownership while reducing mechanics

Do not redesign the context model. Review runtime context dispatch integration only for duplicated source lookup, exact attempt binding, selected-artifact verification, and error conversion.

Context selection remains causal, bounded, provenance-bearing, branch-isolated, and frozen before dispatch. A retry reuses the exact prior selection unless a distinct policy-driven attempt explicitly chooses otherwise.

If context modules still contain repeated generic paging/folding mechanics, consolidate them privately without merging policy selection, historical reconstruction, and provider materialization into one owner.

## 8. Cohesion targets for the runtime scope

Perform an explicit cohesion review of every runtime production file above 1,000 lines and every function above approximately 250 lines. At minimum, materially contract or reorganize:

- `engine/command_planning.rs`;
- `engine/effects.rs`;
- `engine/completion.rs`;
- `engine/support.rs`;
- `projection/node.rs`;
- `query.rs`;
- `reconciliation.rs`;
- `engine/structured/repeat.rs`.

A successful result has small façade/dispatcher modules and named child owners. Merely moving unchanged blocks into arbitrary files is not completion.

Remove the corresponding cohesion exceptions when the files fall below the review threshold. Every remaining exception needs a specific local explanation of why all contained code changes for one invariant.

## Required proof

Add or strengthen independent tests for:

- every final-entry refusal proving zero adapter calls;
- adapter-specific post-entry failure versus worker/clock/panic recovery precedence;
- crash immediately before and after durable entry;
- terminal observation followed by worker failure cannot become uncertainty;
- uncertainty followed by late terminal preserves both history and one current resolution;
- cancellation and retry under each side-effect/idempotency class;
- command replay and concurrent stale transitions;
- replay equals incremental projection after every refactored event family;
- structured/reconciliation behavior across restart and compaction;
- fixed worker and permit ownership on panic, drop, shutdown, and conflict.

Run the full gate plus the runtime, uncertainty, context, controller, and peer mutation shards when touched. Run the effect-worker shutdown proof and relevant release-mode structured-runtime stress lanes.

## Completion threshold

This pass is complete only when:

- one external-entry/observation/terminal/recovery path remains;
- runtime public concepts and lint allowances are lower;
- the named large files are genuinely organized by responsibility;
- duplicated transition guards and event construction are removed;
- no schema changes were made solely for code movement;
- production runtime code is net smaller or demonstrably requires less knowledge to modify safely;
- all current runtime semantics and difficult truth are preserved.
