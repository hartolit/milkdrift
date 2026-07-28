# ADR-0009: Adopt domain, platform, adapter, runtime, and app roots

- **Status:** Accepted
- **Date:** 2026-07-29

## Context

ADR-0005 deliberately kept the original `features` and `engines` roots stable
while the first generation slice was still proving the product architecture. That
slice is complete, `corrective-workflow` has acquired an independent runtime
boundary, and the old physical names now obscure distinctions that the dependency
graph already makes.

In particular, `features` can be confused with Cargo features, while `engines`
groups E0 inference, E1 application coordination, and independently stateful
capabilities under a name that no longer describes their shared property. The
existing `host-runtime` crate is different again: it wraps process-host threading,
bounded channels, monotonic time, and pull-oriented output storage without owning
application, workflow, or inference state.

## Decision

Use these physical roots:

```text
crates/domain/      portable contracts and algorithms
crates/platform/    process-host execution primitives
crates/adapters/    external/vendor/model/storage integrations
crates/runtime/     stateful resource ownership and orchestration
crates/apps/        process and presentation boundaries
```

Move `domain-contracts`, tokenization, context planning, sampling, and task graph
from `features` to `domain`. Move E0, capability runtimes, and E1 from `engines`
to `runtime`.

Move `host-runtime` from `adapters` to `platform`, but keep its package name. The
name describes the narrow host execution substrate it currently implements;
renaming it to a broad term such as `native` would claim a general native-platform
abstraction that does not exist.

`platform` is a physical distinction, not a new logical dependency tier today.
Platform support and adapters both remain lower infrastructure beneath runtimes.
The architecture validator keeps runtime and platform roles explicit rather than
granting them from directory placement. Domain, adapter, and application roots keep
their category rules, while new runtime or platform crates require an intentional
classifier change.

Keep adapters flat for now. Introduce subgroups such as inference, Hugging Face,
storage, or network only when their breadth or dependency rules make those groups
useful; directory symmetry alone is not sufficient.

## Rejected alternatives

- **Keep `features`/`engines` indefinitely:** the names now reduce discovery and conflict with the architecture they describe.
- **Rename `host-runtime` to `native`:** too broad for a crate that currently owns only host threading, timing, channels, and bounded output infrastructure.
- **Put host primitives back in `runtime`:** process-host mechanics are lower infrastructure, not application or inference orchestration.
- **Create adapter subtrees immediately:** the current adapter set is still small enough that deeper grouping would mostly add path churn.
- **Treat folder names as architecture:** package roles and reviewed dependency edges remain enforced independently from physical organization.

## Consequences

- The repository layout communicates domain, host infrastructure, external integrations, orchestration, and presentation separately.
- Legacy `features` and `engines` roots are no longer accepted by the architecture classifier after the migration.
- Adding runtime or platform crates requires explicit classifier review rather than inheriting authority from directory placement.
- Adapter subfolders remain a future migration and are not pre-authorized by the validator.
- Package names and public Rust paths remain unchanged by this physical move, including `host-runtime` / `host_runtime`.

## Review trigger

Review when the platform category gains a second coherent implementation domain,
when adapter breadth materially harms discovery, or when the logical dependency
matrix needs a distinct platform tier rather than the current shared lower
infrastructure layer.
