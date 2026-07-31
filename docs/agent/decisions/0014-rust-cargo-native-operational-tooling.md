# ADR-0014: Keep project-owned operational tooling Rust/Cargo-native

- **Status:** Accepted
- **Date:** 2026-07-31

## Context

The removed GGUF path introduced a project-owned Python fixture generator, and the external Candle validation runbook required the Python-distributed Hugging Face CLI plus external digest tooling. Those requirements were not part of the Rust application architecture, were not exercised by Cargo's selected graph, and created an additional language environment for maintained build, test, validation, and release work.

The repository already owns artifact resolution in `hf-hub-adapter` and repository maintenance in the Rust Cargo package at `tools/xtask`. Operational policy should match those ownership boundaries and prevent an incidental fixture or command from silently reintroducing Python or a removed native engine.

## Decision

Project-owned operational tooling is implemented in Rust and invoked through Cargo. Custom repository policy and composite verification live in `tools/xtask`; ordinary one-step operations use Cargo directly, as specified by [ADR-0016](0016-virtual-workspace-focused-xtask.md). Maintained project workflows must not require a Python runtime, Python package manager, Python-distributed CLI, notebook environment, or embedded Python runtime.

The enforced project boundary includes:

- tracked project source, generators, scripts, notebooks, and environment definitions;
- build scripts, tests, benchmarks, examples, validation and release procedures, CI workflows, and packaging commands;
- maintained root, crate, project-reference, and active execution documentation;
- Cargo manifests and the packages selected by the workspace dependency graph;
- subprocesses launched by project-owned Rust or build configuration.

Within that boundary, the repository does not contain or invoke `python`, `python3`, `pip`, `pipx`, `uv`, `conda`, `poetry`, `pytest`, `maturin`, the Python Hugging Face `hf` CLI, PyO3-style bindings, or another embedded Python runtime. Exact removed llama.cpp packages are also rejected by dependency policy so the deleted native path cannot return indirectly.

Upstream Rust dependencies are not rejected merely because their source repositories contain optional Python examples, bindings, or development tooling that Cargo neither selects nor invokes. The policy governs this repository's selected build, runtime, and maintained operational path.

The Rust-owned hygiene check in `tools/xtask` enforces the boundary across every Git-tracked maintained surface, direct Cargo declarations, and the locked selected dependency graph. Maintained surfaces are scanned without filename-, directory-, historical-purpose-, or document-status-based bypasses; superseded ADRs and execution history do not receive blanket exclusions. The scanner distinguishes negative explanatory policy prose from an operational invocation by content semantics rather than by where the text appears.

Any future exception must identify the exact artifact, invocation, declaration, or selected package; state a narrow rationale and scope; receive explicit review; and include tests proving both the intended allowance and rejection of nearby forbidden cases. Wildcard filename, directory, document-status, or history exemptions are not acceptable.

Where supported cleanly, `cargo-deny` adds exact package bans as defense in depth. Bans remain narrow and evidence-based rather than rejecting general Rust build dependencies that have valid independent owners.

External Hugging Face smoke validation runs through Cargo and reuses E1's production `hf-hub-adapter` resolution and Candle lifecycle. E0 remains network-free. Authentication and cache overrides may use the existing environment-based Rust adapter behavior, but no shell or Python downloader/converter wrapper is added.

## Rejected alternatives

- **Keep Python only for fixtures or validation:** a maintained exception still creates an unpinned secondary toolchain and permits operational dependency drift.
- **Port the deleted GGUF converter to Rust:** preserving a large generator for a removed product path would retain complexity without product value.
- **Document manual `hf download` and digest commands:** that duplicates production artifact resolution and requires a Python-distributed CLI outside Cargo.
- **Ban every Rust crate whose upstream repository mentions Python:** that is neither enforceable nor relevant to the selected Cargo path.
- **Rely only on repository searches or only on `cargo-deny`:** text searches do not understand selected dependencies, while dependency policy cannot detect tracked scripts or maintained commands; both repository and graph evidence are required.
- **Add a shell wrapper around Rust validation:** Cargo already provides the stable invocation boundary, and another script would add no ownership value.

## Consequences

- The repository has one maintained implementation language for operational tooling: Rust through Cargo; custom policy and composite verification are owned by `tools/xtask`.
- Fixture-based tests remain download-free; external model validation remains explicit, opt-in, network-dependent, and production-path based.
- Historical documents may explain former Python and llama.cpp use without becoming executable guidance.
- CI and contributor setup no longer install Python or llama.cpp-specific tooling for project commands.
- If a future dependency embeds Python or a new tracked Python artifact appears, repository validation fails with an actionable policy violation.
- Native system libraries still required by selected Rust dependencies remain documented honestly; Rust/Cargo-native does not mean every transitive implementation is pure Rust.

## Review trigger

A proposal to add any project-owned non-Rust operational requirement must be reviewed through a new ADR that identifies the owner, selected build/runtime path, reproducibility and security implications, CI/toolchain cost, and why a Rust/Cargo implementation is insufficient. A proposed hygiene exception must additionally be exact, reviewed, and covered by positive and negative policy tests; path or document status cannot supply the exception. Upstream optional files that remain unselected do not trigger review.
