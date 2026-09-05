# Milkdrift pristine-readiness execution prompts

This package is for the current post-CLI Milkdrift checkout. It is not a request for another architectural rebirth and it is not a feature roadmap. Its purpose is to make the implemented system proportionate, traceable, routinely usable through public headless interfaces, and stable enough for the owner to begin studying the final architecture rather than accumulated implementation history.

“Pristine” is defined operationally by `00-pristine-readiness-contract.md`. It does not mean that no future improvement is possible. It means that the current product slice has one coherent implementation, accidental complexity has been removed, the normal operator path works without privileged test machinery, and remaining limitations are explicit rather than hidden by more abstraction.

## How to execute

Record the starting commit before pass 1:

```sh
git rev-parse HEAD
git rev-parse HEAD^{tree}
git status --short
```

Use a fresh agent context for every numbered pass. Give that agent:

1. the repository resulting from the previous pass;
2. `00-pristine-readiness-contract.md`;
3. exactly one numbered prompt;
4. no persuasive completion summary from an earlier agent.

Review and apply one result before starting the next pass. Preserve each pass as one reviewable commit or patch. Do not ask agents to commit this prompt package or copy it into repository documentation.

## Execution order

1. `01-contract-ownership-and-public-surface.md`
2. `02-runtime-execution-kernel-contraction.md`
3. `03-persistence-redb-and-daemon-contraction.md`
4. `04-test-and-evidence-contraction.md`
5. `05-headless-operator-readiness.md`
6. `06-documentation-and-comprehension.md`
7. `07-independent-readiness-closure.md`

The order is intentional:

- Contract ownership is corrected before downstream code is reorganized around it.
- Runtime responsibilities are contracted before storage and daemon composition are simplified.
- Tests and evidence are reduced only after production ownership is stable enough to test through canonical paths.
- The CLI is finished after the semantic and host boundaries it consumes have stopped moving.
- Documentation is compressed after code ownership is final.
- A different agent then audits, repairs, qualifies, and either declares readiness or leaves one exact blocker.

## Static baseline from the supplied checkout

These figures are diagnostic starting points, not targets that may be gamed:

- 388 Rust files and approximately 178,245 Rust lines;
- approximately 119,000 production Rust lines after excluding integration-test directories and embedded `#[cfg(test)]` tails;
- approximately 50,000 Rust lines under integration-test directories;
- 64 Markdown files and approximately 8,009 Markdown lines;
- 51 Rust files above 1,000 lines, including 24 production files listed in the repository’s cohesion-exception table;
- 76 `too_many_arguments` allowances;
- approximately 62 private `*Wire` structures, 92 manual `Deserialize` implementations, and 410 `deny_unknown_fields` annotations;
- the CLI directly depends on ten Milkdrift packages;
- the committed `docs/development/phase-prompts/` directory contains about 1,341 lines despite its own instruction not to commit the prompt package.

The agents must reproduce current measurements from the real Git checkout before relying on these approximate values.

## Readiness outcome

After pass 7, the intended state is:

```text
stable semantic owners
    + contracted public and dependency surfaces
    + one traceable command/effect/recovery path
    + tests that observe that path independently
    + a routine daemon/CLI workflow from a fresh directory
    + concise non-duplicated documentation
    + an explicit architecture freeze
```

External credentials or hosted runners may remain unavailable. That prevents external qualification claims, not in-repository cleanup. Agents must never weaken a gate, fabricate evidence, or enable continuous controllers merely to obtain a “ready” label.
