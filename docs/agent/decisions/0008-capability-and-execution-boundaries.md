# ADR-0008: Separate application coordination, capabilities, and model execution

- **Status:** Accepted
- **Date:** 2026-07-28

## Context

`application-runtime` has proved useful as the frontend-neutral E1 façade, but it
currently sits where unrelated domains and concrete infrastructure can easily
accumulate. The corrective workflow already has its own artifact state, retry
lifecycle, events, and service ports and can exist independently from the hosted
model lifecycle.

The longer-term product also needs model execution that is not local inference. A
request may run through E0 on the current machine, on another node over an
existing network, on rented GPU infrastructure, or through a hosted model
service. Those targets do not share E0's model-loader, tensor, sequence, cleanup,
or per-token scheduler semantics.

## Decision

Keep `application-runtime` as the single frontend-neutral application
coordinator. E1 owns application semantics: frontend-shared state, conversation
and request policy, user cancellation intent, target/capability selection, and
coordination of the services required to fulfill those operations.

Introduce an **engine capability** role for independently stateful reusable
orchestration below E1. A capability engine may depend on features, adapters, and
E0 when justified, but never on E1 or an application frontend. E1 may coordinate
capability engines without absorbing their implementation.

The existing corrective workflow becomes the first capability engine in
`crates/runtime/corrective-workflow`. This is an extraction of ownership, not a
redesign of its six-stage behavior.

E0 `inference-runtime` remains the owner of local/native model resources,
sequences, token-step scheduling, sampling, cancellation boundaries, cleanup, and
unload. Hosted model services and peer nodes are **not E0 backends**.

When a second execution kind is implemented, define a coarse model-execution
boundary above E0. It should describe target identity/capabilities, complete
request admission, cancellation intent, bounded streaming output, usage, and
terminal state. The local implementation delegates complete work to E0. Provider
and peer implementations translate the same application intent through their
transports without exposing vendor or wire types through E1.

Conversation history and workflow artifacts must not contain Candle source
types, native model handles as semantic identity, provider request DTOs, or
transport connections. Target-specific context limits, prompt/message formats,
token accounting, sampling controls, tools, cancellation guarantees, privacy
boundaries, and usage reporting remain explicit capabilities. Uniformity must not
be manufactured by pretending every target behaves the same.

Concrete Candle/Hugging Face/redb composition may remain in E1 while it is the
only production composition. A local composition split should occur when GGUF, a
second deployment, or remote execution reveals the actual seam. Do not introduce
a speculative generic service graph first.

## Rejected alternatives

- **Treat hosted or peer models as E0 backends:** their remote request semantics do not satisfy E0's native resource-ownership contract.
- **Put provider clients in conversation code:** vendor transport would become application semantics.
- **Keep every stateful capability inside E1:** unrelated lifecycles would turn the façade into the entire AI system.
- **Genericize `ApplicationRuntime` over every service:** composition details would leak through the public application API.
- **Create empty engine crates for future ideas:** a new engine requires proven state, lifecycle, replacement, testing, or reuse pressure.

## Consequences

- Slint, TUI, headless, and later transported frontends can share one application model.
- Local inference keeps its strict ownership and hot-path design.
- Corrective workflows can evolve without expanding E1's public surface.
- External execution can be added without distorting E0.
- Sending context outside the user's machines remains an explicit choice with target-specific capability, credential, privacy, and failure policy.
- The architecture validator classifies runtime and platform roles explicitly rather than by folder fallback, forbids upward dependencies, and requires exact review for runtime-to-infrastructure or runtime-to-runtime production composition edges.

## Review trigger

Review this decision when the first hosted-provider or peer execution target is
implemented, when capability engines appear to require direct dependencies on one
another, or when concrete local composition still dominates E1 after GGUF parity.
