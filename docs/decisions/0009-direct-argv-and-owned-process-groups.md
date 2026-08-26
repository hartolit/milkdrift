# ADR 0009: Direct argv and owned process groups

- Status: accepted
- Date: 2026-08-26

## Context

A generic coding-agent capability needs configurable executables and arguments, but a shell
command string makes invocation inputs part of shell syntax. Killing only the immediate child can
also leave helpers mutating a workspace after cancellation or output import.

## Decision

Process profile schema v1 stores an executable plus a fixed vector of argument templates. Every
named substitution is validated and replaced inside one existing argument; the adapter never
parses or invokes a shell. The child starts with a cleared environment and only explicit
non-secret names and secret references.

On Unix each child enters a new process group. Graceful and forced termination target that group,
the immediate child is reaped, and terminal cancellation is reported only after group absence is
observed. A successful immediate-child exit with surviving owned descendants is a contract
failure and triggers group cleanup. The profile states honestly that descendants can deliberately
escape by creating a different session/group. Non-Unix builds advertise immediate-child-only
behavior and cannot report confirmed tree cancellation.

## Rejected alternatives

- Escaping substitutions into a shell string, because escaping is shell- and context-dependent.
- Killing only `Child`, because grandchildren may retain pipes, files, or mutations.
- Treating a sent signal as terminal acknowledgement, because delivery is not observed exit.
- Reconnecting after restart by PID, because PID reuse is not process identity.

## Consequences

Shell metacharacters remain literal argument bytes. Profiles that truly need shell semantics must
be a future separate capability with stronger authority. Resource counts such as child count are
observations, not isolation guarantees; enforced CPU/memory/process ceilings require a future
platform sandbox or cgroup/job-object boundary.

## Reconsideration triggers

Introduce a different ownership implementation when a maintained safe API provides stronger
non-Unix job control or verified non-escapable Unix containment without moving unsafe code into
this crate.
