# Milkdrift documentation

This index routes readers to each documentation owner. It does not duplicate product, status,
architecture, or protocol facts.

## Understand the product

- [Product vision](product/vision.md) owns enduring intent and non-negotiable semantics.
- [Architecture constitution](architecture.md) owns boundaries, terminology, dependency direction,
  and compatibility rules.
- [Current status](product/status.md) owns implemented behavior, limitations, and the latest evidence
  snapshot.
- [Roadmap](product/roadmap.md) owns ordered unfinished product work.
- [Architecture decision records](decisions/README.md) explain durable decisions and tradeoffs.

## Develop and verify

- [Development workflow](development/workflow.md) contains build, test, lint, fixture, and focused
  evidence commands.
- [Engineering rules](development/engineering-rules.md) define standing implementation-quality
  policy.
- [Verification and operational evidence](development/verification-evidence.md) defines repeatable
  evidence lanes and their limitations.

## Use Milkdrift

- [Headless prompt-sequence dogfood](guides/headless-dogfood.md)
- [Local process operator guide](guides/local-process.md)
- [Local model endpoint guide](guides/local-model-endpoint.md)
- [External interoperability evidence](guides/external-evidence.md)

## Operate Milkdrift

- [Daemon operation and durable state](operations/daemon.md)
- [Control and execution authority](operations/authority.md)
- [Peer connectivity](operations/peers.md)

## Look up a contract

- [Local control API](reference/control-api.md)
- [Peer protocol](reference/peer-protocol.md)
- [Prompt-sequence schema](reference/prompt-sequence-v2.md)
- [Public API policy](reference/public-api-policy.md)

Executable schema constants, readers, and golden fixtures remain the primary evidence for current
contract versions.
