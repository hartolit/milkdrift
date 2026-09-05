# External evidence profile templates

These are safe placeholders for the operator-driven interoperability harness. They contain no
working credential, executable identity, or provider model selection.

- Replace the coding-agent executable, its parent execute root, exact BLAKE3 `b3_…` digest, byte
  size, package revision, platform facts, argv, and secret mapping. The harness replaces the
  working directory with its disposable repository and adds only its isolated session/repository
  roots. Review argv against the exact executable you pin; the template does not establish
  compatibility with a particular agent release or authentication setup.
- Replace the endpoint URL, allowlisted host, exact model alias, feature claims, and secret
  reference. Use `http://127.0.0.1:PORT`, `no_auth`, and `local_development: true` only for a real
  loopback model server. Remote endpoints require HTTPS.
- Copy a template outside the repository before replacing placeholders. Do not commit the rendered
  profile if it reveals private endpoint identity or operational configuration.

The complete command and interpretation rules are in
[`docs/guides/external-evidence.md`](../../docs/guides/external-evidence.md).
