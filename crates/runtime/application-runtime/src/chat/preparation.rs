//! Pure chat inventory, selection, rendering, encoding, correction, and diagnostics.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt::Write as _;

use context_planner::{
    ContextBudget, ContextContent, ContextEntry, ContextEntryId, ContextPersistence,
    ContextPriority, ContextRole, ContextSource, PlanWorkspace, PlanningError,
    exact_token_correction_candidate_index, plan,
};
use domain_contracts::TokenId;
use tokenization::{SpecialTokenPolicy, TokenizationError, Tokenizer};

use super::{
    ContextDiagnostics, PromptCompatibilityProfile, TINYLLAMA_ASSISTANT_MARKER,
    TINYLLAMA_END_OF_MESSAGE, TINYLLAMA_SYSTEM_MARKER, TINYLLAMA_USER_MARKER,
};
use crate::conversation::ConversationTokenEstimate;
use crate::generation::encode_text_with_policy;
use crate::{
    ApplicationError, ApplicationFailure, ApplicationFailureKind, ConversationRecord,
    ConversationRecordId, ConversationRetention, ConversationRole,
};

const PINNED_PRIORITY: ContextPriority = ContextPriority::new(u8::MAX);
const HISTORICAL_PRIORITY: ContextPriority = ContextPriority::new(128);

pub(super) struct ChatPreparation {
    pub(super) prompt_tokens: Box<[TokenId]>,
    pub(super) diagnostics: ContextDiagnostics,
}

pub(super) struct ChatPreparationRequest<'a, T: Tokenizer + ?Sized> {
    pub(super) raw_records: &'a [ConversationRecord],
    pub(super) target_user: ConversationRecordId,
    pub(super) regenerating: bool,
    pub(super) profile: PromptCompatibilityProfile,
    pub(super) tokenizer: &'a T,
    pub(super) maximum_context_tokens: u32,
    pub(super) maximum_prefill_tokens: u32,
    pub(super) reserved_output_tokens: u32,
}

struct ChatPreparationBudget {
    planning: ContextBudget,
    effective_input_tokens: u32,
    maximum_context_tokens: u32,
    reserved_output_tokens: u32,
}

struct ChatContextInventory<'a> {
    units: Vec<ContextPlanningUnit<'a>>,
}

struct ChatSelection<'inventory, 'records> {
    inventory: &'inventory ChatContextInventory<'records>,
    entries: Vec<ContextEntry<'inventory>>,
    selected: Vec<usize>,
    dropped: Vec<ConversationRecordId>,
}

struct RenderedChatPrompt {
    text: String,
    attempt: u32,
}

struct EncodedChatPrompt {
    tokens: Box<[TokenId]>,
    attempt: u32,
}

/// One atomic context-planning unit. A completed historical user/assistant turn is
/// selected or dropped together while raw conversation records remain independent.
pub(super) struct ContextPlanningUnit<'a> {
    primary: &'a ConversationRecord,
    paired_assistant: Option<&'a ConversationRecord>,
    planning_content: Cow<'a, str>,
    persistence: ContextPersistence,
    estimated_tokens: u32,
}

impl<'a> ContextPlanningUnit<'a> {
    fn new(
        primary: &'a ConversationRecord,
        paired_assistant: Option<&'a ConversationRecord>,
        target_user: ConversationRecordId,
    ) -> Self {
        let planning_content = paired_assistant.map_or_else(
            || Cow::Borrowed(primary.content.as_str()),
            |assistant| {
                let capacity = primary
                    .content
                    .len()
                    .saturating_add(assistant.content.len())
                    .saturating_add(1);
                let mut content = String::with_capacity(capacity);
                content.push_str(primary.content.as_str());
                content.push('\n');
                content.push_str(assistant.content.as_str());
                Cow::Owned(content)
            },
        );
        let persistence = unit_persistence(primary, paired_assistant, target_user);
        let estimated_tokens = primary.token_estimate.tokens().saturating_add(
            paired_assistant.map_or(0, |assistant| assistant.token_estimate.tokens()),
        );
        Self {
            primary,
            paired_assistant,
            planning_content,
            persistence,
            estimated_tokens,
        }
    }

    fn append_record_ids(&self, output: &mut Vec<ConversationRecordId>) {
        output.push(self.primary.id);
        if let Some(assistant) = self.paired_assistant {
            output.push(assistant.id);
        }
    }
}

impl PromptCompatibilityProfile {
    pub(super) fn render(
        self,
        units: &[ContextPlanningUnit<'_>],
        selected_indices: &[usize],
    ) -> Result<String, ApplicationError> {
        let mut prompt = String::new();
        for &index in selected_indices {
            let unit = units.get(index).ok_or_else(|| {
                ApplicationFailure::new(
                    ApplicationFailureKind::Worker,
                    "context plan referenced an unavailable planning unit",
                )
            })?;
            self.render_record(&mut prompt, unit.primary)?;
            if let Some(assistant) = unit.paired_assistant {
                self.render_record(&mut prompt, assistant)?;
            }
        }
        writeln!(prompt, "{TINYLLAMA_ASSISTANT_MARKER}").map_err(render_failure)?;
        Ok(prompt)
    }

    fn render_record(
        self,
        prompt: &mut String,
        record: &ConversationRecord,
    ) -> Result<(), ApplicationError> {
        let marker = match (self, record.role) {
            (Self::TinyLlamaChatV1, ConversationRole::System) => TINYLLAMA_SYSTEM_MARKER,
            (Self::TinyLlamaChatV1, ConversationRole::User) => TINYLLAMA_USER_MARKER,
            (Self::TinyLlamaChatV1, ConversationRole::Assistant) => TINYLLAMA_ASSISTANT_MARKER,
        };
        writeln!(prompt, "{marker}").map_err(render_failure)?;
        prompt.push_str(record.content.as_str());
        prompt.push_str(TINYLLAMA_END_OF_MESSAGE);
        prompt.push('\n');
        Ok(())
    }
}

pub(super) fn prepare_chat<T: Tokenizer + ?Sized>(
    request: &ChatPreparationRequest<'_, T>,
) -> Result<ChatPreparation, ApplicationError> {
    let budget = ChatPreparationBudget::calculate(request)?;
    let inventory = ChatContextInventory::build(
        request.raw_records,
        request.target_user,
        request.regenerating,
    )?;
    let mut selection = ChatSelection::plan(&inventory, budget.planning)?;
    let maximum_attempts = selection.maximum_encoding_attempts();

    for attempt in 1..=maximum_attempts {
        let rendered = selection.render(request.profile, attempt)?;
        match rendered.encode(request.tokenizer, budget.effective_input_tokens)? {
            Some(encoded) => return selection.finish(encoded, &budget),
            None => selection.drop_exact_correction_candidate(budget.effective_input_tokens)?,
        }
    }

    Err(ApplicationError::UnchangedContextCorrection)
}

impl ChatPreparationBudget {
    fn calculate<T: Tokenizer + ?Sized>(
        request: &ChatPreparationRequest<'_, T>,
    ) -> Result<Self, ApplicationError> {
        let effective_input_tokens = request
            .maximum_context_tokens
            .checked_sub(request.reserved_output_tokens)
            .ok_or_else(|| ApplicationError::ContextCapacityExceeded {
                required: u64::from(request.reserved_output_tokens),
                available: u64::from(request.maximum_context_tokens),
            })?
            .min(request.maximum_prefill_tokens);
        let planning_maximum = effective_input_tokens
            .checked_add(request.reserved_output_tokens)
            .ok_or_else(|| ApplicationError::ContextCapacityExceeded {
                required: u64::from(effective_input_tokens)
                    .saturating_add(u64::from(request.reserved_output_tokens)),
                available: u64::from(request.maximum_context_tokens),
            })?;
        let planning = ContextBudget::new(planning_maximum, request.reserved_output_tokens)
            .map_err(|_| ApplicationError::ContextCapacityExceeded {
                required: u64::from(request.reserved_output_tokens),
                available: u64::from(request.maximum_context_tokens),
            })?;
        Ok(Self {
            planning,
            effective_input_tokens,
            maximum_context_tokens: request.maximum_context_tokens,
            reserved_output_tokens: request.reserved_output_tokens,
        })
    }
}

impl<'a> ChatContextInventory<'a> {
    fn build(
        raw_records: &'a [ConversationRecord],
        target_user: ConversationRecordId,
        regenerating: bool,
    ) -> Result<Self, ApplicationError> {
        Ok(Self {
            units: build_context_units(raw_records, target_user, regenerating)?,
        })
    }
}

impl<'inventory, 'records> ChatSelection<'inventory, 'records> {
    fn plan(
        inventory: &'inventory ChatContextInventory<'records>,
        budget: ContextBudget,
    ) -> Result<Self, ApplicationError> {
        let entries: Vec<ContextEntry<'inventory>> =
            inventory.units.iter().map(context_entry).collect();
        let mut ordering = vec![0_usize; entries.len()];
        let mut selected_workspace = vec![0_usize; entries.len()];
        let mut dropped_workspace = vec![0_usize; entries.len()];
        let planned = plan(
            entries.as_slice(),
            budget,
            PlanWorkspace {
                ordering: ordering.as_mut_slice(),
                selected: selected_workspace.as_mut_slice(),
                dropped: dropped_workspace.as_mut_slice(),
            },
        )
        .map_err(planning_failure)?;
        let selected = planned.selected_indices().to_vec();
        let dropped = expand_unit_record_ids(inventory.units.as_slice(), planned.dropped_indices());
        Ok(Self {
            inventory,
            entries,
            selected,
            dropped,
        })
    }

    fn maximum_encoding_attempts(&self) -> usize {
        self.selected
            .iter()
            .filter(|index| {
                self.entries
                    .get(**index)
                    .is_some_and(|entry| entry.persistence != ContextPersistence::Pinned)
            })
            .count()
            .saturating_add(1)
    }

    fn render(
        &self,
        profile: PromptCompatibilityProfile,
        attempt: usize,
    ) -> Result<RenderedChatPrompt, ApplicationError> {
        let text = profile.render(self.inventory.units.as_slice(), self.selected.as_slice())?;
        Ok(RenderedChatPrompt {
            text,
            attempt: u32::try_from(attempt).unwrap_or(u32::MAX),
        })
    }

    fn drop_exact_correction_candidate(
        &mut self,
        effective_input_tokens: u32,
    ) -> Result<(), ApplicationError> {
        let Some(candidate) = exact_token_correction_candidate_index(
            self.entries.as_slice(),
            self.selected.as_slice(),
        ) else {
            return Err(ApplicationError::PinnedBudgetExceeded {
                required_at_least: u64::from(effective_input_tokens).saturating_add(1),
                available: u64::from(effective_input_tokens),
            });
        };
        let previous_length = self.selected.len();
        self.selected.retain(|index| *index != candidate);
        if self.selected.len() >= previous_length {
            return Err(ApplicationError::UnchangedContextCorrection);
        }
        let unit = self.inventory.units.get(candidate).ok_or_else(|| {
            ApplicationFailure::new(
                ApplicationFailureKind::Worker,
                "context correction referenced an unavailable planning unit",
            )
        })?;
        unit.append_record_ids(&mut self.dropped);
        Ok(())
    }

    fn finish(
        self,
        encoded: EncodedChatPrompt,
        budget: &ChatPreparationBudget,
    ) -> Result<ChatPreparation, ApplicationError> {
        let actual_input_tokens = u32::try_from(encoded.tokens.len()).map_err(|_| {
            ApplicationError::ContextCapacityExceeded {
                required: u64::try_from(encoded.tokens.len()).unwrap_or(u64::MAX),
                available: u64::from(budget.effective_input_tokens),
            }
        })?;
        Ok(ChatPreparation {
            prompt_tokens: encoded.tokens,
            diagnostics: ContextDiagnostics {
                selected: expand_unit_record_ids(
                    self.inventory.units.as_slice(),
                    self.selected.as_slice(),
                ),
                dropped: self.dropped,
                estimated_input_tokens: selected_estimated_tokens(
                    self.entries.as_slice(),
                    self.selected.as_slice(),
                ),
                actual_input_tokens,
                reserved_output_tokens: budget.reserved_output_tokens,
                maximum_context_tokens: budget.maximum_context_tokens,
                render_attempts: encoded.attempt,
            },
        })
    }
}

impl RenderedChatPrompt {
    fn encode<T: Tokenizer + ?Sized>(
        self,
        tokenizer: &T,
        effective_input_tokens: u32,
    ) -> Result<Option<EncodedChatPrompt>, ApplicationError> {
        match encode_text_with_policy(
            tokenizer,
            self.text.as_str(),
            effective_input_tokens,
            SpecialTokenPolicy::Allow,
        ) {
            Ok(tokens) => Ok(Some(EncodedChatPrompt {
                tokens,
                attempt: self.attempt,
            })),
            Err(TokenizationError::CapacityExhausted(_)) => Ok(None),
            Err(error) => Err(ApplicationFailure::from_debug(
                ApplicationFailureKind::Tokenizer,
                "chat prompt encoding failed",
                error,
            )
            .into()),
        }
    }
}

pub(super) fn build_context_units(
    raw_records: &[ConversationRecord],
    target_user: ConversationRecordId,
    regenerating: bool,
) -> Result<Vec<ContextPlanningUnit<'_>>, ApplicationError> {
    let mut active_assistants = BTreeMap::new();
    for record in raw_records {
        if record.role != ConversationRole::Assistant || !record.is_active_context() {
            continue;
        }
        let Some(attempt) = record.response_attempt.as_ref() else {
            continue;
        };
        if regenerating && attempt.responding_to == target_user {
            continue;
        }
        if active_assistants
            .insert(attempt.responding_to, record)
            .is_some()
        {
            return Err(ApplicationFailure::new(
                ApplicationFailureKind::Worker,
                "multiple active assistant responses reference one user turn",
            )
            .into());
        }
    }

    let mut units = Vec::with_capacity(raw_records.len().saturating_sub(active_assistants.len()));
    for record in raw_records {
        match record.role {
            ConversationRole::Assistant => {}
            ConversationRole::System => {
                units.push(ContextPlanningUnit::new(record, None, target_user));
            }
            ConversationRole::User => {
                let assistant = active_assistants.remove(&record.id);
                units.push(ContextPlanningUnit::new(record, assistant, target_user));
            }
        }
    }
    if let Some((responding_to, _)) = active_assistants.first_key_value() {
        return Err(ApplicationFailure::new(
            ApplicationFailureKind::Worker,
            format!(
                "active assistant response references missing user record {}",
                responding_to.get()
            ),
        )
        .into());
    }
    Ok(units)
}

fn unit_persistence(
    primary: &ConversationRecord,
    paired_assistant: Option<&ConversationRecord>,
    target_user: ConversationRecordId,
) -> ContextPersistence {
    if primary.id == target_user
        || primary.retention == ConversationRetention::Pinned
        || paired_assistant
            .is_some_and(|assistant| assistant.retention == ConversationRetention::Pinned)
    {
        ContextPersistence::Pinned
    } else if primary.retention == ConversationRetention::Retained
        || paired_assistant
            .is_some_and(|assistant| assistant.retention == ConversationRetention::Retained)
    {
        ContextPersistence::Retained
    } else {
        ContextPersistence::Ephemeral
    }
}

fn context_entry<'unit>(unit: &'unit ContextPlanningUnit<'_>) -> ContextEntry<'unit> {
    let role = match unit.primary.role {
        ConversationRole::System => ContextRole::System,
        ConversationRole::User => ContextRole::User,
        ConversationRole::Assistant => ContextRole::Assistant,
    };
    let source = match unit.primary.role {
        ConversationRole::System => ContextSource::Application,
        ConversationRole::User => ContextSource::User,
        ConversationRole::Assistant => ContextSource::Model,
    };
    ContextEntry {
        id: ContextEntryId::new(unit.primary.id.get()),
        ordinal: unit.primary.ordinal,
        role,
        source,
        priority: if unit.persistence == ContextPersistence::Pinned {
            PINNED_PRIORITY
        } else {
            HISTORICAL_PRIORITY
        },
        persistence: unit.persistence,
        estimated_tokens: unit.estimated_tokens,
        content: ContextContent::Text(unit.planning_content.as_ref()),
    }
}

fn expand_unit_record_ids(
    units: &[ContextPlanningUnit<'_>],
    indices: &[usize],
) -> Vec<ConversationRecordId> {
    let mut record_ids = Vec::new();
    for &index in indices {
        if let Some(unit) = units.get(index) {
            unit.append_record_ids(&mut record_ids);
        }
    }
    record_ids
}

fn selected_estimated_tokens(entries: &[ContextEntry<'_>], selected: &[usize]) -> u32 {
    selected.iter().fold(0_u32, |total, index| {
        total.saturating_add(
            entries
                .get(*index)
                .map_or(0, |entry| entry.estimated_tokens),
        )
    })
}

pub(super) fn estimate_content<T: Tokenizer + ?Sized>(
    tokenizer: &T,
    content: &str,
    maximum_context_tokens: u32,
) -> ConversationTokenEstimate {
    encode_text_with_policy(
        tokenizer,
        content,
        maximum_context_tokens,
        SpecialTokenPolicy::OrdinaryText,
    )
    .map_or_else(
        |_| ConversationTokenEstimate::Conservative(maximum_context_tokens.saturating_add(1)),
        |tokens| {
            ConversationTokenEstimate::Measured(u32::try_from(tokens.len()).unwrap_or(u32::MAX))
        },
    )
}

fn planning_failure(error: PlanningError) -> ApplicationError {
    match error {
        PlanningError::PinnedBudgetExceeded {
            required_tokens,
            available_tokens,
        } => ApplicationError::PinnedBudgetExceeded {
            required_at_least: required_tokens,
            available: available_tokens,
        },
        other => ApplicationFailure::from_debug(
            ApplicationFailureKind::Worker,
            "context planning failed",
            other,
        )
        .into(),
    }
}

fn render_failure(error: std::fmt::Error) -> ApplicationError {
    ApplicationFailure::new(ApplicationFailureKind::Worker, error).into()
}
