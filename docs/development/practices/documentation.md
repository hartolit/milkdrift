# Documentation practice

Use this practice when developing or reviewing explanations in prose, code comments, package
introductions, and examples. It owns how those explanations teach a reader and how to check them.
Use the [workflow](../workflow.md) for assignment scope, completion, and verification, and the
[practice guide](README.md) to select complementary guidance.

## Contents

1. [Explain the work to a reader](#1-explain-the-work-to-a-reader)
2. [Code comments must add understanding](#2-code-comments-must-add-understanding)
3. [Give every package an introduction](#3-give-every-package-an-introduction)
4. [Check accuracy and usefulness](#4-check-accuracy-and-usefulness)
5. [Work on an explanation](#5-work-on-an-explanation)

## 1. Explain the work to a reader

Write for someone trying to understand or use Milkdrift. A contributor may know Rust without
knowing why these components exist; an operator may know a shell without knowing the implementation.
Start with what they are trying to accomplish and the idea that makes the system understandable.
Explain how the relevant parts fit together before asking the reader to absorb their details.

A useful explanation connects a design choice to its purpose: keeping a command's result lets a
caller recover a lost reply without appending its events again. Merely listing identity,
persistence, validation, and replay properties leaves the reader to discover that connection.
Reading more implementation is the author's job, not a prerequisite imposed on the reader.

Use direct subjects and verbs, concrete situations, and consistent technical names. Introduce a
necessary term where it helps explain something; preserve distinctions such as a workflow
definition versus its execution, or a failed operation versus an unknown outcome. Avoid both
compressed jargon and paragraphs that expand the same jargon without explaining the relationship.

Choose the form that teaches the idea most clearly. A short example, comparison, table, or ASCII
diagram can replace several paragraphs about order, ownership, or data flow. Label what arrows
mean and keep the picture small enough to read in source and rendered docs. For example, a
conceptual context-selection sketch might be:

```text
task policy + available inputs + access and branch checks
                            |
                            v
                    runtime selects inputs
                            |
                            v
            manifest records selections and omissions
```

This sketch explains a relationship; API detail belongs with the relevant operation. Verify
diagrams against the same sources as prose and label conceptual sketches or pseudocode.
Use them where they reduce explanation, without a diagram quota or decorative boxes around
obvious code.

Length follows the reader's need. Some concepts need a worked example; some need one sentence;
some code needs no comment. There is no required paragraph count, comment density, list of
properties to mention, or preferred amount of added text. Retaining, shortening, moving, or
deleting documentation can be the right improvement.

## 2. Code comments must add understanding

Read callers and the surrounding operation before documenting a symbol. Decide what a reader
would otherwise have to infer: why it exists, when to choose it, how it participates in the work,
or which consequence is easy to miss. Explain that. Translating a signature, field list, or
validation branch into English rarely supplies the missing understanding.

Put the explanation where a reader needs it:

- A crate or module introduces the operation and the components that collaborate in it.
- An important type or trait explains its role, how a caller enters that operation, and the
  obligations that make the collaboration work.
- A method or field adds the choices or behavior specific to it, linking to the shared explanation
  when useful. It need not retell the type's purpose or the whole workflow.
- A private or test comment explains a non-obvious decision, dependency, or setup. Remove narration
  of straightforward code; leave already clear code alone.

These are placement choices, not a template to fill in for every item. A simple accessor can
have a brief label when the type already supplies context. Keep required public rustdoc under
the repository's `missing_docs` lint, but do not expand it merely to resemble substantial docs.
Do not add repeated disclaimers about work a getter or value constructor obviously does not do.

Contract details still matter when someone must act on them. Preserve meaningful units, defaults,
special values, failure conditions, side effects, and implementation obligations at their owning
API. Explain, for example, whether an error can arrive after a write, because that changes how
the caller recovers. Use `# Errors`, `# Panics`, and examples where they help; do not enumerate
every validation branch or add boilerplate sections by default.

Constants often need little prose. State any ambiguity in what is counted or what the value
means, and put shared rationale with the owning concept. Do not infer why a particular numeric
threshold was chosen. This comment is accurate, but mostly narrates a constructor:

```rust
/// Maximum bytes in each of a receipt's canonical audit and intent JSON documents.
/// CommandReceipt::new_idempotent rejects empty documents or either document above
/// this limit; it is not a combined allowance for the pair.
```

The receipt's documentation is the place to explain the purpose:

```rust
/// Identifies a runtime command so a lost reply can be recovered from storage
/// without appending the command's events again.
```

With that context in place, the constant can stay brief:

```rust
/// Byte limit for each JSON document retained by a command receipt.
```

Keep the document requirements with the constructor that accepts them. The point is to give the
reader a coherent explanation and a useful reference, not to attach a miniature manual to the
constant. These excerpts illustrate placement and scale; check the current owner before using
them as API documentation.

Similarly, `TaskContextPolicy` needs to explain which earlier work a task should receive, how to
attach that request to `TaskConfig`, and how runtime selection produces a manifest. The default
choice and access restrictions belong in that explanation. Its accessors do not each need another
account of selection. A manifest version identifies the saved format; the manifest's purpose and
the checks on loaded content belong with its type and reader. Avoid copying one explanatory
paragraph across all three levels.

## 3. Give every package an introduction

Every workspace package, including adapters, applications, and development tools, needs a
`README.md` beside its `Cargo.toml`. Help a newcomer understand why they would use or change the
package, how it participates in a real operation, and where to begin. A directory inventory or
list of exported types is not an introduction.

Build the explanation around a supported use, a small example, or a trace through a real consumer.
Choose whichever teaches the package with the least incidental setup. Link to the relevant API
detail and adjacent owners. Include setup, feature choices, limits, or verification guidance when
they affect that use; a small internal helper does not need an operator manual or a catalogue of
everything it declines to do.

Library `//!` documentation must also orient readers arriving through rustdoc. Keep enough
purpose and direction there to start using the API. Put detailed contracts with their APIs and
operator procedures in their maintained guides. Do not repeat the same walkthrough in a README,
crate introduction, type, and method. Short introductions may overlap intentionally; substantial
explanations should have one home with useful links.

Scale each introduction to the package. Keep the architecture document's responsibility and
dependency map, status's current qualification, and the operator guide's setup with those owners.
Do not introduce a documentation framework to avoid a few purposeful sentences.

## 4. Check accuracy and usefulness

Trace explanations through the owning implementation, real callers, and relevant tests. Distinguish
established purpose from inference and intended behavior from an implementation gap. Do not invent
a rationale or quietly turn a defect into a promise. Keep Rust usage examples executable as doctests
where practical; label excerpts and pseudocode so readers know what they can run. Do not use
`ignore` or `no_run` merely to hide a broken example.

Choose checks under the [verification policy](../workflow.md#choose-verification-for-the-change).
Preserve fixture bytes, wire fields, versions, CLI spellings, and recorded evidence. Check that
links, examples, and diagrams remain readable in their intended format; inspect rendered output
when source review cannot establish that. Compilation, link checks, and `missing_docs` establish
useful properties, but cannot establish that an explanation teaches.

Review as a reader first, then check accuracy against source. Can you explain why the components
exist, follow the work between them, and choose the relevant API without reverse-engineering it?
For a consequential operation, can you anticipate the outcome or mistake that matters? Apply
these questions to the explanation as a whole, not as a form every symbol must complete.

Also read for subtraction: which sentences merely restate code, repeat a nearby explanation, or
interrupt the main idea with detail that belongs elsewhere? A review should be willing to retain
clear writing and request cuts as well as additions. Identify the actual confusion or unnecessary
burden when requesting a change; neither longer comments nor more examples demonstrate improvement
by themselves. Use the [office procedure](../virtual-office/README.md) for temporary working notes.

## 5. Work on an explanation

Begin by tracing the behavior being explained and improving its explanation. A documentation task
does not authorize code redesign, API renaming, new features, or changes to serialized data.
A broad documentation assignment should cover a complete
related area, including its introductions, API explanations, examples, and surrounding guidance.
Do not reduce it to a succession of symbol or package fragments requiring fresh assignments.
Use internal iterations to manage the work and hand off when the assigned reader outcome is complete.
If prose and code disagree, establish which is wrong; do not describe an accidental implementation
as the intended contract. Apply the [findings policy](../workflow.md#findings-beyond-the-assignment).
