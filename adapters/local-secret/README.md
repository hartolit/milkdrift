# Local secret sources

This adapter turns an explicit `SecretRef` into sensitive bytes at the point an authorized caller
needs a credential. Configuration maps the reference to an environment variable or absolute file
path. The mapping contains no secret value; each resolution rereads the source so rotation is visible
without caching credential bytes.

Use `LocalSecretSource::environment` or `LocalSecretSource::file`, collect the sources in a
`BTreeMap<SecretRef, LocalSecretSource>`, and construct `LocalSecretResolver`. The daemon's
[authenticator](../../apps/daemon/src/auth.rs) builds this resolver and shares it with process, model,
and peer adapters. Operator syntax belongs in the [authority guide](../../docs/operations/authority.md).

Source constructors validate configuration without reading it. Resolution accepts nonempty values
up to 4,096 bytes. Files must be regular and at most 4,097 bytes before one trailing LF or CRLF is
removed; the final value must still fit 4,096 bytes. On Unix, any group/other permission bit causes
refusal. The non-Unix implementation does not inspect ACLs, so file access restrictions must be
established outside this resolver. It does not enumerate environment variables or search for files.

`SecretResolver::resolve` receives no actor or grant. Its caller must already have authorized the
reference; possession of a configured name is not an authority check. Missing mappings, unreadable
sources, and invalid values return the same redacted `Unavailable` error. `SensitiveSecret::expose`
limits where callers deliberately use the bytes, while `Debug` hides source locations and values.
The tests beside the implementation check rotation, bounds, and redaction; the environment test
uses a child process to avoid changing the test runner's shared environment.
