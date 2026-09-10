# milkdrift-prompt-sequence

Use a prompt sequence to turn ordered implementation prompts into a workflow with explicit
verification and failure handling. This package reads a bounded import and compiles it into an
ordinary [blueprint revision](../blueprint/README.md). The daemon submits that revision through
the existing control path; the sequence has no separate executor.

The [maintained Markdown example](../../examples/headless-dogfood-sequence.md) supplies a complete
import, and the [crate example](src/lib.rs) parses and compiles it without starting work. For an
actual run, the named coding, verification, and reviewer capabilities must already be configured.
Schema v2 accepts `process.execute` with `trusted_host_process` and no provider-profile selector.
The [headless workflow guide](../../docs/guides/headless-dogfood.md) owns operator setup.

## Follow one stage

Each stage supplies a prompt, capability requirements, and a verification contract. Compilation
produces this control flow:

```text
coding --> verification --> success artifact present? -- yes --> next stage / success
                                      |
                                      no
                                      |
                         failure terminal, or review --> approval wait
```

The gate tests for the configured success artifact after verification completes; it does not
interpret prose such as “tests passed.” A verifier should publish result evidence for a completed
check and publish the success artifact only on success. A failed verifier invocation does not
itself become this artifact-absence branch.

With `PauseForReview`, the initial approval wait leads to a failure terminal. To continue with a
repair, apply an approved prospective remediation revision before delivering the signal. A signal
alone does not invent repair work or turn failed verification into success.

The repository profile, stage contract, and verification contract become task inputs. Configured
processes interpret those declarations; importing them does not create worktrees, inspect Git,
configure credentials, or enforce a filesystem allowlist. `context_policy_ref` is descriptive
data; the compiler builds the task's context policy itself. It requests relevant ancestor evidence
and records output roles for later tasks. A session declaration alone does not arrange external
continuation. Generated stages use `process.execute`, so the model request's session-agreement
check does not interpret their stage contract; see the [blueprint policy API](../blueprint/src/context.rs).

## Read imports and associate results

Use `PromptSequenceDocument::from_bytes` for JSON or Markdown and `from_json` for JSON alone.
Markdown starts with a fenced `milkdrift-sequence` JSON header whose stages omit `prompt`, followed
by one `## Prompt: STAGE` section per stage. The reader normalizes line endings and ends each prompt
with one newline. Duplicate, missing, extra, or empty sections are refused. JSON inline prompts
retain their supplied text; artifact prompts need exact digest, size, and media facts to compile.

These readers validate the complete import. Public document fields permit Rust assembly, so
direct Serde decoding or a struct literal is not a substitute for the production reader before
compilation. `CompiledPromptSequence` exposes the revision, import/profile digests, and stage
summaries. The revision reason names the validated import schema version; changing that reason
changes new revision identity without relabeling stored imports. Use `stage_node_ids` on a saved
revision to associate executions with a stage, including
inserted remediation nodes; do not infer ownership from a node-name prefix.

## Propose a repair

`build_remediation_proposal` takes the original import, exact current revision bytes, and observed
run/sequence facts. It checks import provenance and the configured generation bound, then builds
a normal proposal requiring approval. It validates the prospective graph without changing the
base revision. Control and runtime subsequently check live state, authority, and reconciliation.

Keep the original import unchanged for this check. A verifier replacement belongs in
`RemediationProposalSpec::verification_override`; changing the original import would contradict
the provenance of its base. The generation limit bounds the caller's supplied generation, and
does not install a controller loop or execute proposals automatically.

The [sequence tests](tests/sequence.rs) cover import refusals, both failure routes, and prospective
remediation. This package has no feature flags. Follow the
[verification policy](../../docs/development/workflow.md#choose-verification-for-the-change) for
changes; pure import/compilation checks need no running external agent.
