//! Integration tests for deterministic context planning and capacity handling.

use context_planner::{
    ContextBudget, ContextContent, ContextEntry, ContextEntryId, ContextPersistence,
    ContextPriority, ContextRole, ContextSource, PlanWorkspace, PlanningError,
    exact_token_correction_candidate_index, plan,
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

fn correction_candidate(entries: &[ContextEntry<'static>]) -> Result<Option<usize>, PlanningError> {
    let budget = ContextBudget::new(100, 0)?;
    let mut ordering = [0_usize; 3];
    let mut selected = [0_usize; 3];
    let mut dropped = [0_usize; 3];
    let result = plan(
        entries,
        budget,
        PlanWorkspace {
            ordering: &mut ordering,
            selected: &mut selected,
            dropped: &mut dropped,
        },
    )?;

    Ok(result.exact_token_correction_candidate_index())
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
fn exact_token_correction_candidate_uses_selection_order() -> Result<(), PlanningError> {
    let lower_priority = [
        entry(1, 2, 1, ContextPersistence::Retained, 1, "lower"),
        entry(2, 1, 2, ContextPersistence::Ephemeral, 1, "higher"),
    ];
    assert_eq!(correction_candidate(&lower_priority)?, Some(0));

    let ephemeral = [
        entry(1, 1, 1, ContextPersistence::Retained, 1, "retained"),
        entry(2, 2, 1, ContextPersistence::Ephemeral, 1, "ephemeral"),
    ];
    assert_eq!(correction_candidate(&ephemeral)?, Some(1));

    let older = [
        entry(1, 2, 1, ContextPersistence::Retained, 1, "newer"),
        entry(2, 1, 1, ContextPersistence::Retained, 1, "older"),
    ];
    assert_eq!(correction_candidate(&older)?, Some(1));

    let identity_tie_break = [
        entry(2, 1, 1, ContextPersistence::Retained, 1, "larger id"),
        entry(1, 1, 1, ContextPersistence::Retained, 1, "smaller id"),
    ];
    assert_eq!(correction_candidate(&identity_tie_break)?, Some(0));
    Ok(())
}

#[test]
fn exact_token_correction_candidate_advances_as_selected_indices_shrink() {
    let entries = [
        entry(1, 0, 0, ContextPersistence::Pinned, 1, "pinned"),
        entry(2, 5, 1, ContextPersistence::Retained, 1, "low priority"),
        entry(3, 5, 2, ContextPersistence::Ephemeral, 1, "ephemeral"),
        entry(4, 1, 2, ContextPersistence::Retained, 1, "older"),
        entry(6, 2, 2, ContextPersistence::Retained, 1, "larger id"),
        entry(5, 2, 2, ContextPersistence::Retained, 1, "smaller id"),
    ];

    let candidates = [
        exact_token_correction_candidate_index(&entries, &[0, 1, 2, 3, 4, 5]),
        exact_token_correction_candidate_index(&entries, &[0, 2, 3, 4, 5]),
        exact_token_correction_candidate_index(&entries, &[0, 3, 4, 5]),
        exact_token_correction_candidate_index(&entries, &[0, 4, 5]),
        exact_token_correction_candidate_index(&entries, &[0, 5]),
        exact_token_correction_candidate_index(&entries, &[0]),
    ];

    assert_eq!(
        candidates,
        [Some(1), Some(2), Some(3), Some(4), Some(5), None]
    );
}

#[test]
fn exact_token_correction_candidate_is_none_when_all_selected_entries_are_pinned()
-> Result<(), PlanningError> {
    let entries = [
        entry(1, 1, 1, ContextPersistence::Pinned, 1, "a"),
        entry(2, 2, 1, ContextPersistence::Pinned, 1, "b"),
    ];

    assert_eq!(correction_candidate(&entries)?, None);
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
