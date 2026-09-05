# Local process operator guide

`milkdrift-local-process` profile schema v2 configures one byte-pinned executable generation. The
adapter is a `trusted_host_process`: it runs with the daemon account's OS authority while
Milkdrift mediates direct argv, a rebuilt environment, selected inputs, declared outputs, and
configured paths. It is not a sandbox.

## Profile schema 2

For ordinary setup use the [operator process example](../../examples/operator/README.md#one-byte-pinned-local-process).
The complete [coding-agent profile template](../../examples/external-evidence/coding-agent-profile.example.json)
is validated by the production reader. Copy it outside the repository and supply the actual
executable digest, size, paths, arguments, and declared resources before registration.
`revision` and `descriptor_revision` must be the same nonzero value. `content_digest` is `b3_`
followed by the lowercase 64-hex BLAKE3 digest, and `size_bytes` is the exact nonzero file size.
The executable is streamed through a bounded 64-KiB buffer and may be at most 1 GiB. The optional
package revision is implementation provenance and contributes to implementation identity. The
bounded documentation reference is provenance metadata: changing only it changes the full profile
and descriptor facts but not the executable identity or execution-policy digest.

The descriptor freezes:

- safe digests of the configured and canonical executable paths, exact content digest and size,
  optional package revision, and regular-file/platform observations;
- the complete profile digest;
- an execution-policy digest covering argv/substitution, working-directory/root, input,
  environment and secret-reference names, stdin/capture/output, limits, restart, operation,
  side-effect/idempotency/cancellation, admission, trust, and platform facts; and
- the exact `trusted_host_process` trust class and honest process-group support facts.

The resolved capability snapshot retains those bounded descriptor facts in durable attempt
provenance. The attempt inspector returns the snapshot, implementation, content, profile, and
execution-policy digests plus optional safe package/documentation references; it never returns an
executable path.

Capability selection checks the profile's full resource ceilings, even for a smaller task. The
grant's `artifact_bytes` must cover `max_total_materialized_bytes + max_total_output_bytes`;
duration, invocation, and concurrency grants must also cover the declared requirements.

### Persistent authorized repository working directory

`working_directory` normally uses `isolated_root` or a relative directory beneath the fresh
Milkdrift-owned execution root. Ordered coding prompts may instead select one exact operator-owned
repository:

```json
"working_directory": {
  "type": "authorized_host_path",
  "path": "/srv/milkdrift/worktrees/project-a"
}
```

The exact path must be covered by a `read_write` `filesystem_roots` entry. Registration
canonicalizes it and requires an ordinary directory. Every invocation re-resolves the configured
path and rejects a path change, symlink replacement, non-directory, or escape from the canonical
read-write roots before process entry. Separate invocations therefore receive fresh context and
isolated Milkdrift materialization/output roots while their child processes intentionally share the
same repository files.

This mode is for operator-authorized sequential work. It is not safe implicit concurrency and does
not implement Git, branch selection, dirty-tree checks, credentials, merges, cleanup, or rollback.
Those remain explicit process/capability operations and prompt-sequence repository-policy facts.
Parallel alternatives should use separately prepared worktrees or another adapter with a stronger
workspace isolation contract.

## Registration, rotation, and health

At registration the adapter canonicalizes the executable and every configured native root,
verifies the executable remains within an execute root using native path components, and converts
the resulting host evidence to the authority grammar (`/...` on Unix and uppercase `C:/...` with
`/` separators for ordinary Windows drives). It likewise verifies an authorized host working
directory is beneath a canonical read-write root before conversion. Authority evaluation is then
pure and uses exact root identity and component containment; it does not query the filesystem or
approximate Windows case behavior. The adapter opens and streams the regular file, verifies
executable permissions on Unix, and compares the observed size and BLAKE3 digest with the
declaration. A symlink resolution change, root escape, non-regular source, permission failure, or
identity mismatch refuses registration.

Health repeats the same identity check. A mismatch marks that immutable adapter generation
unavailable with a bounded `tool_*` reason code and does not mutate its descriptor. Invalidation is
sticky: restoring the previous bytes does not revive the old registered adapter. Immediately
before `Command::spawn`, after inputs, argv, environment, and secrets are prepared, the adapter
rehashes and revalidates the same identity and root/path facts. A mismatch rejects before child
entry and releases ordinary host admission ownership. A successful spawn records the exact
verified identity digest in bounded attempt progress, while the frozen attempt snapshot supplies
the complete safe identity facts.

To deploy changed executable bytes or package identity:

1. stop selecting the old generation or let health make it unavailable;
2. compute the new exact digest and size;
3. advance both profile and descriptor revision;
4. update the immutable implementation declaration and any policy changes; and
5. explicitly register/restart with the new profile generation.

Schema v1 was path-only and is deliberately refused. There is no automatic migration because
silently hashing whatever happens to exist during startup would turn mutable host state into an
operator identity decision. Regenerate v1 profiles as schema v2 under operator control.

## Execution and trust boundaries

An isolated workflow may materialize a selected repository input into its fresh execution
directory and export a declared patch. The coding-agent template instead uses the explicit
`authorized_host_path` mode for persistent operator-owned repository progress and bounded prompt
stdin. Each invocation starts a fresh process; a recorded PID is never restart identity. Template
platform facts are the Unix values and must equal the support facts of the build loading the document.

Secret-bearing profiles must keep process-text progress streaming disabled. Captures remain
bounded artifacts and exact secret bytes are redacted before publication. Configure
`retry_with_stable_key` only when the executable really accepts a profile-scoped stable key through
an `idempotency_key` substitution; otherwise restart retains a lost process as uncertain.

`trusted_host_process` does not constrain what the executable itself can read, write, execute, or
reach over the network using the daemon account. Materialization roots and symlink checks protect
Milkdrift-owned staging/import operations; they are not a filesystem jail. Network declarations
are policy/authority facts unless an external host mechanism enforces them. Broad trusted-process
grants therefore give broad host-code execution authority and must be explicit. A workflow or
grant requiring `sandboxed_process` never matches this adapter; a real container, namespace, or VM
sandbox belongs in a separate future adapter.

Portable safe Rust cannot atomically execute the already-hashed open file on every supported
platform. The adapter checks an open handle against path metadata, re-resolves the configured path,
and keeps the final verification immediately adjacent to spawn, but a residual replacement race
remains between that check and OS process entry. Unix process groups support observed group cleanup
but a malicious descendant can escape into another session/group. Non-Unix builds report no
complete process-tree cancellation. Child-count and resource limits remain observations unless an
external host sandbox enforces them.
