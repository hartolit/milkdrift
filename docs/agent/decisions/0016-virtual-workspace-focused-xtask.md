# ADR-0016: Use a virtual workspace and a focused `xtask`

- **Status:** Accepted
- **Date:** 2026-08-01

## Context

The repository root previously served both as the Cargo workspace and as an `llm-app` maintenance package. That package was not the product application; it hosted repository checks and wrappers around ordinary Cargo operations. The package identity obscured the actual applications and made a custom command layer appear necessary even when Cargo already expressed an operation directly.

The repository still needs project-specific architecture and hygiene policy plus one reproducible composite quality gate. Those operations add semantics that a single built-in Cargo command does not provide.

## Decision

Keep the root `Cargo.toml` as a virtual workspace with no root package. Put project-owned workspace automation in the explicit `tools/xtask` member and expose it through the Cargo alias `cargo xtask`.

Aside from help, `xtask` has only these command roles:

- `cargo xtask architecture` runs custom workspace-layout and dependency-policy checks;
- `cargo xtask hygiene` runs custom maintained-surface and selected-graph policy checks;
- `cargo xtask verify` runs those custom policies and the canonical fail-fast composite quality gate.

`cargo xtask verify` is the sole composite verification command. It may orchestrate multiple Cargo operations because composition, ordering, shared policy, and fail-fast behavior are the value it adds.

One-step operations use Cargo directly. Formatting, checking, testing, linting, documentation, benchmarks, package selection, examples, metadata, and similar single Cargo actions must not gain pass-through `xtask` subcommands or a second forwarding interface. A new custom command is justified only when it implements repository-specific policy or composes multiple steps with behavior that direct Cargo invocation cannot express clearly.

This command boundary specializes the Rust/Cargo-native tooling policy in [ADR-0014](0014-rust-cargo-native-operational-tooling.md); it does not create another scripting layer beside Cargo.

## Rejected alternatives

- **Keep a root maintenance package named `llm-app`:** it makes tooling look like the product binary and prevents the root manifest from expressing a conventional virtual workspace.
- **Wrap every common Cargo operation in `xtask`:** unchanged argument forwarding duplicates Cargo, increases documentation surface, and creates competing command names.
- **Provide multiple composite verification entry points:** equivalent gates drift in ordering and policy and make the acceptance boundary ambiguous.
- **Use ad hoc shell wrappers for the same commands:** they add another maintained orchestration surface without adding ownership or portability value.

## Consequences

- Workspace membership and package identity are explicit; the root itself is not a package.
- `tools/xtask` is the single home for custom repository policy and composite verification.
- Contributors use `cargo xtask verify` for the complete gate and direct `cargo` commands for focused one-step work.
- Removing the former root runner is a command migration, not a product-application rename.
- Future automation must demonstrate added policy or composition semantics rather than convenience forwarding.

## Review trigger

Review when a required project workflow cannot be represented safely as direct Cargo or one of the existing policy/composite roles, when `xtask` gains enough independent responsibilities to require a narrower ownership split, or when the repository root must become a real publishable or executable package.
