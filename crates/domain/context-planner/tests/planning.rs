//! Integration tests for deterministic context planning and capacity handling.

use context_planner::{
    ContextBudget, ContextContent, ContextEntry, ContextEntryId, ContextPersistence,
    ContextPriority, ContextRole, ContextSource, PlanWorkspace, PlanningError, plan,
};

const fn entry(
    id: u64,
    ordinal: u64,
    priority: u8,
    persistence: ContextPersistence,
    estimated_tokens: u32,
    text: &'static str,
) -> ContextEntry<'static> {
    ContextEntry {
        id: ContextEntryId::new(id),
        ordinal,
        role: ContextRole::User,
        source: ContextSource::User,
        priority: ContextPriority::new(priority),
        persistence,
        estimated_tokens,
        content: ContextContent::Text(text),
    }
}

#[test]
fn planner_preserves_pinned_and_selects_high_priority_entries() -> Result<(), PlanningError> {
    let entries = [
        entry(1, 1, 255, ContextPersistence::Pinned, 3, "system"),
        entry(2, 2, 10, ContextPersistence::Retained, 4, "old"),
        entry(3, 3, 20, ContextPersistence::Retained, 4, "important"),
        entry(4, 4, 20, ContextPersistence::Ephemeral, 2, "latest"),
    ];
    let budget = ContextBudget::new(11, 2)?;
    let mut ordering = [0_usize; 4];
    let mut selected = [0_usize; 4];
    let mut dropped = [0_usize; 4];

    let result = plan(
        &entries,
        budget,
        PlanWorkspace {
            ordering: &mut ordering,
            selected: &mut selected,
            dropped: &mut dropped,
        },
    )?;

    assert_eq!(result.input_tokens(), 9);
    assert_eq!(result.selected_indices(), &[0, 2, 3]);
    assert_eq!(result.dropped_indices(), &[1]);
    let ids: std::vec::Vec<_> = result
        .selected_entries()
        .map(|value| value.id.get())
        .collect();
    assert_eq!(ids, [1, 3, 4]);
    Ok(())
}

#[test]
fn pinned_overflow_is_explicit() -> Result<(), PlanningError> {
    let entries = [
        entry(1, 1, 1, ContextPersistence::Pinned, 5, "a"),
        entry(2, 2, 1, ContextPersistence::Pinned, 5, "b"),
    ];
    let budget = ContextBudget::new(8, 1)?;
    let mut ordering = [0_usize; 2];
    let mut selected = [0_usize; 2];
    let mut dropped = [0_usize; 2];

    let result = plan(
        &entries,
        budget,
        PlanWorkspace {
            ordering: &mut ordering,
            selected: &mut selected,
            dropped: &mut dropped,
        },
    );

    assert_eq!(
        result.err(),
        Some(PlanningError::PinnedBudgetExceeded {
            required_tokens: 10,
            available_tokens: 7,
        })
    );
    Ok(())
}

#[test]
fn duplicate_entry_identity_is_rejected() -> Result<(), PlanningError> {
    let entries = [
        entry(1, 1, 1, ContextPersistence::Retained, 1, "a"),
        entry(1, 2, 1, ContextPersistence::Retained, 1, "b"),
    ];
    let budget = ContextBudget::new(8, 1)?;
    let mut ordering = [0_usize; 2];
    let mut selected = [0_usize; 2];
    let mut dropped = [0_usize; 2];

    let result = plan(
        &entries,
        budget,
        PlanWorkspace {
            ordering: &mut ordering,
            selected: &mut selected,
            dropped: &mut dropped,
        },
    );

    assert_eq!(
        result.err(),
        Some(PlanningError::DuplicateEntryId(ContextEntryId::new(1)))
    );
    Ok(())
}
