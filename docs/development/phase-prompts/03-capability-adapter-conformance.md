# Pass 3 — Capability-adapter conformance and lifecycle closure

Use this prompt with `00-shared-execution-contract.md`. Run it after Pass 2.

## Objective

Turn `CapabilityAdapter` from an open interface with individually tested implementations into one explicit, reusable, production-proven contract. Establish the minimum common lifecycle, exact-execution, cancellation, health, authority, and reporting semantics, run the same conformance suite against every production adapter, and remove ambiguous default behavior.

The goal is not to make unlike adapters behave identically. The goal is to encode what every implementation must guarantee and make every legitimate difference explicit.

## 1. Inventory the complete interface and implementations

Inspect the current `CapabilityAdapter`, `AdapterReporter`, capability host registry/execution path, runtime executor port, daemon composition, and all production implementations. The 2026-09-01 audit identified:

- `LocalProcessAdapter`;
- `ModelEndpointAdapter`;
- `RemoteCapabilityAdapter`;
- `WorkflowControlAdapter`.

Search again after previous passes and include any current production implementation. Exclude test fakes from the production implementation count, but use explicitly feature-gated test support to build the suite.

For every method and lifecycle phase, trace:

- who calls it;
- which lock/permit/registration is held;
- whether external code can run;
- what constitutes admission and entry;
- what durable observation is expected;
- how cancellation correlates;
- how drain and shutdown interact with in-flight work;
- what restart can and cannot preserve.

## 2. Define the common contract precisely

At minimum, the shared contract must address:

### Construction and immutable facts

- The adapter generation exposes immutable authority requirements and descriptor facts.
- Requirements are deterministic for that generation and do not read mutable ambient authority.
- Formatting and diagnostics redact secrets.

### Start and registration

- Start occurs before the generation becomes selectable.
- A failed start does not publish a partially live generation.
- Repeated start behavior is explicit: exact idempotent replay, a typed lifecycle conflict, or a documented stateless variant. It may not be accidental default no-op behavior.

### Exact execution

- The adapter receives one exact resolved capability generation, operation, request, invocation identity, and optional durable execution context.
- It performs no fallback to another model, process, peer, profile, generation, or operation.
- It does not mutate runtime/workflow state directly.
- Observations use the supplied invocation identity, valid sequence rules, bounded payloads, and at most one accepted terminal boundary.
- Returning `Ok(())` without accepted terminal evidence and returning an error after durable entry remain truthfully distinguishable by the host/runtime owner.
- Adapter panic is contained and classified without unwinding across an owner boundary.

### Reporting

- Reporter failure is not silently ignored or converted into successful execution.
- Heartbeat behavior extends only the owning lease through the canonical durable path.
- An adapter cannot report another invocation or forge host-owned execution coordinates.

### Cancellation

- Cancellation acknowledgement is bound to the exact invocation and request sequence.
- Acknowledged receipt is not automatically proof of remote/process termination.
- Unknown invocation, duplicate cancellation, completed invocation, and unsupported cancellation have explicit behavior.
- Cancellation does not create a second terminal observation after one is accepted.

### Health

- `health(observed_at_unix_ms)` reports the exact supplied boundary time rather than reading another clock for the observation timestamp.
- Health is bounded, redacted, and truthful about sticky identity/configuration failures.
- Health does not mutate workflow state or implicitly restart the adapter.

### Drain and shutdown

- Drain stops new adapter-owned admission through one canonical host path while allowing already admitted work to reach its defined outcome.
- Shutdown owns and joins/releases adapter resources exactly once or returns a typed incomplete outcome; it does not leak threads, child processes, connections, permits, or registrations.
- Stateless/no-resource adapters must declare that semantic explicitly rather than inheriting accidental no-op lifecycle defaults.
- No host lock is held while calling arbitrary adapter code.

Do not encode mechanism-specific behavior—process groups, HTTP details, peer sessions, or controller policy—into the common interface.

## 3. Replace ambiguous lifecycle defaults

Review the current default implementations of `authority_requirements`, `start`, `begin_drain`, and `shutdown`.

- Keep a default only when it represents one explicit and tested semantic valid for every implementation that inherits it.
- Otherwise make the method required or introduce the smallest closed lifecycle capability/policy that makes stateless behavior deliberate.
- Do not add a broad adapter base class, service locator, or generic lifecycle framework.
- Migrate every implementation atomically and remove local parallel lifecycle interpretations.

The capability host remains the owner of registration visibility, exact-generation selection, concurrency permits, and panic containment. Adapters own only their resources and external mechanism.

## 4. Build one reusable conformance suite

Add a factory-driven or harness-driven suite under explicit capability-host test support. The suite must be reusable without making test internals part of the default product API.

Each production adapter must run the same common assertions using a minimal mechanism-specific fixture. Layer its existing focused tests on top rather than replacing them.

The suite should prove, where applicable:

1. start-before-visible and failed-start cleanup;
2. exact descriptor/generation/operation/request identity;
3. no fallback;
4. supplied durable execution context is preserved;
5. valid ordered observations and terminal uniqueness;
6. reporter failure propagation;
7. cancellation correlation and truthful termination claims;
8. exact supplied health timestamp and redaction;
9. post-drain refusal through the canonical host path;
10. in-flight completion/cancellation during drain;
11. shutdown idempotency/conflict behavior and resource cleanup;
12. panic containment at start, execute, cancellation, health, drain, and shutdown boundaries where arbitrary adapter code can run;
13. no host registry lock held during adapter calls;
14. stable behavior under duplicate calls and bounded concurrency.

Tests must not weaken the contract to the least capable fixture. When an implementation has a legitimate difference, encode and assert the difference through an explicit semantic field or fixture expectation.

## 5. Coordinate with the controller-admission work

The repository’s existing controller-admission prompts may add a request-specific admission-envelope/prepared-entry contract. Do not implement that resource ledger in this pass.

Leave the conformance harness structured so the later controller pass extends the same adapter conformance owner with envelope assertions. It must not create a second unrelated “adapter admission” suite.

If the controller work has already been applied in the current checkout, include its current admission/preparation methods in the same conformance suite and remove duplicate test harnesses.

## 6. Evidence

Run:

- the capability-host test suite;
- every concrete adapter’s complete test suite;
- daemon process/model/control and peer integration tests;
- effect-worker shutdown/backpressure tests;
- the full local gate;
- the relevant `runtime`, `uncertainty`, and `peer` mutation shards if adapter/lifecycle source in those scopes changed.

Inspect for surviving child processes, threads, or listeners after relevant tests. Do not replace coordination defects with sleeps.

## Scope exclusions

Do not implement the controller resource ledger, add a provider family, redesign peer transport, create a plugin framework, alter workflow semantics, expand the CLI, or add UI.

## Acceptance criteria

The pass is complete only when:

- `CapabilityAdapter` has one documented and executable common contract;
- ambiguous lifecycle defaults are removed or made explicit;
- every production adapter passes one reusable conformance suite plus its mechanism-specific tests;
- host locks and ownership boundaries are proven;
- no parallel adapter lifecycle path remains;
- focused, mutation-as-applicable, and full local gates pass.
