# Development practices

Practices guide substantial kinds of work in Milkdrift. Choose the guidance that helps complete
the assignment, and reference its files directly in the prompt. Each practice owns its detailed
rules and examples; [AGENTS.md](../../../AGENTS.md) remains the repository entry point.

## Choose and combine practices

| Practice | Use it for |
| --- | --- |
| [Implementation](implementation.md) | Designing, implementing, refactoring, or reviewing code, including Rust mechanisms, ownership, adoption, and module boundaries. |
| [Documentation](documentation.md) | Developing or reviewing explanations in prose, API docs, comments, package introductions, and examples. |

Read the practices relevant to the actual work, including when an assignment omits an explicit
link. Apply their requirements within the authorized scope. Practices may be used alone or
combined with complementary guidance in the order the assignment needs; there is no required
pairing, parent practice, or fixed sequence. A reference to another practice helps locate shared
guidance without requiring the whole catalogue for every task.

Selecting practices does not change product constraints, authorize additional work, or replace
reading the relevant source, consumers, and tests. [Architecture](../../architecture.md) owns
system responsibilities and invariants. The [workflow](../workflow.md) owns assignment procedure,
scope, completion, and verification. The [virtual office](../virtual-office/README.md) owns
temporary sprint coordination and its whiteboard.

## Keep each practice useful

Add a practice when real work establishes a substantial, reusable area of guidance that an
assignment can sensibly select. Keep related techniques together when they support the same
purpose. Avoid creating a file for every technique or placeholders for possible future work.

Give shared guidance one home and link to it from complementary practices. A specialized practice
adds its own methods and judgment without copying the general rules. If two practices repeatedly
need to be read and changed together, reconsider their boundary. File counts and preferred
document lengths are not reasons to split or merge them.
