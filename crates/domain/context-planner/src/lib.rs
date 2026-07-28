#![no_std]
#![forbid(unsafe_code)]
#![doc = "Deterministic context planning over caller-owned storage."]

use core::cmp::Ordering;
use core::slice;

use domain_contracts::{ArtifactId, CapacityExhausted, CapacityResource, TokenId};

/// Stable identity of one context entry.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContextEntryId(u64);

impl ContextEntryId {
    /// Creates a context-entry identifier.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the stable numeric value.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Semantic role assigned to an entry before model-specific rendering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ContextRole {
    /// System-level instruction.
    System,
    /// User-provided content.
    User,
    /// Assistant-produced content.
    Assistant,
    /// Tool request or result.
    Tool,
    /// Retrieved supporting material.
    Retrieved,
    /// Compiler, validator, or reviewer diagnostic.
    Diagnostic,
}

/// Provenance category used for inspection and policy decisions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ContextSource {
    /// Application-defined instruction.
    Application,
    /// Direct user input.
    User,
    /// Model-generated output.
    Model,
    /// External tool output.
    Tool,
    /// Retrieved artifact or document.
    Retrieval,
    /// Deterministic validator output.
    Validator,
}

/// Retention policy applied during budget selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextPersistence {
    /// Entry must be selected or planning fails.
    Pinned,
    /// Entry competes normally for available input tokens.
    Retained,
    /// Entry is eligible for removal before retained entries at equal priority.
    Ephemeral,
}

/// Explicit context priority. Larger values are selected first.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContextPriority(u8);

impl ContextPriority {
    /// Creates a priority value.
    #[must_use]
    pub const fn new(value: u8) -> Self {
        Self(value)
    }

    /// Returns the raw priority value.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

/// Non-owning entry payload retained independently from the selection plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ContextContent<'a> {
    /// UTF-8 text rendered later by a model-specific template.
    Text(&'a str),
    /// Already-tokenized content.
    Tokens(&'a [TokenId]),
    /// Immutable artifact resolved by an application or adapter.
    Artifact(ArtifactId),
}

/// One typed context record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContextEntry<'a> {
    /// Stable entry identity.
    pub id: ContextEntryId,
    /// Monotonic conversation or workflow order.
    pub ordinal: u64,
    /// Semantic role.
    pub role: ContextRole,
    /// Provenance category.
    pub source: ContextSource,
    /// Explicit selection priority.
    pub priority: ContextPriority,
    /// Retention policy.
    pub persistence: ContextPersistence,
    /// Conservative token estimate used for admission.
    pub estimated_tokens: u32,
    /// Non-owning content payload.
    pub content: ContextContent<'a>,
}

/// Total model-context budget and reserved generation capacity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContextBudget {
    /// Maximum model context length.
    pub maximum_tokens: u32,
    /// Tokens kept free for model output.
    pub reserved_output_tokens: u32,
}

impl ContextBudget {
    /// Creates and validates a context budget.
    ///
    /// # Errors
    ///
    /// Returns [`PlanningError::InvalidBudget`] when `maximum_tokens` is zero or
    /// `reserved_output_tokens` exceeds `maximum_tokens`.
    pub const fn new(
        maximum_tokens: u32,
        reserved_output_tokens: u32,
    ) -> Result<Self, PlanningError> {
        if maximum_tokens == 0 || reserved_output_tokens > maximum_tokens {
            return Err(PlanningError::InvalidBudget);
        }
        Ok(Self {
            maximum_tokens,
            reserved_output_tokens,
        })
    }

    /// Returns tokens available to selected input entries.
    #[must_use]
    pub const fn available_input_tokens(self) -> u32 {
        self.maximum_tokens
            .saturating_sub(self.reserved_output_tokens)
    }
}

/// Caller-owned scratch and result buffers.
pub struct PlanWorkspace<'a> {
    /// Candidate ordering scratch with capacity for every entry.
    pub ordering: &'a mut [usize],
    /// Selected entry indices with capacity for every entry.
    pub selected: &'a mut [usize],
    /// Dropped entry indices with capacity for every entry.
    pub dropped: &'a mut [usize],
}

/// Deterministic context-planning failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PlanningError {
    /// Maximum context length or reservation is invalid.
    InvalidBudget,
    /// Two entries share one identity.
    DuplicateEntryId(ContextEntryId),
    /// Pinned entries alone exceed the available input budget.
    PinnedBudgetExceeded {
        /// Tokens required by pinned entries.
        required_tokens: u64,
        /// Tokens available after output reservation.
        available_tokens: u64,
    },
    /// A caller-owned planning buffer is too small.
    CapacityExhausted(CapacityExhausted),
}

impl From<CapacityExhausted> for PlanningError {
    fn from(value: CapacityExhausted) -> Self {
        Self::CapacityExhausted(value)
    }
}

/// Borrowed result that references original entries and caller-owned index buffers.
pub struct ContextPlan<'entries, 'workspace> {
    entries: &'entries [ContextEntry<'entries>],
    selected: &'workspace [usize],
    dropped: &'workspace [usize],
    input_tokens: u32,
    reserved_output_tokens: u32,
    maximum_tokens: u32,
}

impl<'entries> ContextPlan<'entries, '_> {
    /// Returns selected indices in original ordinal order.
    #[must_use]
    pub const fn selected_indices(&self) -> &[usize] {
        self.selected
    }

    /// Returns dropped indices in original ordinal order.
    #[must_use]
    pub const fn dropped_indices(&self) -> &[usize] {
        self.dropped
    }

    /// Returns the admitted input token estimate.
    #[must_use]
    pub const fn input_tokens(&self) -> u32 {
        self.input_tokens
    }

    /// Returns tokens reserved for generation.
    #[must_use]
    pub const fn reserved_output_tokens(&self) -> u32 {
        self.reserved_output_tokens
    }

    /// Returns the complete model context capacity.
    #[must_use]
    pub const fn maximum_tokens(&self) -> u32 {
        self.maximum_tokens
    }

    /// Iterates selected entries without copying their content.
    #[must_use]
    pub fn selected_entries(&self) -> ContextEntryIter<'_, 'entries> {
        ContextEntryIter {
            entries: self.entries,
            indices: self.selected.iter(),
        }
    }
}

/// Iterator over entries referenced by a context plan.
pub struct ContextEntryIter<'plan, 'entries> {
    entries: &'entries [ContextEntry<'entries>],
    indices: slice::Iter<'plan, usize>,
}

impl<'entries> Iterator for ContextEntryIter<'_, 'entries> {
    type Item = &'entries ContextEntry<'entries>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let index = *self.indices.next()?;
            if let Some(entry) = self.entries.get(index) {
                return Some(entry);
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.indices.size_hint()
    }
}

impl ExactSizeIterator for ContextEntryIter<'_, '_> {}

/// Selects entries into a deterministic budgeted plan.
///
/// # Errors
///
/// Returns [`PlanningError::CapacityExhausted`] when a workspace buffer cannot
/// hold every entry, [`PlanningError::DuplicateEntryId`] when entry identities
/// are not unique, or [`PlanningError::PinnedBudgetExceeded`] when pinned entries
/// exceed the available input-token budget.
pub fn plan<'entries, 'workspace>(
    entries: &'entries [ContextEntry<'entries>],
    budget: ContextBudget,
    workspace: PlanWorkspace<'workspace>,
) -> Result<ContextPlan<'entries, 'workspace>, PlanningError> {
    validate_workspace(entries.len(), &workspace)?;
    validate_unique_ids(entries)?;

    let PlanWorkspace {
        ordering,
        selected,
        dropped,
    } = workspace;

    let available_tokens = u64::from(budget.available_input_tokens());
    let mut selected_length = 0_usize;
    let mut candidate_length = 0_usize;
    let mut pinned_tokens = 0_u64;

    for (index, entry) in entries.iter().enumerate() {
        if entry.persistence == ContextPersistence::Pinned {
            pinned_tokens = pinned_tokens.saturating_add(u64::from(entry.estimated_tokens));
            write_index(selected, selected_length, index)?;
            selected_length += 1;
        } else {
            write_index(ordering, candidate_length, index)?;
            candidate_length += 1;
        }
    }

    if pinned_tokens > available_tokens {
        return Err(PlanningError::PinnedBudgetExceeded {
            required_tokens: pinned_tokens,
            available_tokens,
        });
    }

    let ordering_capacity = ordering.len();
    let Some(candidates) = ordering.get_mut(..candidate_length) else {
        return Err(workspace_capacity(candidate_length, ordering_capacity));
    };
    candidates.sort_unstable_by(|left, right| candidate_order(entries, *left, *right));

    let mut admitted_tokens = pinned_tokens;
    let mut dropped_length = 0_usize;
    for &index in candidates.iter() {
        let Some(entry) = entries.get(index) else {
            return Err(PlanningError::CapacityExhausted(CapacityExhausted::new(
                CapacityResource::ContextEntries,
                (index as u64).saturating_add(1),
                entries.len() as u64,
            )));
        };
        let entry_tokens = u64::from(entry.estimated_tokens);
        if admitted_tokens.saturating_add(entry_tokens) <= available_tokens {
            write_index(selected, selected_length, index)?;
            selected_length += 1;
            admitted_tokens = admitted_tokens.saturating_add(entry_tokens);
        } else {
            write_index(dropped, dropped_length, index)?;
            dropped_length += 1;
        }
    }

    let selected_capacity = selected.len();
    let Some(selected_result) = selected.get_mut(..selected_length) else {
        return Err(workspace_capacity(selected_length, selected_capacity));
    };
    selected_result.sort_unstable_by(|left, right| ordinal_order(entries, *left, *right));

    let dropped_capacity = dropped.len();
    let Some(dropped_result) = dropped.get_mut(..dropped_length) else {
        return Err(workspace_capacity(dropped_length, dropped_capacity));
    };
    dropped_result.sort_unstable_by(|left, right| ordinal_order(entries, *left, *right));

    let input_tokens =
        u32::try_from(admitted_tokens).map_err(|_| PlanningError::PinnedBudgetExceeded {
            required_tokens: admitted_tokens,
            available_tokens,
        })?;

    Ok(ContextPlan {
        entries,
        selected: selected_result,
        dropped: dropped_result,
        input_tokens,
        reserved_output_tokens: budget.reserved_output_tokens,
        maximum_tokens: budget.maximum_tokens,
    })
}

fn validate_workspace(required: usize, workspace: &PlanWorkspace<'_>) -> Result<(), PlanningError> {
    let available = workspace
        .ordering
        .len()
        .min(workspace.selected.len())
        .min(workspace.dropped.len());
    if available < required {
        return Err(workspace_capacity(required, available));
    }
    Ok(())
}

fn validate_unique_ids(entries: &[ContextEntry<'_>]) -> Result<(), PlanningError> {
    for (left_index, left) in entries.iter().enumerate() {
        let Some(tail) = entries.get(left_index.saturating_add(1)..) else {
            continue;
        };
        if tail.iter().any(|right| right.id == left.id) {
            return Err(PlanningError::DuplicateEntryId(left.id));
        }
    }
    Ok(())
}

fn write_index(output: &mut [usize], position: usize, value: usize) -> Result<(), PlanningError> {
    let available = output.len();
    let Some(slot) = output.get_mut(position) else {
        return Err(workspace_capacity(position.saturating_add(1), available));
    };
    *slot = value;
    Ok(())
}

const fn workspace_capacity(required: usize, available: usize) -> PlanningError {
    PlanningError::CapacityExhausted(CapacityExhausted::new(
        CapacityResource::ContextEntries,
        required as u64,
        available as u64,
    ))
}

fn candidate_order(entries: &[ContextEntry<'_>], left: usize, right: usize) -> Ordering {
    let left_entry = entries.get(left);
    let right_entry = entries.get(right);
    match (left_entry, right_entry) {
        (Some(left_entry), Some(right_entry)) => right_entry
            .priority
            .cmp(&left_entry.priority)
            .then_with(|| {
                persistence_rank(right_entry.persistence)
                    .cmp(&persistence_rank(left_entry.persistence))
            })
            .then_with(|| right_entry.ordinal.cmp(&left_entry.ordinal))
            .then_with(|| left_entry.id.cmp(&right_entry.id)),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => left.cmp(&right),
    }
}

fn ordinal_order(entries: &[ContextEntry<'_>], left: usize, right: usize) -> Ordering {
    match (entries.get(left), entries.get(right)) {
        (Some(left_entry), Some(right_entry)) => left_entry
            .ordinal
            .cmp(&right_entry.ordinal)
            .then_with(|| left_entry.id.cmp(&right_entry.id)),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => left.cmp(&right),
    }
}

const fn persistence_rank(persistence: ContextPersistence) -> u8 {
    match persistence {
        ContextPersistence::Pinned => 2,
        ContextPersistence::Retained => 1,
        ContextPersistence::Ephemeral => 0,
    }
}
