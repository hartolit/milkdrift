# 0030 — Bounded headless operator contract

Status: accepted

The CLI is an external control client. Its direct Milkdrift dependencies are the control client,
control protocol, and prompt-sequence authoring owner. Revision, proposal, authority, persistence,
workspace, and adapter semantics remain behind those owners. Local sequence compilation emits an
ordinary immutable blueprint document; remediation construction and stage association consume
canonical document bytes inside the prompt-sequence owner.

CLI JSON schema 2 replaces schema 1 atomically. Every operation emits the same bounded stdout
envelope, including failures, help/version, reconnect transitions, and final stream outcomes.
There is no schema-1 compatibility switch. Raw documents and artifact bytes require an explicit
file destination in JSON mode. Error details are fixed redacted classifications, never raw
transport or document content. The control protocol and durable document versions do not change.

Ordinary operations have a visible default deadline. Run wait and noninteractive follow require
an explicit deadline, with additional bounded polling/reconnection. Timeout or client cancellation
ends observation and does not assert that accepted work stopped. Only daemon evidence establishes
terminal or uncertain work. Partial CLI output files have one cancellation-safe owner.

Maintained operator files under `examples/operator` are validated by the production reader. The
initial authority is workflow-scoped and finite, with no external adapter enabled. Run creation
clips the daemon's artifact defaults to the authenticated grant ceiling; it never expands a
grant. Artifact materialization reads that accepted immutable run budget from the first journal
event; the host constructor no longer accepts a competing global publication budget.
Additional process/model scopes require explicit configuration and existing authority
evaluation. Milkdrift configures a separately managed model endpoint without managing its server.

The actual-binary evidence lane consumes those accepted files and the public endpoint from a fresh
directory. Mock endpoints and byte-pinned helper processes supply deterministic external work;
they do not substitute for real model qualification or platform crash-durability evidence.
