# Model Lifecycle and Cancellation Guarantees

`domain-contracts::ModelLifecycle` provides a deterministic policy state machine:

```text
Active
  -> Draining(deadline)
  -> Cancelling(DrainTimeout)
  -> Unloading
  -> Absent
```

The drain deadline is mandatory and non-zero. Expiration always returns
`LifecycleAction::CancelActive` with `CancellationReason::DrainTimeout`.

## Safe reclamation boundary

The state machine cannot safely destroy model resources while backend code still
owns a mutable borrow of the loaded model or sequence. Rust threads cannot be
forcibly terminated without violating resource and lock invariants. Therefore,
engine-level deterministic reclamation must use at least one of these execution
contracts:

1. backend prefill and decode calls have documented bounded duration and observe
   cancellation at safe boundaries;
2. long prefill work is split into bounded chunks controlled by the runtime;
3. an untrusted or potentially hanging backend runs in a separate process whose
   termination delegates final memory reclamation to the operating system.

A cooperative in-process backend may delay physical reclamation until its current
bounded step returns. The runtime must never drop a model concurrently with an
active backend call. For CUDA, successful sequence destruction and model unload
also require explicit synchronization of the verified selected device; tensor drop
alone is not a successful cleanup contract.

## Terminal cleanup and degraded state

All generation terminal paths—EOS, token limit, stop match, cancellation, backend
failure, sampling failure, drain escalation, and shutdown—use the same explicit
sequence-destruction transition. A destruction failure moves the sequence out of
the normal active-request registry into runtime-owned quarantine. Its identity,
model sequence slot, and memory footprint remain accounted, and the affected model
rejects new requests until cleanup succeeds.

Maintenance retries at most one non-exhausted quarantined cleanup operation per
worker loop. Selection rotates across pending sequences, failed preparations, and
complete models, then rotates within each class, so ordered identities cannot
permanently starve another owner. The initial failed cleanup counts as attempt one;
the default policy permits three total attempts and may be overridden through
`CleanupRetryPolicy`. Each retry records inspectable attempt state in the runtime
snapshot. After the total-attempt limit is reached, automatic maintenance skips the
resource while retaining its ownership and capacity. A successful retry removes
ownership and accounting exactly once and records `RetainedOwnership::Released`; a
release on the final permitted attempt is released, not exhausted. Model unload
preparation follows the same rule: success is the only ordinary permission to
release the owner.

A conforming verified model retains `RetainedOwnership::Exact` for its named final
phase. A failed materialization owner initially retains the accepted loading peak
exactly only while its reported plan remains identical to the admitted plan. E0
checks that report before and after every cleanup attempt; substitution or mutation
reclassifies the owner as `RetainedOwnership::Unverified`, removes the formerly
exact reservation once, preserves monotonic conservative evidence, and blocks new
admission until release.

A retained sequence follows the same rule for its complete accepted
`SequencePlan`. E0 verifies identity, capacity, and plan after creation and after
successful execution, then reads the plan before and after destruction/retry. Any
contradiction makes failed-cleanup ownership unverified even when an individual
footprint value still matches. Successful destruction releases the admitted
sequence total once; caller-owned terminal workspace remains separately accounted
until its output storage is released.

A complete model that contradicts its accepted handle, descriptor, device, scalar,
or footprint is likewise `RetainedOwnership::Unverified` if unload fails. E0
preserves the accepted peak, reported footprint, and checked conservative component
evidence, but does not call any of them an exact upper bound. Unverified owners are
exposed separately from exact `reserved_footprint`; existing healthy admitted work
remains runnable. Successful cleanup transitions the retry record to `Released`
while removing the owner exactly once.

Generation output orders `Terminal`, optional `CleanupPending`, optional
`CleanupExhausted`, and `Released` records. A terminal generation task also retains
its admitted host-workspace accounting until `Released` is published and the task
storage is dropped. Consequently, completion of token generation is not presented
as proof that backend resources or request-owned host storage have already been
released.

Shutdown consumes only the finite remaining cleanup budget in deterministic fair
order. If any resource remains, it returns `TerminalCleanupRetention` with the
first exhausted owner and a bounded summary of failed preparations, verified
models, incompatible models, retained sequences, and unverified conservative
evidence. The hosted worker then applies `RetainUntilProcessExit`: it deliberately
retains the complete runtime allocation, exits, and can be joined, while process
termination remains the reclamation boundary. This outcome is terminal and cannot
be retried because no observable runtime endpoint remains. Endpoint disconnection
applies the same fail-closed policy: unresolved native ownership is retained rather
than being passed to an undocumented implicit `Drop` cleanup path. A missing worker
handle or disconnected endpoint is never itself evidence that cleanup succeeded.

E1 retains a structured terminal shutdown failure independently from its worker
join handle. A join timeout is retryable and can later complete cleanly when E0
shutdown succeeded. A terminal E0 cleanup failure remains an error after the
worker has exited and after its handle is joined; a repeated application shutdown
must not convert it into success.

E1 represents application cleanup through one private coordinator rather than
parallel incompatible-unload and retained-inspection trackers. Its checked action
contains the command identity and ticket together, retains lower cleanup counts
separately from bounded E1 submission counts, and rejects mismatched tickets or
resources. Public detailed evidence remains in `ApplicationState`; cleanup events
carry only resource identity and the observed disposition transition. Clearing
that state requires a correlated unload/shutdown success or an explicit released
lower cleanup record, and happens once with the coordinator removal.

At E1 the model lifecycle and generation lifecycle are deliberately orthogonal
private enums. This permits a correlated generation terminal/output owner to remain
observable while model draining completes, a worker disconnect quarantines model
ownership, or shutdown begins. It does not permit new generation admission without
the canonical loaded phase. Phase and request transitions are checked, stale events
leave state unchanged, and frontend activity/capability values are exhaustive
projections rather than independently synchronized fields.
