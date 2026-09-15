//! Frozen conversation data prepared by runtime context selection. Adapters consume this
//! document through a manifest-selected artifact; they never discover predecessors themselves.

use std::collections::{BTreeMap, BTreeSet};

use milkdrift_capability::ArtifactReference;
use milkdrift_persistence::RunSequence;
use serde::{Deserialize, Serialize};

use crate::{Message, MessageRole, ModelContractError, ModelTaskRequest, SessionSelection};

/// Media family for runtime-prepared conversation history.
pub const CONTINUATION_MEDIA_TYPE: &str = "application/vnd.milkdrift.continuation.v1+json";
/// Maximum number of prior invocations in one continuation, including inherited history.
pub const MAX_CONTINUATION_DEPTH: usize = 32;

/// Exact predecessor artifacts and journal anchors. Runtime verifies every anchor against
/// the same run before reading conversation data, including when a retained selection is reused.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContinuationTurn {
    /// Manifest consumed by the producing invocation.
    pub manifest: ArtifactReference,
    /// Complete canonical response from that invocation.
    pub response: ArtifactReference,
    /// Node eligibility, scheduling, authorization, output publication and terminal anchors.
    pub events: [RunSequence; 5],
    /// Exclusive end of this turn's messages in the flattened history. The previous end
    /// (or zero) is its start, so inspection can attribute every message to exact sources.
    pub message_end: u32,
}

/// Bounded, ordered conversation history with the exact sources needed to recheck its use.
/// System/developer instructions are deliberately absent. The current task supplies them.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContinuationHistory {
    turns: Vec<ContinuationTurn>,
    messages: Vec<Message>,
    authority: String,
}

impl ContinuationHistory {
    /// Validates the finite source chain and message bounds. Runtime additionally proves
    /// artifact integrity, causality, current authority and the messages' exact derivation.
    pub fn new(
        turns: Vec<ContinuationTurn>,
        messages: Vec<Message>,
        authority: String,
    ) -> Result<Self, ModelContractError> {
        let mut seen = BTreeSet::new();
        if turns.is_empty()
            || turns.len() > MAX_CONTINUATION_DEPTH
            || turns.iter().any(|turn| {
                !seen.insert(turn.manifest.identity())
                    || turn.events.contains(&RunSequence::ZERO)
                    || turn.events.windows(2).any(|events| events[0] >= events[1])
            })
            || turns
                .last()
                .is_none_or(|turn| turn.message_end as usize != messages.len())
            || turns.first().is_none_or(|turn| turn.message_end == 0)
            || turns
                .windows(2)
                .any(|turns| turns[0].message_end >= turns[1].message_end)
            || authority.is_empty()
            || authority.len() > 192
            || messages.iter().any(|message| {
                matches!(message.role(), MessageRole::System | MessageRole::Developer)
            })
        {
            return Err(invalid("invalid continuation sources or instruction roles"));
        }
        // Reuse the request owner's message and aggregate text bounds.
        ModelTaskRequest::new(
            messages.clone(),
            Vec::new(),
            None,
            SessionSelection::Fresh,
            None,
            1,
            false,
            BTreeMap::new(),
        )?;
        validate_tool_history(&messages, false)?;
        Ok(Self {
            turns,
            messages,
            authority,
        })
    }

    /// Oldest-to-newest exact predecessor sources.
    #[must_use]
    pub fn turns(&self) -> &[ContinuationTurn] {
        &self.turns
    }
    /// Role-labelled history, excluding current task messages and prior instructions.
    #[must_use]
    pub fn messages(&self) -> &[Message] {
        &self.messages
    }
    /// Frozen initiating authority decision used during assembly.
    #[must_use]
    pub fn authority(&self) -> &str {
        &self.authority
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HistoryWire {
    turns: Vec<ContinuationTurn>,
    messages: Vec<Message>,
    authority: String,
}
milkdrift_contracts::deserialize_via!(ContinuationHistory, HistoryWire, |wire| Self::new(
    wire.turns,
    wire.messages,
    wire.authority
));

impl ModelTaskRequest {
    /// Combines runtime-prepared history with this request. Current instructions precede
    /// the history; remaining current messages follow it. Every proposed tool call must
    /// have exactly one following result before another user/assistant turn or HTTP entry.
    pub fn with_continuation(
        &self,
        history: &ContinuationHistory,
    ) -> Result<Self, ModelContractError> {
        let Some(last) = history.turns().last() else {
            return Err(invalid("missing continuation predecessor"));
        };
        if self.session()
            != &(SessionSelection::ExplicitContinuation {
                manifest: last.manifest.clone(),
                response: last.response.clone(),
            })
        {
            return Err(invalid("continuation does not match the exact request"));
        }
        let instructions = |message: &&Message| {
            matches!(message.role(), MessageRole::System | MessageRole::Developer)
        };
        let mut messages: Vec<_> = self
            .messages()
            .iter()
            .filter(instructions)
            .cloned()
            .collect();
        messages.extend_from_slice(history.messages());
        messages.extend(
            self.messages()
                .iter()
                .filter(|message| !instructions(message))
                .cloned(),
        );
        validate_tool_history(&messages, true)?;
        Self::new(
            messages,
            self.tools().to_vec(),
            self.structured_output().cloned(),
            self.session().clone(),
            self.reasoning(),
            self.maximum_output_units(),
            self.streaming(),
            self.extensions().clone(),
        )
    }
}

fn validate_tool_history(messages: &[Message], complete: bool) -> Result<(), ModelContractError> {
    let mut pending = BTreeSet::new();
    let mut seen = BTreeSet::new();
    for message in messages {
        if message.role() == MessageRole::ToolResult {
            if !message.tool_call_id().is_some_and(|id| pending.remove(id)) {
                return Err(invalid("tool result has no unique preceding call"));
            }
        } else {
            if !pending.is_empty() {
                return Err(invalid("tool exchange is incomplete"));
            }
            for call in message.tool_calls() {
                if !seen.insert(call.id()) {
                    return Err(invalid("tool call identity is repeated"));
                }
                pending.insert(call.id());
            }
        }
    }
    if complete && !pending.is_empty() {
        return Err(invalid("tool exchange is incomplete"));
    }
    Ok(())
}

fn invalid(reason: &str) -> ModelContractError {
    ModelContractError::Invalid(reason.to_owned())
}
