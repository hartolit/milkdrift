# Milkdrift — product vision

Milkdrift lets a person give work to a daemon, watch it develop, and return from another device
without having to keep an agent window open. The daemon owns each workflow: its tasks, execution,
results, permissions and changes. Models and tools perform the work outside the core.

This is the intended product, revised from the user's October 12 account and continuation.
[Status](status.md) describes the existing implementation, which does not yet deliver this whole
experience. [Architecture](../architecture.md) distinguishes existing owners from the selected
replacement design; [the roadmap](roadmap.md) identifies the remaining implementation assignments.
The preceding vision remains in Git and the `milkdrift-pre-product-reset-2026-10-12` tag.

## Work starts with tasks and can grow

A task combines instructions with named input bindings. Instructions can be written before their
inputs exist. Inputs may be text, files or earlier task results; explicit dependencies determine
when the daemon may start. A configured external model and permitted tools supply capabilities.
Research, splitting assignments, criticism and composition are ordinary uses of this task model.

For example, a person asks for a Rust approach to learned animation. A planning task proposes three
research tasks with separate project folders. They run in parallel. A composition task receives
their results, including a failed outcome if it explicitly accepts one, and proposes the next work.
The daemon validates and admits additions; model prose does not secretly become scheduling logic.

People and agents use the same authorized operations to add work or revise tasks that have not
started. Once a task starts, its actual instructions, inputs and capability selection are fixed.
Progress and status may change; corrections become future work with a link to what they address.
A stop request does not prove a process stopped. Lost responses and unresolved remote effects stay
uncertain until evidence or authorized reconciliation resolves them.

Useful automatic continuation needs clear limits. The workflow explains when it is complete, waiting
for input, blocked by failure, paused by a person, or stopped by exhausted authority or budget.
Criticism and repair can continue within the user's grants without another manual launch each round.

## Methods capture reusable preferences

A method is an owner-editable template available across workflows. Using one never gives its consumer
permission to alter the shared template. A queued use refers to the template until it activates.
At activation the owning daemon expands the current saved content into ordinary tasks in that same
workflow and records their origin. Those local future tasks can then be adapted normally. Later
template edits affect future expansions, not existing copies.

Restart cannot duplicate an accepted expansion or silently select newer content for it. Queued
references protect a template from deletion until they are deliberately removed or replaced.
Sharing templates between daemons means copying content under explicit authority, not executing a
remote workflow or keeping a permanent live link to another owner.

## Environments supply tools

An execution host offers commands, model endpoints and managed environments, including the existing
Podman direction. It enforces the access and limits it offers and retains enough operational state
to report accepted work honestly. The workflow's daemon remains its owner when it uses a remote tool.

Connections are configured on that daemon, so its authorized clients see the same available tools
and reported state. Seeing a host through one daemon does not distribute credentials or grant direct
administration of the other host. Operators provide LAN or mesh connectivity. Milkdrift does not
build a network overlay or inference engine.

Project-specific procedures, architecture and environment explanations belong primarily in ordinary
Markdown files and Git. Agents maintain them using ordinary tools. Milkdrift's operational state
exists to run and recover work; it is not a permanent knowledge, documentation or lesson service.

## The client makes the work understandable

Svelte is the first GUI, maintained as a standalone client of public daemon operations. It can
connect to several daemons while keeping their workflows, settings and permissions separate. The
CLI and other authorized clients use the same operations. No client owns execution, opens the
database, calls CLI subprocesses or reconstructs a workflow from a local recovery file.

A daemon owns its human-readable name. An authorized rename persists there and appears across
clients; users do not maintain a different name in every browser. Connecting requires an address
and credential. The client learns and remembers the daemon's stable identity, explains an unexpected
replacement, and keeps technical identity details out of ordinary setup. A display name is not an
identity or a permission.

The timeline runs left to right with titles, short descriptions, actual start and end times, and
clear queued, running, completed, failed and stopped states. Parallel work and dependencies remain
visible. Queued tasks have no fabricated start time. Method copies show origin; corrective tasks
show what they address. An accessible list and inspector provide the same actions and information.

Humans and agents can browse recent workflows and tasks and search retained results on their
selected daemon without already knowing an ID. A fresh authorized client can discover ongoing work.
Closing a phone changes only observation, not execution.

## Keep the useful history, protect the necessary state

Completed history and context use configurable rolling retention. The product explains what expired
and why some state remains protected. Age alone never discards unfinished work, unresolved remote
operations or inputs required by pending tasks. Exact request replay and conflict detection remain
bounded operational responsibilities, not a promise to retain every result forever.

Definitions accepted for execution are immutable and retained execution facts are append-only.
Reconciliation changes the future. Context has explicit sources, bounds and access checks; parallel
tasks receive only their declared inputs and permitted files. Authority and resource ceilings bind
people, agents, services and peers through the same command path.

## Success and exclusions

Success means a person can submit useful work early, inspect real results, repair future work, reuse
a method, use a remote tool and reconnect from a fresh client. Repeatable controlled tests establish
the daemon's construction, scheduling and recovery. Actual user reviews establish usability for the
observed journeys; neither substitutes for evidence about real model quality.

The selected product needs no remote workflow ownership, source-version hierarchy exposed as its
primary UI, specialized splitter/compositor products, permanent forensic archive, mandatory evaluated
learning, central method service or specialized environment-documentation system. Existing evidence
and code remain available in the preserved history without making every former facility a requirement.
