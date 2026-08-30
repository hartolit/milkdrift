# Prompt-sequence import schema 1

Prompt-sequence schema 1 is an operator document compiled into a normal immutable blueprint
revision. The accepted encodings are strict JSON and a Markdown envelope. Both produce the same
validated `PromptSequenceDocument`; unknown fields, duplicate JSON keys, unsupported versions,
invalid identities, unsafe paths, and exceeded bounds fail closed.

## Markdown envelope

The first nonempty line must open a fenced JSON header:

````text
```milkdrift-sequence
{"schema_version":1,"sequence":{"stages":[...]}}
```

## Prompt: stage-id
Exact untrusted Markdown prompt bytes.
````

Header stages must omit `prompt`. There must be exactly one nonempty `## Prompt: ID` section for
each declared stage and no extra sections. Section bytes, including their normalized final newline,
become `{"type":"inline_markdown","content":"..."}` before validation and canonical digesting.
JSON documents instead include each `prompt` directly.

## Closed document shape

The envelope contains `schema_version: 1` and one `sequence` with:

| Field | Contract |
| --- | --- |
| `id`, `title`, `workflow_id` | Stable sequence identity, display title, and workflow lineage. |
| `repository` | One `RepositoryWorkspaceProfile`. |
| `stages` | Ordered nonempty array of distinct `StageDefinition` values. |
| `budget` | Sequence-wide revision, review-loop, elapsed, capability-call, and artifact-byte ceilings. |
| `extensions` | At most 32 bounded namespaced data-only values. |

A repository profile contains `id`, opaque `root_ref`, optional `starting_revision`, relative
`allowed_paths`, `allowed_operations`, `dirty_tree`, `isolation`, `cleanup`, required artifact
policy, and bounded opaque credential/remote-profile references. Schema 1 requires `read`, `write`,
and `execute`; `isolated_worktrees` also requires `version_control`. Its artifact policy must
require starting-state, diff, and verification evidence.

Each stage contains:

| Field | Contract |
| --- | --- |
| `id`, `title` | Stable unique stage identity and display title. |
| `prompt` | Inline Markdown or a digest/media-type/size-bound artifact reference. |
| `session` | `fresh` or `explicit_continuation`; generated coding nodes preserve it exactly. |
| `coding` | Preconfigured exact capability/profile requirement. |
| `verification` | Preconfigured verifier, safe check identities, and distinct success/result/log artifact names. |
| `checkpoint` | `verification_artifacts` in schema 1. |
| `failure` | `pause_for_review` or `fail_run`. |
| `reviewer` | Preconfigured reviewer/controller capability requirement. |
| `approval` | `shared_control_path`. |
| `context_policy_ref` | Bounded named policy provenance reference. |
| `budget` | Nonzero coding/verification attempts, timeout, and output-byte ceilings. |
| `outputs` | Bounded distinct coding output names, media types, and required flags. |

A capability/profile requirement contains exact `capability`, namespaced `operation`, optional
`provider_profile`, `execution_trust`, and `maximum_side_effect`. It selects an already configured
generation. No import field can define executable argv, a network destination, a secret value, or
ambient authority. Verification `checks` are safe namespaced data identifiers, not commands or
shell strings. If the repository artifact policy requires a diff, every stage must declare `diff`.

## Bounds

- document: 2 MiB;
- stages: 1–128;
- one inline prompt: 64 KiB;
- aggregate inline prompts: 1 MiB;
- JSON depth: 48; object/array items: 4,096; key bytes: 192;
- verification checks: 1–64;
- declared outputs: at most 32 per stage;
- repository allowed paths: 1–128;
- credential and remote reference lists: at most 32 each;
- extensions: at most 32.

All identities, paths, media types, budgets, artifact references, and nested JSON are subject to
additional typed bounds. Relative allowed paths reject absolute paths, traversal, empty segments,
NUL, and platform prefixes. Canonical JSON is recursively key-sorted and used for a
domain-separated import digest. Prompt and repository-profile digests are domain separated.

## Generated semantics

Each stage generates coding and verification `Task` nodes, a safe `Branch`, and either a failure
`Terminal` or review `Task` plus approval `SignalWait` and failure `Terminal`. A final success
`Terminal` is shared. Coding receives prompt, repository profile, and stage contract as typed direct
inputs. Verification receives repository and verification contracts. The gate's optional data edge
tests only whether the exact success artifact exists.

Coding context selects causal implementation/requirement evidence and applies the declared session
policy. Verification is always fresh and selects bounded implementation/requirement ancestors.
Review is fresh and selects implementation, requirement, verification, failure-evidence, and review
roles while explicitly excluding prior prompts, raw progress, tool traces, verbose command output,
and final-output chronology.

Generated revision metadata under `org.milkdrift/prompt-sequence` records schema, sequence identity,
import digest, repository identity/reference/digest, sequence budget, ordered node mapping, prompt
digests, and checkpoint artifact names.

Validation/import use the existing `validate_blueprint`/`import_blueprint` authority operations and
ordinary immutable revision store. Execution and remediation use the existing run, proposal,
approval, apply, reconciliation, signal, artifact, timeline, and attempt contracts.
