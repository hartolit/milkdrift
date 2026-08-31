# External evidence profile templates

These are safe placeholders for the operator-driven interoperability harness. They contain no
working credential, executable identity, or provider model selection.

- Replace the coding-agent executable, its parent execute root, exact BLAKE3 `b3_…` digest, byte
  size, package revision, platform facts, argv, and secret mapping. The harness replaces the
  working directory with its disposable repository and adds only its isolated session/repository
  roots. The example argv was checked against `codex-cli 0.147.0-alpha.6.6`; it is an example, not
  proof that another Codex release or authentication setup behaves identically.
- Replace the endpoint URL, allowlisted host, exact model alias, feature claims, and secret
  reference. Use `http://127.0.0.1:PORT`, `no_auth`, and `local_development: true` only for a real
  loopback model server. Remote endpoints require HTTPS.
- Copy a template outside the repository before replacing placeholders. Do not commit the rendered
  profile if it reveals private endpoint identity or operational configuration.

The complete command and interpretation rules are in
[`docs/external-evidence.md`](../../docs/external-evidence.md).
