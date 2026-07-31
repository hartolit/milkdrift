use application_runtime::{
    ApplicationOutputBatch, ApplicationOutputRecordKind, ApplicationOutputState,
    ApplicationRuntime, ConversationRecord, ConversationRole, GenerationTerminalKind,
    GenerationTerminalOutcome, ResponseAttemptState,
};

use super::model::ComposerMode;
use crate::AppWindow;

const DEFAULT_TERMINAL_TEXT: &str = "No response has completed.";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum TranscriptPresentation {
    #[default]
    Conversation,
    DirectCompletion,
}

#[derive(Default)]
pub(super) struct PresentationState {
    displayed_request: Option<u64>,
    terminal_text: String,
    transcript: TranscriptPresentation,
}

impl PresentationState {
    pub(super) const fn displayed_request(&self) -> Option<u64> {
        self.displayed_request
    }

    pub(super) fn terminal_text(&self) -> &str {
        &self.terminal_text
    }

    fn begin_request(&mut self, request_id: u64) {
        self.displayed_request = Some(request_id);
        self.terminal_text.clear();
        self.terminal_text
            .push_str("Generation submitted; waiting for admission.");
    }

    pub(super) fn begin_chat_request(&mut self, request_id: u64) {
        self.transcript = TranscriptPresentation::Conversation;
        self.begin_request(request_id);
    }

    pub(super) fn begin_direct_request(
        &mut self,
        request_id: u64,
        prompt: &str,
    ) -> GeneratedOutputUpdate {
        self.transcript = TranscriptPresentation::DirectCompletion;
        self.begin_request(request_id);
        GeneratedOutputUpdate::Replace(format_direct_completion_transcript(prompt))
    }

    pub(super) fn clear(&mut self, mode: ComposerMode) -> GeneratedOutputUpdate {
        self.displayed_request = None;
        self.terminal_text.clear();
        self.terminal_text.push_str(DEFAULT_TERMINAL_TEXT);
        self.transcript = if mode == ComposerMode::DirectCompletion {
            TranscriptPresentation::DirectCompletion
        } else {
            TranscriptPresentation::Conversation
        };
        GeneratedOutputUpdate::Replace(String::new())
    }

    pub(super) const fn allows_conversation_snapshot(&self) -> bool {
        matches!(self.transcript, TranscriptPresentation::Conversation)
    }

    pub(super) fn apply_delta(&mut self, delta: FrameOutputDelta) -> PresentationUpdate {
        let terminal_changed = if let Some(terminal_text) = delta.terminal_text {
            self.terminal_text = terminal_text;
            true
        } else {
            false
        };
        PresentationUpdate {
            output: (!delta.text.is_empty()).then_some(GeneratedOutputUpdate::Append(delta.text)),
            terminal_changed,
            invalid_text_record: delta.invalid_text_record,
        }
    }
}

fn format_direct_completion_transcript(prompt: &str) -> String {
    format!("Prompt:\n{prompt}\n\nCompletion:\n")
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum GeneratedOutputUpdate {
    Append(String),
    Replace(String),
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct PresentationUpdate {
    pub(super) output: Option<GeneratedOutputUpdate>,
    pub(super) terminal_changed: bool,
    invalid_text_record: bool,
}

pub(super) const fn replace_conversation_update(transcript: String) -> GeneratedOutputUpdate {
    GeneratedOutputUpdate::Replace(transcript)
}

pub(super) fn render_generated_output_update(window: &AppWindow, update: GeneratedOutputUpdate) {
    match update {
        GeneratedOutputUpdate::Append(text) => window.invoke_append_generated_text(text.into()),
        GeneratedOutputUpdate::Replace(text) => {
            window.invoke_replace_transcript(text.into());
        }
    }
}

pub(super) fn render_presentation_update(
    window: &AppWindow,
    presentation: &PresentationState,
    update: PresentationUpdate,
) {
    if let Some(output) = update.output {
        render_generated_output_update(window, output);
    }
    if update.terminal_changed {
        window.set_terminal_text(presentation.terminal_text.clone().into());
    }
    if update.invalid_text_record {
        window.set_status_text(
            "Generated output contained an invalid UTF-8 range; the affected fragment was skipped."
                .into(),
        );
    }
}

#[derive(Default)]
pub(super) struct FrameOutputDelta {
    pub(super) text: String,
    pub(super) terminal_text: Option<String>,
    pub(super) invalid_text_record: bool,
}

pub(super) fn collect_output_batch(
    batch: &ApplicationOutputBatch<'_>,
    displayed_request: Option<u64>,
) -> FrameOutputDelta {
    let mut delta = FrameOutputDelta::default();
    for record in batch.records() {
        if displayed_request != Some(record.request_id.get()) {
            continue;
        }
        match record.kind {
            ApplicationOutputRecordKind::Text(_) => match batch.text_for(record) {
                Some(text) => delta.text.push_str(text),
                None => delta.invalid_text_record = true,
            },
            ApplicationOutputRecordKind::State(state) => {
                if let Some(message) = output_state_message(state) {
                    delta.terminal_text = Some(message);
                }
            }
        }
    }
    delta
}

pub(super) fn output_state_message(state: ApplicationOutputState) -> Option<String> {
    match state {
        ApplicationOutputState::Yielded(_) => None,
        ApplicationOutputState::Terminal(kind) => Some(format!(
            "{} Backend cleanup is in progress.",
            terminal_kind_message(kind)
        )),
        ApplicationOutputState::CleanupPending => {
            Some("Generation ended; backend cleanup is pending and will be retried.".to_owned())
        }
        ApplicationOutputState::CleanupExhausted => Some(
            "Generation ended, but backend cleanup retries were exhausted; resources remain retained."
                .to_owned(),
        ),
        ApplicationOutputState::Released(kind) => {
            Some(released_terminal_message(&terminal_presentation(kind)))
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum TerminalPresentation {
    Finished(String),
    Failed,
}

fn terminal_presentation(kind: GenerationTerminalKind) -> TerminalPresentation {
    match kind {
        GenerationTerminalKind::Finished(reason) => {
            TerminalPresentation::Finished(format!("{reason:?}"))
        }
        GenerationTerminalKind::Failed => TerminalPresentation::Failed,
    }
}

fn terminal_kind_message(kind: GenerationTerminalKind) -> String {
    terminal_presentation_message(&terminal_presentation(kind))
}

fn terminal_presentation_message(presentation: &TerminalPresentation) -> String {
    match presentation {
        TerminalPresentation::Finished(reason) => format!("Generation finished: {reason}."),
        TerminalPresentation::Failed => "Generation failed.".to_owned(),
    }
}

pub(super) fn released_terminal_message(presentation: &TerminalPresentation) -> String {
    format!(
        "{} Backend resources were released.",
        terminal_presentation_message(presentation)
    )
}

pub(super) fn format_terminal_outcome(outcome: &GenerationTerminalOutcome) -> String {
    match outcome {
        GenerationTerminalOutcome::Finished(reason) => {
            format!("Generation finished: {reason:?}. Backend resources were released.")
        }
        GenerationTerminalOutcome::Failed(failure) => {
            format!("Generation failed: {failure}. Backend resources were released.")
        }
    }
}

pub(super) fn format_conversation(records: &[ConversationRecord]) -> String {
    let mut transcript = String::new();
    for record in records {
        let label = match record.role {
            ConversationRole::System => "System",
            ConversationRole::User => "User",
            ConversationRole::Assistant => "Assistant",
        };
        transcript.push_str(label);
        transcript.push_str(": ");
        transcript.push_str(record.content.as_str());
        if let Some(attempt) = record.response_attempt.as_ref() {
            match &attempt.state {
                ResponseAttemptState::Streaming => {}
                ResponseAttemptState::Completed(reason) => {
                    transcript.push_str(format!("\n[completed: {reason:?}]").as_str());
                }
                ResponseAttemptState::Cancelled(reason) => {
                    transcript.push_str(format!("\n[cancelled: {reason:?}]").as_str());
                }
                ResponseAttemptState::Failed(failure) => {
                    transcript.push_str(format!("\n[failed: {failure}]").as_str());
                }
            }
            if attempt.superseded {
                transcript.push_str("\n[superseded by regeneration]");
            }
        }
        if !matches!(
            record
                .response_attempt
                .as_ref()
                .map(|attempt| &attempt.state),
            Some(ResponseAttemptState::Streaming)
        ) {
            transcript.push_str("\n\n");
        }
    }
    transcript
}

pub(super) fn synchronize_conversation(window: &AppWindow, runtime: &ApplicationRuntime) {
    let transcript = format_conversation(runtime.conversation());
    render_generated_output_update(window, replace_conversation_update(transcript));
}

pub(super) fn synchronize_usage(
    window: &AppWindow,
    runtime: &ApplicationRuntime,
    displayed_request: Option<u64>,
) {
    let state = runtime.state();
    let usage = state
        .active_generation()
        .filter(|summary| displayed_request == Some(summary.request_id.get()))
        .map(|summary| summary.usage)
        .or_else(|| {
            state
                .last_generation()
                .filter(|terminal| displayed_request == Some(terminal.request_id.get()))
                .map(|terminal| terminal.usage)
        });
    let (prompt_tokens, generated_tokens) = usage.map_or_else(
        || ("0".to_owned(), "0".to_owned()),
        |usage| {
            (
                usage.prompt_tokens.to_string(),
                usage.generated_tokens.to_string(),
            )
        },
    );
    window.set_prompt_token_count(prompt_tokens.into());
    window.set_generated_token_count(generated_tokens.into());
}
